//! Non-blocking raw terminal input guard, ANSI sequence decoder, resize handler, and frame output.
//!
//! Configures standard input on interactive POSIX terminals so single keypresses arrive without
//! waiting for newline characters or echoing over the rendered grid, while leaving output
//! post-processing enabled so frame redraws preserve line alignment. Enters the alternate screen
//! buffer on interactive terminals, tracks window-resize signals asynchronously, wraps TTY updates
//! in synchronized output markers, and restores terminal state on drop or panic.

use std::io::{self, IsTerminal, Write};
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once, OnceLock};

use terminal_graphics::Display;

use crate::model::Action;

/// Saved original terminal attributes used by the panic hook to restore shell state on crash.
static SAVED_TERMIOS: OnceLock<Mutex<Option<libc::termios>>> = OnceLock::new();

/// Tracks whether the alternate screen buffer is active so cleanup leaves normal scrollback intact.
static ALT_SCREEN_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Flag set by the window-change signal handler when the terminal size changes.
static RESIZE_PENDING: AtomicBool = AtomicBool::new(false);

/// Ensures the terminal-restoring panic hook is registered at most once per process.
static PANIC_HOOK_ONCE: Once = Once::new();

/// Ensures the window-resize signal handler is registered at most once per process.
static SIGWINCH_ONCE: Once = Once::new();

/// Decoded keyboard or control event read from standard input.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RawInput {
    /// Player movement, wait, or interact intent forwarded to the world controller.
    Action(Action),
    /// Toggle pause state for timed or hybrid pacing modes.
    TogglePause,
    /// Pause the simulation clock and advance exactly one simulation step.
    StepOnce,
    /// Reset simulation time to zero while keeping the active random seed.
    ResetSameSeed,
    /// Reset simulation time to zero with a freshly generated random seed.
    ResetNewSeed,
    /// Toggle visibility of the live entity-component inspector view.
    ToggleInspector,
    /// Move selection backward to the previous entity in the inspector list.
    InspectorPrev,
    /// Move selection forward to the next entity in the inspector list.
    InspectorNext,
    /// Dismiss any non-fatal overlay such as an inline error box or inspector modal.
    DismissOverlay,
    /// Exit the application cleanly.
    Quit,
}

/// RAII guard that enables raw keyboard input on interactive terminals and restores state on drop.
///
/// Leaving non-interactive standard input streams untouched allows piped smoke tests and unit test
/// runners to execute without terminal ioctl failures. Enters the alternate screen buffer and hides
/// the cursor on interactive terminals, restoring saved attributes deterministically on drop.
pub struct TerminalGuard {
    /// Original terminal configuration captured before enabling raw mode, if attached to a TTY.
    orig: Option<libc::termios>,
}

