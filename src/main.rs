//! Interactive terminal loop for evaluating user-entered Lua code.
//!
//! Reads commands and script lines from standard input, runs Lua chunks in an embedded virtual
//! machine, and paints colored frames to standard output through [Display]. Global Lua state
//! persists across turns, and calls to Lua's print function route into the rendered output frames
//! rather than bypassing the character grid.
//!
//! Input handling reads one line per prompt turn on the main thread, distinguishing built-in
//! control commands from Lua source before compilation.

use std::cell::RefCell;
use std::io::{self, IsTerminal, Write};
use std::rc::Rc;

use mlua::prelude::*;
use terminal_graphics::shapes::Rect;
use terminal_graphics::{Colour, Container, Display, Graphic, Text};

/// Horizontal cell width shared by framed panels and wrapped output frames.
const PANEL_WIDTH: u32 = 72;

/// Literal prompt string painted before each user input read.
const PROMPT_TEXT: &str = "turtle> ";

/// Four-space replacement expanded for each horizontal tab before drawing into a cell grid.
const TAB_SPACES: &str = "    ";

/// Shared per-turn sink collecting formatted lines emitted by Lua's print function.
type OutputBuffer = Rc<RefCell<Vec<String>>>;

/// Parsed action produced from one trimmed line of user input.
///
/// Lifetime `'a` borrows directly from the raw line buffer when input is routed to Lua evaluation,
/// avoiding extra allocation on the common script path.
enum ReplCommand<'a> {
    /// Empty or whitespace-only line that re-prompts without evaluating Lua.
    Blank,
    /// Built-in command requesting the summary panel.
    Help,
    /// Built-in command requesting loop termination.
    Quit,
    /// Raw user text forwarded to the Lua evaluator.
    Eval(&'a str),
}

/// Multi-line block of individually colored strings positioned relative to an origin cell.
///
/// Implements [Graphic] so [Container] can lay out several consecutive rows inside one bordered
/// panel. Owns its line buffer and stores its top-left origin in display cells, with horizontal
/// coordinates growing rightward and vertical coordinates growing downward. Constructed directly by
/// panel helpers and rendered sequentially on the main thread.
struct TextBlock {
    /// Horizontal origin column in display cells.
    pos_x: isize,
    /// Vertical origin row in display cells.
    pos_y: isize,
    /// Ordered text rows paired with their foreground color.
    lines: Vec<(String, Colour)>,
}

impl Graphic for TextBlock {
    fn draw(&self, display: &mut Display) {
        for (row, (line, fg)) in self.lines.iter().enumerate() {
            Text::new(
                self.pos_x,
                self.pos_y.saturating_add(row as isize),
                line,
                *fg,
                Colour::Black,
            )
            .draw(display);
        }
    }

    fn position(&self) -> (isize, isize) {
        (self.pos_x, self.pos_y)
    }

    fn set_position(&mut self, pos_x: isize, pos_y: isize) {
        self.pos_x = pos_x;
        self.pos_y = pos_y;
    }
}

/// Classifies one line of standard input into a control command or Lua source.
///
/// Leading and trailing whitespace are ignored when matching built-in command names. Any other
/// non-blank text keeps its original borrowed slice as [ReplCommand::Eval].
///
/// - `input`: raw single line entered by the user without a trailing newline.
fn parse_command(input: &str) -> ReplCommand<'_> {
    match input.trim() {
        "" => ReplCommand::Blank,
        "help" => ReplCommand::Help,
        "quit" => ReplCommand::Quit,
        _ => ReplCommand::Eval(input),
    }
}

/// Splits text into wrapped lines free of carriage returns and horizontal tabs.
///
/// Strips carriage returns, replaces each tab with [TAB_SPACES], breaks on newlines, and chunks
/// long lines by Unicode scalar count so every output row fits within `max_width` terminal cells.
/// Empty input or empty lines between newlines each yield one empty string so blank lines remain
/// visible.
///
/// - `text`: arbitrary source string to prepare for grid drawing.
/// - `max_width`: maximum character columns per output row; values below one clamp to one column.
fn wrap_lines(text: &str, max_width: usize) -> Vec<String> {
    let width = max_width.max(1);
    let normalized = text.replace('\r', "").replace('\t', TAB_SPACES);
    let mut wrapped = Vec::new();

    for raw_line in normalized.split('\n') {
        if raw_line.is_empty() {
            wrapped.push(String::new());
            continue;
        }

        let mut current = String::new();
        let mut count = 0;
        for ch in raw_line.chars() {
            if count == width {
                wrapped.push(current);
                current = String::new();
                count = 0;
            }
            current.push(ch);
            count += 1;
        }
        if !current.is_empty() {
            wrapped.push(current);
        }
    }

    wrapped
}

