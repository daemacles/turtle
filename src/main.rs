// turtle — learning project: embedding Lua in Rust via mlua (v0.12, Lua 5.4).
// Phase 0: run inline Lua code that prints "hello, world".
use mlua::prelude::*;

fn main() -> LuaResult<()> {
    let lua = Lua::new();
    // In mlua 0.12 the old `lua.run_string(...)` became a Chunk builder:
    //   load(source) -> Chunk, then .exec() runs it with no args/return values.
    lua.load(r#"print('hello, world')"#).exec()?;
    Ok(())
}