impl TerminalGuard {
    /// Switches an interactive terminal into raw input mode inside the alternate screen buffer.
    ///
    /// When standard input or standard output is not a terminal, succeeds without modifying file
    /// descriptor attributes or writing terminal mode escapes. Registers process panic and resize
    /// handlers the first time raw mode is entered.
    ///
    /// # Errors
    ///
    /// Returns any operating-system error reported while reading or updating terminal attributes or
    /// writing the alternate-screen and cursor-hide escape sequences.
    pub fn enter() -> io::Result<Self> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Ok(Self { orig: None });
        }

        let mut orig = MaybeUninit::<libc::termios>::uninit();
        // SAFETY: `STDIN_FILENO` is the open standard input descriptor and `orig` points to aligned
        // writable storage sized for `libc::termios`.
        let rc = unsafe { libc::tcgetattr(libc::STDIN_FILENO, orig.as_mut_ptr()) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `tcgetattr` succeeded, so `orig` is fully initialized by the OS kernel.
        let orig = unsafe { orig.assume_init() };

        install_panic_restore_hook();
        install_sigwinch_handler();
        store_saved_termios(Some(orig));

        let mut raw = orig;
        // Disable flow control, CR translation, and break/parity translation on input.
        raw.c_iflag &= !(libc::IXON | libc::ICRNL | libc::BRKINT | libc::INPCK | libc::ISTRIP);
        // Disable line buffering, echo, extended input, and OS signals (`Ctrl-C` arrives as 0x03).
        // Keep `c_oflag` (`OPOST | ONLCR`) intact so Display frame newlines do not staircase.
        raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN | libc::ISIG);
        raw.c_cflag |= libc::CS8;
        raw.c_cc[libc::VMIN] = 0;
        raw.c_cc[libc::VTIME] = 0;

        // SAFETY: `raw` is a valid initialized `libc::termios` copy applied to `STDIN_FILENO`.
        let set_rc = unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &raw) };
        if set_rc != 0 {
            store_saved_termios(None);
            return Err(io::Error::last_os_error());
        }

        let enter_res = {
            let mut stdout = io::stdout().lock();
            stdout.write_all(b"\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H")?;
            stdout.flush()
        };
        if let Err(err) = enter_res {
            // SAFETY: `orig` holds valid kernel-provided `termios` attributes for `STDIN_FILENO`.
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &orig);
            }
            store_saved_termios(None);
            return Err(err);
        }

        ALT_SCREEN_ACTIVE.store(true, Ordering::Release);
        Ok(Self { orig: Some(orig) })
    }

    /// Reports whether standard input and output are connected to an interactive terminal.
    pub fn is_interactive_tty(&self) -> bool {
        self.orig.is_some()
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if let Some(orig) = self.orig.take() {
            // SAFETY: `orig` holds valid kernel-provided `termios` attributes for `STDIN_FILENO`.
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &orig);
            }
            store_saved_termios(None);
            let leave_alt = ALT_SCREEN_ACTIVE.swap(false, Ordering::AcqRel);
            let mut stdout = io::stdout().lock();
            if leave_alt {
                let _ = stdout.write_all(b"\x1b[?25h\x1b[0m\x1b[?1049l");
            } else {
                let _ = stdout.write_all(b"\x1b[?25h\x1b[0m");
            }
            let _ = stdout.flush();
        }
    }
}

/// Saves or clears the fallback terminal configuration consulted by the panic hook.
fn store_saved_termios(val: Option<libc::termios>) {
    let lock = SAVED_TERMIOS.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = lock.lock() {
        *guard = val;
    }
}

/// Registers a panic hook once that restores terminal attributes before printing panic details.
fn install_panic_restore_hook() {
    PANIC_HOOK_ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if let Some(lock) = SAVED_TERMIOS.get()
                && let Ok(mut guard) = lock.lock()
                && let Some(orig) = guard.take()
            {
                // SAFETY: `orig` was captured from `tcgetattr(STDIN_FILENO)` on startup.
                unsafe {
                    libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &orig);
                }
                let seq: &[u8] = if ALT_SCREEN_ACTIVE.swap(false, Ordering::AcqRel) {
                    b"\x1b[?25h\x1b[0m\x1b[?1049l\r\n"
                } else {
                    b"\x1b[?25h\x1b[0m\r\n"
                };
                // SAFETY: `seq` points to valid static bytes written synchronously to stdout.
                unsafe {
                    libc::write(
                        libc::STDOUT_FILENO,
                        seq.as_ptr().cast::<libc::c_void>(),
                        seq.len(),
                    );
                }
            }
            prev(info);
        }));
    });
}

/// Async-signal-safe handler marking a pending window resize when the terminal changes size.
extern "C" fn handle_sigwinch(_sig: libc::c_int) {
    RESIZE_PENDING.store(true, Ordering::Relaxed);
}

/// Installs the window-resize signal handler without restart flags so poll wakes immediately.
fn install_sigwinch_handler() {
    SIGWINCH_ONCE.call_once(|| {
        // SAFETY: `libc::sigaction` is a C POD struct whose zeroed representation is valid prior
        // to initializing `sa_sigaction`, `sa_mask`, and `sa_flags`.
        let mut sa: libc::sigaction = unsafe { std::mem::zeroed() };
        sa.sa_sigaction = handle_sigwinch as *const () as libc::sighandler_t;
        sa.sa_flags = 0;
        // SAFETY: `sa.sa_mask` points to aligned writable storage sized for `sigset_t`.
        unsafe {
            libc::sigemptyset(&mut sa.sa_mask);
            // SAFETY: `handle_sigwinch` only performs an atomic store and `SIGWINCH` registration
            // uses a properly initialized `sigaction` struct.
            libc::sigaction(libc::SIGWINCH, &sa, std::ptr::null_mut());
        }
    });
}