/// Reserves vertical terminal rows and writes a buffered frame to standard output.
///
/// Calls [Display::reserve_space] immediately before [Display::print] so the cursor-up escape
/// sequence inside the rendered frame overwrites only freshly allocated blank lines instead of
/// earlier terminal scrollback.
///
/// - `display`: off-screen character grid holding the complete frame to emit.
///
/// # Errors
///
/// Returns any standard-output error reported while reserving space or printing the frame.
fn flush_display(display: &Display) -> io::Result<()> {
    display.reserve_space()?;
    display.print()
}

/// Builds an off-screen character grid holding a bordered panel of wrapped colored lines.
///
/// Wraps each entry to leave one cell of horizontal padding inside both vertical border edges and
/// fills the panel interior with black.
///
/// - `rows`: ordered pairs of text and foreground color drawn top to bottom.
/// - `border`: color used for the surrounding box border.
fn build_panel_display(rows: &[(&str, Colour)], border: Colour) -> Display {
    // Reserve 2 cells for left/right borders plus 1 cell of padding on each side.
    let inner_width = PANEL_WIDTH.saturating_sub(4).max(1) as usize;
    let mut wrapped_lines = Vec::new();
    for (text, fg) in rows {
        for line in wrap_lines(text, inner_width) {
            wrapped_lines.push((line, *fg));
        }
    }

    let content_rows = wrapped_lines.len().max(1);
    let total_height = content_rows as isize + 2;
    // Offset (1, 0) inside Container leaves one column of horizontal padding from the left border.
    let block = TextBlock {
        pos_x: 1,
        pos_y: 0,
        lines: wrapped_lines,
    };
    let container = Container::new(0, 0, PANEL_WIDTH as isize, total_height, border, Colour::Black)
        .with_content(block);

    let mut display = Display::new(PANEL_WIDTH, total_height as u32);
    container.draw(&mut display);
    display
}

/// Renders a bordered terminal panel containing colored lines of wrapped text.
///
/// Wraps each input entry to fit inside the container padding and writes the resulting box to
/// standard output through [flush_display].
///
/// - `rows`: ordered pairs of message text and foreground color drawn top to bottom.
/// - `border`: color used for the surrounding box border.
///
/// # Errors
///
/// Returns any standard-output error encountered while reserving space or printing the frame.
fn render_panel(rows: &[(&str, Colour)], border: Colour) -> io::Result<()> {
    let display = build_panel_display(rows, border);
    flush_display(&display)
}

/// Displays the startup welcome panel describing available session commands.
///
/// # Errors
///
/// Returns any standard-output error encountered while writing the panel.
fn render_banner() -> io::Result<()> {
    render_panel(
        &[
            ("turtle — interactive Lua 5.4 REPL", Colour::BrightCyan),
            (
                "Type 'help' for commands, 'quit' to exit, or enter Lua code.",
                Colour::White,
            ),
        ],
        Colour::Cyan,
    )
}

/// Displays the built-in command reference panel.
///
/// # Errors
///
/// Returns any standard-output error encountered while writing the panel.
fn render_help() -> io::Result<()> {
    render_panel(
        &[
            ("Available commands:", Colour::BrightYellow),
            ("  help  - show this command summary", Colour::White),
            ("  quit  - exit the REPL", Colour::White),
            (
                "  <lua> - evaluate a Lua expression or statement",
                Colour::White,
            ),
        ],
        Colour::Yellow,
    )
}

/// Displays the input prompt and optionally positions the cursor inline for interactive typing.
///
/// Flushes [PROMPT_TEXT] in green to standard output. When `inline_cursor` is true, moves the
/// terminal cursor back onto the prompt row just after the trailing space so user input appears on
/// the same line.
///
/// - `inline_cursor`: whether standard input and output are terminals supporting inline editing.
///
/// # Errors
///
/// Returns any standard-output error encountered while writing or positioning the prompt.
fn render_prompt(inline_cursor: bool) -> io::Result<()> {
    let prompt_width = PROMPT_TEXT.chars().count();
    let mut display = Display::new(prompt_width as u32, 1);
    Text::new(0, 0, PROMPT_TEXT, Colour::BrightGreen, Colour::Black).draw(&mut display);
    flush_display(&display)?;

    if inline_cursor {
        // Display::print() ends one row below the grid; move back up and past the prompt text.
        let mut stdout = io::stdout().lock();
        write!(stdout, "\x1b[1F\x1b[{prompt_width}C")?;
        stdout.flush()?;
    }

    Ok(())
}

