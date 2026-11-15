// turtle — learning project: embedding Lua in Rust via mlua (v0.12, Lua 5.4).
// Phase 1: load and execute a hardcoded main.lua file next to the project.
use mlua::prelude::*;

fn main() -> LuaResult<()> {
    let lua = Lua::new();
    // In mlua 0.12 the old `lua.run_string(...)` became a Chunk builder:
    //   load(source) -> Chunk, then .exec() runs it with no args/return values.
    // For files, read the content into a string and pass it to `lua.load(source)`.
    let source = std::fs::read_to_string("main.lua")?;
    lua.load(&source).exec()?;
    Ok(())
}
