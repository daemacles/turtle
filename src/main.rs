// turtle — learning project: embedding Lua in Rust via mlua (v0.12, Lua 5.4).
// Phase 2: interactive Lua REPL with terminal_graphics rendering.
use mlua::prelude::*;
use terminal_graphics::{Colour, Display, Graphic, Text};

/// Renders a single line of text at a fixed y-coordinate with a foreground and background colour.
///
/// The caller owns the `Display` handle and is responsible for flushing it with `print()`.
/// Coordinates are `(x, y)` with `(0, 0)` at the top-left; `x` grows right, `y` grows down.
fn render_line(display: &mut Display, x: isize, y: isize, text: &str, fg: Colour, bg: Colour) {
    Text::new(x, y, text, fg, bg).draw(display);
}

/// Renders a prompt line at y=1 with a green foreground and black background.
fn render_prompt(display: &mut Display) {
    render_line(display, 0, 1, "turtle> ", Colour::Green, Colour::Black);
}

/// Renders a normal output line at y=2 with a white foreground and black background.
fn render_output(display: &mut Display) {
    render_line(display, 0, 2, "", Colour::White, Colour::Black);
}

/// Renders an error line at y=3 with a red foreground and black background.
fn render_error(display: &mut Display) {
    render_line(display, 0, 3, "", Colour::Red, Colour::Black);
}

/// Renders a Lua result: success prints "OK", failure prints the error.
fn render_lua_result(display: &mut Display, result: LuaResult<()>) {
    match result {
        Ok(()) => {
            render_output(display);
            let _ = display.print();
        }
        Err(err) => {
            render_error(display);
            let _ = display.print();
            println!("Error: {err:?}");
        }
    }
}

/// Reads a single line of input from stdin and returns it without the trailing newline.
fn read_input() -> String {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).expect("failed to read input");
    line.trim_end_matches('\n').to_string()
}

/// Executes a single REPL iteration: prompt, read, parse command, execute Lua.
fn repl_iteration(lua: &Lua) {
    // Render the prompt.
    let mut display = Display::new(80, 24);
    render_prompt(&mut display);
    let _ = display.print();

    // Read the user's input.
    let input = read_input();

    // Parse and dispatch commands.
    match input.as_str() {
        "" => return, // empty line: exit iteration
        "quit" => return, // quit command: exit iteration
        "help" => {
            let _ = display.print();
            println!("Available commands:");
            println!("  help    - show this message");
            println!("  quit    - exit the REPL");
            println!("  print   - print a Lua value");
            return;
        }
        _ => {
            // Execute as Lua code.
            render_lua_result(&mut display, lua.load(&input).exec());
        }
    }
}

/// Runs the REPL loop until the user enters an empty line or "quit".
fn run_repl(lua: &Lua) {
    println!("Welcome to turtle REPL (type 'help' for commands, empty line or 'quit' to exit).");
    loop {
        repl_iteration(lua);
        let input = read_input();
        if input.is_empty() || input == "quit" {
            break;
        }
    }
}

fn main() -> LuaResult<()> {
    let lua = Lua::new();
    // Register a simple `help` command for demonstration.
    lua.create_function(|_: &Lua, _: ()| {
        println!("Available commands:");
        println!("  help    - show this message");
        println!("  quit    - exit the REPL");
        println!("  print   - print a value");
        Ok(())
    })?;
    run_repl(&lua);
    Ok(())
}