/// Builds an off-screen character grid containing wrapped lines painted over a black background.
///
/// Returns `None` when `lines` is empty.
///
/// - `lines`: raw output lines whose embedded tabs, carriage returns, and newlines are normalized.
/// - `fg`: foreground color applied to every rendered character.
fn build_lines_display(lines: &[String], fg: Colour) -> Option<Display> {
    if lines.is_empty() {
        return None;
    }

    let mut wrapped = Vec::new();
    for line in lines {
        wrapped.extend(wrap_lines(line, PANEL_WIDTH as usize));
    }

    let height = wrapped.len() as u32;
    let mut display = Display::new(PANEL_WIDTH, height);
    // Display::new() fills cells with a white background; paint black first so unused cells match.
    Rect::new(0, 0, PANEL_WIDTH as isize, height as isize, Colour::Black).draw(&mut display);
    for (row, text) in wrapped.iter().enumerate() {
        Text::new(0, row as isize, text, fg, Colour::Black).draw(&mut display);
    }

    Some(display)
}

/// Renders zero or more lines on a black background with a uniform foreground color.
///
/// Does nothing and returns immediately when `lines` is empty. Otherwise wraps every line to
/// [PANEL_WIDTH], fills the grid background with black, and flushes the frame to standard output.
///
/// - `lines`: raw output lines whose embedded tabs, carriage returns, and newlines are normalized.
/// - `fg`: foreground color applied to every rendered character.
///
/// # Errors
///
/// Returns any standard-output error encountered while writing the frame.
fn render_lines(lines: &[String], fg: Colour) -> io::Result<()> {
    if let Some(display) = build_lines_display(lines, fg) {
        flush_display(&display)?;
    }
    Ok(())
}

/// Renders Lua print output and expression results in bright white text.
///
/// Does nothing when `lines` is empty.
///
/// - `lines`: formatted lines collected from Lua evaluation.
///
/// # Errors
///
/// Returns any standard-output error encountered while writing the frame.
fn render_output(lines: &[String]) -> io::Result<()> {
    render_lines(lines, Colour::BrightWhite)
}

/// Formats and renders a Lua compilation or runtime error in bright red text.
///
/// - `err`: error returned by compiling or running the user's Lua chunk.
///
/// # Errors
///
/// Returns any standard-output error encountered while writing the error frame.
fn render_error(err: &LuaError) -> io::Result<()> {
    let message = format!("Error: {err}");
    render_lines(&[message], Colour::BrightRed)
}

/// Converts any Lua value to a printable string using Lua's global conversion helper.
///
/// Invokes the global `tostring` function inside `lua` so primitives, tables, and custom
/// metamethods format identically to standard Lua output.
///
/// - `lua`: active Lua virtual machine providing the global `tostring` function.
/// - `value`: Lua value to convert into human-readable text.
///
/// # Errors
///
/// Returns a [LuaError] if `tostring` is missing, non-callable, or raises an error inside a
/// metamethod.
fn format_lua_value(lua: &Lua, value: LuaValue) -> LuaResult<String> {
    let tostring: LuaFunction = lua.globals().get("tostring")?;
    tostring.call(value)
}

/// Formats a sequence of Lua values into a single tab-separated line.
///
/// - `lua`: active Lua virtual machine providing global value formatting.
/// - `values`: ordered Lua values returned or passed to print.
///
/// # Errors
///
/// Returns a [LuaError] if formatting any element via [format_lua_value] fails.
fn format_multi_value(lua: &Lua, values: LuaMultiValue) -> LuaResult<String> {
    let mut parts = Vec::with_capacity(values.len());
    for value in values {
        parts.push(format_lua_value(lua, value)?);
    }
    Ok(parts.join("\t"))
}

/// Creates a Lua virtual machine with its print output routed to an in-memory buffer.
///
/// Replaces the default global `print` binding with a Rust callback backed by [OutputBuffer]. Each
/// call formats its variadic arguments through [format_lua_value], joins them with horizontal
/// tabs, and records one line per call.
///
/// # Errors
///
/// Returns a [LuaError] if creating or registering the print callback fails.
fn create_lua_vm() -> LuaResult<(Lua, OutputBuffer)> {
    let lua = Lua::new();
    let output: OutputBuffer = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&output);

    let print_fn = lua.create_function(move |lua, args: LuaMultiValue| {
        let line = format_multi_value(lua, args)?;
        sink.borrow_mut().push(line);
        Ok(())
    })?;
    lua.globals().set("print", print_fn)?;

    Ok((lua, output))
}