/// Returns and clears whether a terminal resize signal arrived since the last check.
pub fn take_resize_pending() -> bool {
    RESIZE_PENDING.swap(false, Ordering::AcqRel)
}

/// Flushes a composed frame to standard output using synchronized framing on interactive TTYs.
///
/// Interactive mode positions the cursor below the display height so cursor-up rendering overwrites
/// from the top row cleanly inside the alternate screen buffer, wrapping updates in synchronized
/// output markers to eliminate flicker. Non-terminal invocations fall back to direct printing.
///
/// - `display`: composed character grid ready for output.
/// - `is_tty`: whether standard output is running inside the interactive alternate screen.
/// - `clear_screen`: true after layout changes so stale cells outside the new frame are erased.
///
/// # Errors
///
/// Returns any standard output write or flush error reported by the operating system.
pub fn present_frame(display: &Display, is_tty: bool, clear_screen: bool) -> io::Result<()> {
    if !is_tty {
        return display.print();
    }

    let stdout = io::stdout();
    let mut lock = stdout.lock();
    lock.write_all(b"\x1b[?2026h")?;
    if clear_screen {
        lock.write_all(b"\x1b[2J")?;
    }
    let anchor_row = display.height().saturating_add(1);
    write!(lock, "\x1b[{anchor_row};1H")?;
    display.write_to(&mut lock)?;
    lock.write_all(b"\x1b[?2026l")?;
    lock.flush()
}

/// Queries the current terminal dimensions in columns and rows if attached to a terminal.
///
/// Returns `None` when standard output is not a terminal or reports zero dimensions.
pub fn terminal_size() -> Option<(u16, u16)> {
    let mut ws = MaybeUninit::<libc::winsize>::zeroed();
    // SAFETY: `TIOCGWINSZ` writes a `libc::winsize` struct into `ws` when `STDOUT_FILENO` is a TTY.
    let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, ws.as_mut_ptr()) };
    if rc != 0 {
        return None;
    }
    // SAFETY: `ioctl` returned zero, so `ws` contains initialized window dimensions.
    let ws = unsafe { ws.assume_init() };
    if ws.ws_col == 0 || ws.ws_row == 0 {
        None
    } else {
        Some((ws.ws_col, ws.ws_row))
    }
}

/// Waits up to the timeout duration for input bytes and drains all immediately available events.
///
/// Returns an empty list on timeout or signal interruption. When standard input reaches
/// end-of-file on a closed pipe, yields a single quit event so headless invocations exit cleanly.
///
/// - `timeout_ms`: maximum wait time in milliseconds, or negative to block until input arrives.
///
/// # Errors
///
/// Returns any operating-system error reported while polling or reading standard input.
pub fn poll_and_drain_inputs(timeout_ms: i32) -> io::Result<Vec<RawInput>> {
    let mut pfd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };

    // SAFETY: `pfd` points to one valid `pollfd` entry covering `STDIN_FILENO`.
    let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
    if rc < 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            return Ok(Vec::new());
        }
        return Err(err);
    }
    if rc == 0 {
        return Ok(Vec::new());
    }

    if (pfd.revents & libc::POLLIN) == 0 {
        if (pfd.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL)) != 0 {
            return Ok(vec![RawInput::Quit]);
        }
        return Ok(Vec::new());
    }

    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64];
    loop {
        // SAFETY: `chunk` is valid writable stack storage on `STDIN_FILENO`.
        let n = unsafe {
            libc::read(
                libc::STDIN_FILENO,
                chunk.as_mut_ptr().cast::<libc::c_void>(),
                chunk.len(),
            )
        };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted
                || err.kind() == io::ErrorKind::WouldBlock
            {
                break;
            }
            return Err(err);
        }
        if n == 0 {
            if bytes.is_empty() {
                return Ok(vec![RawInput::Quit]);
            }
            break;
        }
        bytes.extend_from_slice(&chunk[..n as usize]);

        if !has_pending_stdin_bytes(0)? {
            break;
        }
    }

    // Wait briefly if a read ended mid-escape so split arrow sequences stay intact.
    if bytes.last() == Some(&0x1b) && has_pending_stdin_bytes(5)? {
        // SAFETY: `chunk` is valid writable stack storage and `poll` confirmed readable bytes.
        let n = unsafe {
            libc::read(
                libc::STDIN_FILENO,
                chunk.as_mut_ptr().cast::<libc::c_void>(),
                chunk.len(),
            )
        };
        if n > 0 {
            bytes.extend_from_slice(&chunk[..n as usize]);
        }
    }

    Ok(parse_input_bytes(&bytes))
}

