# turtle — LEDGER (external memory / state file)

**New session? Read this whole file first**, then `README.md`, then run `git log --oneline` and `ls`. Everything needed to resume is in here. Do not redo validated work. Before handing off, update the "Current state" section and append a dated entry under Session log.

## Project snapshot
- Name: **turtle** — learning project for embedding Lua in Rust.
- Location: workspace dir `lmstudio/`; git repo rooted at `turtle/`, branch `main`, no remote yet.
- Stack: Rust 1.95, mlua v0.12.2 pinned with features `["lua54", "vendored"]`.
- Process rule (user-set): user validates each phase (`cargo run`) before authorizing the next; Phase 2+ are TBD until after Phase 1 is validated.

## Phase board
| Phase | Description                                                                                          | Status                                                        |
| ----- | ----------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| 0     | Rust app runs inline Lua code that prints `hello, world`                                              | ✅ built & smoke-tested locally — validated by user (2026-11-14) |
| 1     | Load and run a hardcoded `main.lua` file next to the project. Assumed location: CWD-relative `turtle/main.lua`, invoked from inside `turtle/` | ✅ built & smoke-tested locally — validated by user (2026-11-14) |
| 2–N   | TBD                                                                                                    | user will define after Phase 1 validation                     |

## Current state (most recent)
- Phase 1 implemented in `src/main.rs`; `cargo run` prints exactly the two lines from `main.lua`, exit 0. Agent smoke-tested on this machine; validated by user.
- Docs: README.md + LEDGER.md updated to reflect Phase 1 completion.

## How to run right now
```bash
cd turtle && cargo run
```
Expected output:
```
main.lua loaded and executed successfully
Phase 1 validated
```

## Key decisions & gotchas (do not regress)
1. **Vendored Lua**: without the `vendored` feature mlua builds fail on machines lacking system liblua54 (`pkg-config` can't find `lua54.pc`). Keep features = `["lua54", "vendored"]`.
2. **mlua 0.12 API ≠ older tutorials**: there is no `Lua::run_string` anymore (it compiled-fails). The idiom is `lua.load(source).exec()?;` — a Chunk builder; use file/string sources for Phase 1+.
3. **File loading in 0.12**: `Lua::load_file(path)` does not exist. Read the file into a string and pass it to `lua.load(&source).exec()?;` — this is the safest cross-version approach.
4. Git identity set **locally only** in this repo: placeholder `jay <jay@localhost>` (global config was empty) — user may want to change it.
5. Commit style: one commit per significant change, brief imperative sentence. Cargo.lock is committed (binary project).
6. Do NOT start Phase 2 until the user validates Phase 1 and says go (explicit gate rule).

## Next steps
- [ ] Wait for user to validate Phase 1 (`cargo run` → two lines, exit 0).
- [ ] On go-ahead: interview user about Phase 2+ scope, record it in this phase board **before** implementing.

## Session log (append-only)
### 2026-11-13 — session 1 (setup + Phase 0)
- Created repo on branch `main`; local git identity placeholder set.
- Scaffold commit: mlua 0.12.2 dep, `.gitignore` (`/target`).
- First build failed (no system liblua54 found via pkg-config) → added `vendored` feature; builds clean after that.
- Phase 0 smoke-tested OK (`hello, world`, exit 0); committed d0424b3.
- Wrote README.md + LEDGER.md (their commit follows this entry).

### 2026-11-14 — session 2 (Phase 1)
- Created `main.lua` in repo root; updated `src/main.rs` to read file into a string and pass to `lua.load(&source).exec()?`.
- Smoke-test OK (`main.lua loaded and executed successfully`, `Phase 1 validated`, exit 0); committed 4863170.
- Updated README.md + LEDGER.md to reflect Phase 1 completion.