/// Evaluates one line of Lua source and returns all captured print and return lines.
///
/// Clears `output` at the start of the call. First attempts to compile the input as a return
/// expression without executing it; if that succeeds, runs the compiled function once, and
/// otherwise falls back to evaluating `source` as a statement chunk. Appends any non-empty
/// returned values formatted through [format_lua_value] after captured print lines.
///
/// - `lua`: shared Lua virtual machine whose global state persists across turns.
/// - `output`: buffer bound to Lua's global print callback for capturing lines during execution.
/// - `source`: user-entered Lua expression or statement text.
///
/// # Errors
///
/// Returns a [LuaError] if `source` has invalid syntax or fails at runtime. Any print lines
/// emitted before a runtime failure remain available in `output`.
fn eval_lua_line(lua: &Lua, output: &OutputBuffer, source: &str) -> LuaResult<Vec<String>> {
    output.borrow_mut().clear();

    // Compile-only check distinguishes expressions from statements without double execution.
    let values = match lua.load(format!("return {source}")).into_function() {
        Ok(func) => func.call::<LuaMultiValue>(())?,
        Err(_) => lua.load(source).eval::<LuaMultiValue>()?,
    };

    let mut lines = std::mem::take(&mut *output.borrow_mut());
    if !values.is_empty() {
        lines.push(format_multi_value(lua, values)?);
    }
    Ok(lines)
}

/// Reads one line from standard input with trailing line endings removed.
///
/// Blocks the calling thread until standard input produces a newline or reaches end-of-file.
/// Returns [None] at end-of-file, or the entered line stripped of trailing carriage-return and
/// newline characters.
///
/// # Errors
///
/// Returns any standard-input error, including non-UTF-8 byte sequences.
fn read_input() -> io::Result<Option<String>> {
    let mut line = String::new();
    let bytes = io::stdin().read_line(&mut line)?;
    if bytes == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim_end_matches(['\r', '\n']).to_string()))
}

/// Runs the interactive prompt loop until the user quits or closes standard input.
///
/// Displays the welcome panel, then prompts once per turn and reads one input line until either
/// end-of-file or [ReplCommand::Quit] arrives. Blank lines re-prompt without touching Lua, help
/// commands display the command panel, and Lua chunks write print output, expression values, or
/// formatted errors to standard output.
///
/// - `lua`: initialized Lua virtual machine whose global state persists across turns.
/// - `output`: buffer populated by Lua's global print callback during chunk evaluation.
///
/// # Errors
///
/// Returns any unrecoverable standard-input or standard-output error encountered during the loop.
fn run_repl(lua: &Lua, output: &OutputBuffer) -> io::Result<()> {
    let inline_cursor = io::stdin().is_terminal() && io::stdout().is_terminal();
    render_banner()?;

    loop {
        render_prompt(inline_cursor)?;
        let Some(line) = read_input()? else {
            if inline_cursor {
                println!();
            }
            break;
        };

        match parse_command(&line) {
            ReplCommand::Blank => continue,
            ReplCommand::Quit => break,
            ReplCommand::Help => render_help()?,
            ReplCommand::Eval(source) => match eval_lua_line(lua, output, source) {
                Ok(lines) => render_output(&lines)?,
                Err(err) => {
                    // Flush any lines printed before the runtime failure occurred.
                    let partial = std::mem::take(&mut *output.borrow_mut());
                    render_output(&partial)?;
                    render_error(&err)?;
                }
            },
        }
    }

    Ok(())
}