/// Checks whether standard input has readable bytes within the given timeout.
fn has_pending_stdin_bytes(timeout_ms: i32) -> io::Result<bool> {
    let mut pfd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: `pfd` points to one valid `pollfd` entry covering `STDIN_FILENO`.
    let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
    if rc < 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(err);
    }
    Ok(rc > 0 && (pfd.revents & libc::POLLIN) != 0)
}

/// Decodes a raw byte slice containing ASCII keys and ANSI arrow escape sequences into events.
///
/// Unknown escape sequences are consumed cleanly up to their terminating byte without emitting
/// stray character presses.
///
/// - `bytes`: raw terminal input bytes captured from standard input.
pub fn parse_input_bytes(bytes: &[u8]) -> Vec<RawInput> {
    let mut events = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x1b {
            if let Some((maybe_event, next_idx)) = parse_escape_sequence(bytes, i) {
                if let Some(ev) = maybe_event {
                    events.push(ev);
                }
                i = next_idx;
                continue;
            }
            events.push(RawInput::DismissOverlay);
            i += 1;
            continue;
        }

        if let Some(ev) = map_single_byte(b) {
            events.push(ev);
        }
        i += 1;
    }

    events
}

/// Attempts to parse a multi-byte escape sequence starting at the given offset.
fn parse_escape_sequence(bytes: &[u8], start: usize) -> Option<(Option<RawInput>, usize)> {
    let prefix = *bytes.get(start + 1)?;
    if prefix == b'O' {
        let code = *bytes.get(start + 2)?;
        let dir = match code {
            b'A' => Some(RawInput::Action(Action::Move { dx: 0, dy: -1 })),
            b'B' => Some(RawInput::Action(Action::Move { dx: 0, dy: 1 })),
            b'C' => Some(RawInput::Action(Action::Move { dx: 1, dy: 0 })),
            b'D' => Some(RawInput::Action(Action::Move { dx: -1, dy: 0 })),
            _ => None,
        };
        return Some((dir, start + 3));
    }

    if prefix == b'[' {
        let mut j = start + 2;
        while j < bytes.len() && (0x20..=0x3f).contains(&bytes[j]) {
            j += 1;
        }
        let final_byte = *bytes.get(j)?;
        if !(0x40..=0x7e).contains(&final_byte) {
            return None;
        }
        let is_plain_csi = j == start + 2;
        let dir = if is_plain_csi {
            match final_byte {
                b'A' => Some(RawInput::Action(Action::Move { dx: 0, dy: -1 })),
                b'B' => Some(RawInput::Action(Action::Move { dx: 0, dy: 1 })),
                b'C' => Some(RawInput::Action(Action::Move { dx: 1, dy: 0 })),
                b'D' => Some(RawInput::Action(Action::Move { dx: -1, dy: 0 })),
                _ => None,
            }
        } else {
            None
        };
        return Some((dir, j + 1));
    }

    None
}