/// Initializes the embedded Lua virtual machine and drives the terminal session loop.
///
/// Treats a broken standard-output pipe as a clean exit so piped invocations terminate quietly.
///
/// # Errors
///
/// Returns a [LuaError] if creating the Lua virtual machine fails or if terminal I/O fails with
/// an error other than a broken pipe.
fn main() -> LuaResult<()> {
    let (lua, output) = create_lua_vm()?;
    match run_repl(&lua, &output) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(err) => Err(LuaError::external(err)),
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests covering command parsing, wrapping, terminal grid rendering, and Lua execution.

    use super::*;

    fn row_text(display: &Display, y: isize) -> String {
        (0..display.width() as isize)
            .map(|x| display.pixel(x, y).unwrap().character())
            .collect()
    }

    #[test]
    fn parse_command_classifies_builtins_and_preserves_lua() {
        assert!(matches!(parse_command(""), ReplCommand::Blank));
        assert!(matches!(parse_command("   \t "), ReplCommand::Blank));
        assert!(matches!(parse_command("help"), ReplCommand::Help));
        assert!(matches!(parse_command("  help  "), ReplCommand::Help));
        assert!(matches!(parse_command("quit"), ReplCommand::Quit));
        assert!(matches!(parse_command(" quit "), ReplCommand::Quit));
        assert!(matches!(parse_command("quit()"), ReplCommand::Eval("quit()")));
        assert!(matches!(
            parse_command("  x = 1  "),
            ReplCommand::Eval("  x = 1  ")
        ));
    }

    #[test]
    fn wrap_lines_normalizes_tabs_carriage_returns_and_widths() {
        assert_eq!(wrap_lines("", 10), vec![""]);
        assert_eq!(wrap_lines("a\tb\r\nc", 20), vec!["a    b", "c"]);
        assert_eq!(wrap_lines("abcdef", 4), vec!["abcd", "ef"]);
        assert_eq!(wrap_lines("é☂xyz", 2), vec!["é☂", "xy", "z"]);
    }

    #[test]
    fn eval_handles_expressions_persistence_and_print_capture() {
        let (lua, output) = create_lua_vm().unwrap();

        assert_eq!(eval_lua_line(&lua, &output, "1 + 2").unwrap(), vec!["3"]);
        assert!(eval_lua_line(&lua, &output, "x = 40").unwrap().is_empty());
        assert_eq!(eval_lua_line(&lua, &output, "x + 2").unwrap(), vec!["42"]);
        assert_eq!(
            eval_lua_line(&lua, &output, "print('a', 2, nil)").unwrap(),
            vec!["a\t2\tnil"]
        );
        assert_eq!(
            eval_lua_line(&lua, &output, "nil").unwrap(),
            vec!["nil"]
        );
    }

    #[test]
    fn eval_executes_phase_one_main_lua_file() {
        let (lua, output) = create_lua_vm().unwrap();
        let lines = eval_lua_line(&lua, &output, "dofile(\"main.lua\")").unwrap();
        assert_eq!(
            lines,
            vec![
                "main.lua loaded and executed successfully",
                "Phase 1 validated",
            ]
        );
    }

    #[test]
    fn eval_does_not_double_execute_on_runtime_error() {
        let (lua, output) = create_lua_vm().unwrap();
        let err = eval_lua_line(
            &lua,
            &output,
            "counter = (counter or 0) + 1; error('boom')",
        )
        .unwrap_err();
        assert!(err.to_string().contains("boom"));
        assert_eq!(
            eval_lua_line(&lua, &output, "counter").unwrap(),
            vec!["1"]
        );

        let expr_err = eval_lua_line(
            &lua,
            &output,
            "((function() hits = (hits or 0) + 1; error('bang') end)())",
        )
        .unwrap_err();
        assert!(expr_err.to_string().contains("bang"));
        assert_eq!(eval_lua_line(&lua, &output, "hits").unwrap(), vec!["1"]);
    }

    #[test]
    fn eval_reports_syntax_errors() {
        let (lua, output) = create_lua_vm().unwrap();
        assert!(eval_lua_line(&lua, &output, "1 +").is_err());
    }

    #[test]
    fn panel_and_output_displays_render_wrapped_text_on_black_background() {
        let panel = build_panel_display(&[("hello\tworld", Colour::BrightCyan)], Colour::Cyan);
        assert_eq!(panel.pixel(0, 0).unwrap().character(), '┌');
        assert_eq!(panel.pixel(1, 1).unwrap().background(), Colour::Black);
        assert!(row_text(&panel, 1).contains("hello    world"));

        assert!(build_lines_display(&[], Colour::BrightWhite).is_none());
        let lines = build_lines_display(&["alpha\tbeta\ngamma".to_string()], Colour::BrightWhite)
            .unwrap();
        assert_eq!(lines.height(), 2);
        assert_eq!(lines.pixel(0, 0).unwrap().background(), Colour::Black);
        assert_eq!(lines.pixel(71, 0).unwrap().background(), Colour::Black);
        assert!(row_text(&lines, 0).starts_with("alpha    beta"));
        assert!(row_text(&lines, 1).starts_with("gamma"));
    }
}