/// Maps one non-escape input byte to its corresponding key or engine command.
fn map_single_byte(b: u8) -> Option<RawInput> {
    match b {
        0x03 | b'q' | b'Q' => Some(RawInput::Quit),
        b'p' | b'P' => Some(RawInput::TogglePause),
        b'n' | b'N' => Some(RawInput::StepOnce),
        b'r' => Some(RawInput::ResetSameSeed),
        b'R' => Some(RawInput::ResetNewSeed),
        b'\t' | b'i' | b'I' => Some(RawInput::ToggleInspector),
        b'[' => Some(RawInput::InspectorPrev),
        b']' => Some(RawInput::InspectorNext),
        b' ' | b'.' => Some(RawInput::Action(Action::Wait)),
        b'e' | b'E' | b'\r' | b'\n' => Some(RawInput::Action(Action::Act)),
        b'w' | b'W' | b'k' | b'K' => Some(RawInput::Action(Action::Move { dx: 0, dy: -1 })),
        b's' | b'S' | b'j' | b'J' => Some(RawInput::Action(Action::Move { dx: 0, dy: 1 })),
        b'a' | b'A' | b'h' | b'H' => Some(RawInput::Action(Action::Move { dx: -1, dy: 0 })),
        b'd' | b'D' | b'l' | b'L' => Some(RawInput::Action(Action::Move { dx: 1, dy: 0 })),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests verifying ASCII keybindings, ANSI arrow decoding, and inspector shortcuts.

    use super::*;

    #[test]
    fn parses_movement_wait_act_inspector_and_engine_keys() {
        let bytes = b"wAsD hJkL .\r\nepNrR\tiI[]q\x03";
        assert_eq!(
            parse_input_bytes(bytes),
            vec![
                RawInput::Action(Action::Move { dx: 0, dy: -1 }),
                RawInput::Action(Action::Move { dx: -1, dy: 0 }),
                RawInput::Action(Action::Move { dx: 0, dy: 1 }),
                RawInput::Action(Action::Move { dx: 1, dy: 0 }),
                RawInput::Action(Action::Wait),
                RawInput::Action(Action::Move { dx: -1, dy: 0 }),
                RawInput::Action(Action::Move { dx: 0, dy: 1 }),
                RawInput::Action(Action::Move { dx: 0, dy: -1 }),
                RawInput::Action(Action::Move { dx: 1, dy: 0 }),
                RawInput::Action(Action::Wait),
                RawInput::Action(Action::Wait),
                RawInput::Action(Action::Act),
                RawInput::Action(Action::Act),
                RawInput::Action(Action::Act),
                RawInput::TogglePause,
                RawInput::StepOnce,
                RawInput::ResetSameSeed,
                RawInput::ResetNewSeed,
                RawInput::ToggleInspector,
                RawInput::ToggleInspector,
                RawInput::ToggleInspector,
                RawInput::InspectorPrev,
                RawInput::InspectorNext,
                RawInput::Quit,
                RawInput::Quit,
            ]
        );
    }

    #[test]
    fn parses_ansi_arrows_bare_escape_and_ignores_unknown_csi() {
        assert_eq!(
            parse_input_bytes(b"\x1b[A\x1b[B\x1b[C\x1b[D\x1bOA\x1bOB\x1bOC\x1bOD"),
            vec![
                RawInput::Action(Action::Move { dx: 0, dy: -1 }),
                RawInput::Action(Action::Move { dx: 0, dy: 1 }),
                RawInput::Action(Action::Move { dx: 1, dy: 0 }),
                RawInput::Action(Action::Move { dx: -1, dy: 0 }),
                RawInput::Action(Action::Move { dx: 0, dy: -1 }),
                RawInput::Action(Action::Move { dx: 0, dy: 1 }),
                RawInput::Action(Action::Move { dx: 1, dy: 0 }),
                RawInput::Action(Action::Move { dx: -1, dy: 0 }),
            ]
        );

        assert_eq!(parse_input_bytes(b"\x1b"), vec![RawInput::DismissOverlay]);
        assert_eq!(parse_input_bytes(b"\x1b[15~\x1b[<0;10;5M"), vec![]);
    }
}
