# Code Commenting Guide

This guide is for AI coding agents and the humans who review their work. It applies to all
source in the project it is installed in (conventionally at `docs/COMMENTING.md`). Language-specific
rules for Kotlin, Rust, Python and Lua are in section 6; the linters that enforce the
mechanical part of this guide are in section 8.

## 1. Who the comments are for

The reader is an experienced engineer who can read Kotlin, Rust, Python and Lua well enough to
follow any line. What they lack is time. **Comments exist so the reader can understand what a
file, type or function is *for*, and how to call it, without reading its body.**

- Do **not** explain language features, syntax, or the standard library.
- Do **not** draw analogies to other languages ("like a C++ `unique_ptr`", "like a Python
  generator"). Describe the code on its own terms.
- **Do** explain purpose, inputs, outputs, units, coordinate frames, state, threading,
  failure behavior, and design decisions a careful reader could not deduce from the code.

**Acceptance test (the "skim test"):** a reader who reads only the comments and signatures,
skipping every function body, can say what the file does, where it sits in the pipeline,
what each function takes and returns (with units), and which calls have side effects.

## 2. Required comments

### 2.1 File / module header
At the top of each source file: what the file is for and its place in the system: what feeds it,
what consumes its output, and the main types it defines. A few sentences, not an essay.

- Kotlin: a `/* ... */` block above `package`.
- Rust: `//!` module docs.
- Python: a module docstring.
- Lua: a `--` comment block (or `--[[ ... ]]`) at the top, followed by a blank line. Not `---`,
  which marks a doc comment for the next declaration.

### 2.2 Types (classes, interfaces, objects, enums, data classes, structs, traits)
Every non-trivial type, **including private ones**, gets a doc comment stating:
1. **Purpose**: one sentence: what problem this type solves or what it represents.
2. **Responsibility and state**: what it owns or remembers between calls.
3. **Lifecycle**: how it is created, reset, and released, if that is not obvious.
4. **Threading**: which threads may call it and what synchronization it provides.
5. **Properties / constructor parameters**: as in 2.4.

### 2.3 Functions and methods
Every public or internal function, and every private function that is longer than a few
lines or has side effects, gets a doc comment with:
1. **Summary**: one sentence in terms of the *caller's* goal ("Turns X into Y", "Decides whether Z").
2. **Each parameter**: its role, plus **units**, **coordinate frame**, valid range, and what
   `null` / `None` / `Option::None` means, wherever any of those apply.
3. **Return value**: what it means and its units. Say what "empty" or "zero" means if
   that is ambiguous (for example, "0 means either stationary or unknown").
4. **Side effects**: state mutated, I/O, allocation in hot paths.
5. **Failure behavior**: exceptions thrown, `Result` errors, panics, or silent fallbacks.
6. **Calling contract**: required call order or frequency ("call exactly once per sweep").

Omit items 4–6 when a function is pure, infallible, and has no ordering/threading constraints;
do not pad doc comments with `"Side effects: none"` boilerplate.

- **`override` and trait-implementation methods:** inherit the interface or trait's contract.
  Do not copy it (copies go stale when the interface changes). Add a doc comment only when this
  implementation narrows the contract, adds side effects or synchronization, or fails in a
  concrete way the interface left open.
- **Test files:** include a file header stating what subsystem or invariants the suite covers,
  and comment non-obvious fixtures, tolerances, or scenario setups. Self-explanatory test
  functions (`@Test`, `#[test]`, `test_*`) do not need the six-part contract above.

### 2.4 Properties and fields
Units, coordinate frame, invariants, and who mutates it. Skip it only if the name and
type already say all of that.

### 2.5 Implementation comments (inside bodies)
- **Map comments:** in a function with distinct phases, a one-line comment before each phase
  so the reader can navigate it (e.g. `// 2. Drop votes older than the rolling window.`).
- **Why comments:** before any non-obvious choice: an algorithmic shortcut, a magic number, an
  ordering dependence, a workaround.
- **Behavior a reader would get wrong:** e.g. "Greedy and order-dependent: an earlier candidate
  can claim a track a later one was closer to."

## 3. Prohibited comments

- **Restating the code.** `// increment counter` above `count++`.
- **Copying constant values into prose.** Write "within [clusterRadiusMeters]", not "within 40 mm".
  The prose copy goes stale when the constant changes, which is the most common source of
  comments that contradict the code. Link the symbol instead. Exception: citing the spec that
  *set* the value (see 4).
- **Implementation details in interface docs.** The doc comment says what a function does for its
  caller. How it does it belongs in implementation comments.
- **Change history** ("added in v2", "fixed the bug where…"). That belongs in git.
- **Spec slogans and marketing** ("world-locked ultra-fast voter"). Use plain descriptive language.
- **Cross-language analogies** (see 1).

## 4. Rationale must be sourced, never invented

A "why" comment is a factual claim about someone's intent. Agents must not make one up.

- If the reason is recorded somewhere, **cite it**: `(AGENTS.md §5)`, `(docs/ROADMAP.md, M2)`,
  `(see GpuDetector: its GPU context is bound to one thread)`.
- If the reason can be **derived from the code** (e.g. "y is vertical, so horizontal speed uses x and z"),
  state it plainly.
- If a reader would naturally ask "why?" and no source answers it, write a greppable marker
  instead of a guess:
  `// RATIONALE-UNKNOWN: <the question a reader would ask>`
- Never write "probably", "likely" or "presumably" about intent.

## 5. Keeping comments true

- Any change to code updates the comments that describe it **in the same edit**. A stale
  comment is a bug.
- When a comment and the code disagree, the code defines what happens. Fix the comment, and
  mention the disagreement in your summary to the user, because it may mean the code is what's wrong.

## 6. Language specifics

### Kotlin
- Doc comments are KDoc: `/** ... */` directly above the declaration, Markdown formatting.
- Use `@property` for visible (`public`, `internal`, `protected`) primary-constructor `val`/`var`
  properties, `@param` for `private` constructor properties and plain parameters, `@return`,
  `@throws`. Link symbols with `[Name]` or `[Type.member]`.
- **Never put `/*` inside a comment** (e.g. a glob like `src/**/*.kt`). Kotlin block comments
  nest, so this opens a new comment and breaks compilation. Write `src/**` + "`.kt` files" or use a `//` comment.

### Rust
- `//!` for module/crate docs, `///` for items. Use the standard sections where they apply:
  `# Errors`, `# Panics`, `# Safety`, `# Examples`.
- Every `unsafe` block is preceded by `// SAFETY:` explaining why its preconditions hold.
- **Code blocks in `///` and `//!` are compiled and run as doctests.** Mark anything that is not
  runnable Rust as ` ```text ` or ` ```ignore `.

### Python
- Docstrings (Google style: `Args:`, `Returns:`, `Raises:`, `Attributes:`). Type hints don't
  replace units: write `distance_m: float` or state "meters" in the docstring. Document class or
  dataclass fields in the class docstring's `Attributes:` section, `#` comments, or attribute
  docstrings immediately below the field.

### Lua
- Doc comments are `---` lines directly above the declaration, using LuaLS (LuaCATS)
  annotations: a description, then `---@param name type description` for each parameter (`name?`
  when it may be `nil`), and `---@return type name description` for each return value, one line
  per value. Describe a table's shape with `---@class Name` and one `---@field name type
  description` per field.
- Keep plain `--` for file headers and implementation comments; `---` is read as documentation.
- State the failure convention for every fallible function: whether it raises with `error()` or
  returns `nil, message`. Callers cannot tell from the signature.
- **A long comment ends at the first matching bracket.** `--[[ ... ]]` stops at the first `]]`
  (for example in `t[a[1]]`), and long comments don't nest. If the text contains `]]`, use a
  level: `--[==[ ... ]==]`.

## 7. Example (Kotlin, from the GneissFind project)

Before: generated code. The doc lists spec numbers, says nothing about parameters or return value, and leaves the threading contract out.

```kotlin
/**
 * World-locked multi-frame spatial voter & ARCore/VIO walk-speed gate:
 *  - Maintains a rolling window of `M = 6` sweeps (`windowSize = 6`).
 *  - Associates detections falling within a `40 mm` (`0.040 m`) 3D ground radius.
 */
class TemporalTargetTracker(
    private val minVotesToConfirm: Int = DEFAULT_MIN_VOTES,
```

After: purpose first, constants linked rather than copied, contract stated.

```kotlin
/**
 * Turns noisy per-sweep detections into confirmed stone targets by requiring each stone to be
 * re-detected at the same place on the ground in several recent sweeps. Also classifies the
 * user's walking speed against the over-speed limit.
 *
 * Votes are counted in sweeps, not seconds, so the window's wall-clock length depends on the
 * sweep rate.
 *
 * Thread safety: [updateSweep] and [reset] are synchronized.
 *
 * @param minVotesToConfirm N in N-of-M voting: hits within the window needed to confirm a track.
 * @param rollingWindowSweeps M: how many of the most recent sweeps count toward votes.
 * @param clusterRadiusMeters Maximum world-space distance (m) at which a detection and a track
 *   are treated as the same stone.
 */
```

## 8. Enforcement with doc linters (Tier 1)

Linters check the mechanical part of this guide, so agents don't have to remember it:
public declarations have doc comments, documented parameters match the signature, and
`unsafe` blocks have `SAFETY:` comments. Whether a comment is *useful* (purpose, units, sourced
rationale) remains a matter for this guide and for review. The dexiotites installer adds the
configuration for each language a project uses (the masters are in that repo's
`instructions/lint/`). Each gate fails on documentation findings only:

| Language | Tool and config | Finish-gate command (must pass) |
|---|---|---|
| Kotlin | detekt 2.0, `detekt-comments.yml` | `./gradlew detekt` |
| Rust | rustc, clippy and rustdoc lints, `rust-cargo-lints.toml` | `cargo clippy --all-targets -- -D missing_docs` and `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps` |
| Python | ruff (pydocstyle, Google convention), `ruff-docstrings.toml` | `ruff check --select D .` |
| Lua | lua-language-server (LuaLS), `luarc-docs.json` | `lua-language-server --check=. --checklevel=Warning --configpath=.luarc.json` |

- **New projects:** the gate must pass before you finish any change.
- **Projects that adopted the linter after code existed:** fix every finding in the files you
  changed; pre-existing findings elsewhere are not part of your change. (Kotlin projects record
  them in a detekt baseline file instead.)
- Fix findings by writing real documentation. Never satisfy a linter with placeholder text, and
  never suppress a finding unless the user agrees.

## 9. Retrofitting existing code

Use the `annotate-code` skill. In short: **change comments only**, verify the change with the
skill's `comment_diff_check.py` script, and report, rather than fix, any suspected bugs you notice.

---

## Exemplar codebases (for the human reader)

These are well known for comments that teach the reader. They are worth browsing for calibration.

| Project | Language | What to look at |
|---|---|---|
| SQLite (`src/`) | C | The project states its source is over a third comments: object meanings, invariants, and intent at the top of every function. |
| Redis | C | Function, design and "teacher" comments. Antirez's essay (below) uses Redis as its worked examples. |
| `regex-automata` (BurntSushi) | Rust | Long `//!` module docs explaining the design before any code, and the regex internals written up as a library. |
| Rust `core` / `std` | Rust | Canonical `# Safety` / `# Panics` sections and `// SAFETY:` comments. |
| `kotlinx.coroutines` | Kotlin | Thorough KDoc on concurrency contracts: cancellation, threading, and exception behavior. |
| AndroidX (Jetpack) | Kotlin | Follows the AndroidX KDoc guideline that every parameter and property is documented. |
| PostgreSQL (`src/backend/access/nbtree/README` and neighbors) | C | Per-subsystem design READMEs next to the code: design comments at directory scale. |
| Neovim (`runtime/lua/vim/`) | Lua | Public functions documented with LuaLS annotations from a fixed template; the help pages are generated from them. |

## References (for the human reader)

> **Note to AI agents:** this section is for the human maintainer's further reading. You do not
> need to fetch or read these sources; the rules above are self-contained.

1. Ousterhout, J. (2018; 2nd ed. 2021). *A Philosophy of Software Design*. Yaknyam Press.
   Ch. 12 "Why Write Comments? The Four Excuses", Ch. 13 "Comments Should Describe Things
   that Aren't Obvious from the Code", Ch. 15 "Write the Comments First", Ch. 16 "Modifying
   Existing Code". The source of the interface/implementation comment split used in §2.
2. Ousterhout, J. CS 190 lecture notes, "Writing Comments", Stanford University.
   https://web.stanford.edu/~ouster/cgi-bin/cs190-winter18/lecture.php?topic=comments
3. Sanfilippo, S. ("antirez") (2018). "Writing system software: code comments."
   http://antirez.com/news/124 — A taxonomy of nine comment types (function, design, why,
   teacher, checklist, guide, trivial, debt, backup), drawn from Redis.
4. McConnell, S. (2004). *Code Complete*, 2nd ed., Microsoft Press. Ch. 32 "Self-Documenting Code".
5. Knuth, D. E. (1984). "Literate Programming." *The Computer Journal* 27(2), 97–111.
   The origin of writing programs as explanations for human readers.
6. Rust API Guidelines, "Documentation" (C-CRATE-DOC, C-EXAMPLE, C-FAILURE, C-LINK).
   https://rust-lang.github.io/api-guidelines/documentation.html
7. Rust RFC 1574, "More API Documentation Conventions" (standard section headings).
   https://rust-lang.github.io/rfcs/1574-more-api-documentation-conventions.html
8. Microsoft, *Pragmatic Rust Guidelines*, M-CANONICAL-DOCS.
   https://microsoft.github.io/rust-guidelines/guidelines/docs/
9. JetBrains, "Document Kotlin code: KDoc." https://kotlinlang.org/docs/kotlin-doc.html
10. Android Open Source Project, "Kotlin documentation (KDoc) guidelines" (AndroidX).
    https://android.googlesource.com/platform/frameworks/support/+/HEAD/docs/kdoc_guidelines.md
11. LuaLS, "Annotations" (the LuaCATS format used in section 6).
    https://luals.github.io/wiki/annotations/
12. Neovim, "Lua documentation" developer guidelines (`:help dev-lua-doc`): the template Neovim
    uses for Lua function docs.
13. Ierusalimschy, R., de Figueiredo, L. H., Celes, W. *Lua 5.4 Reference Manual*, §3.1
    "Lexical Conventions" (comments and long brackets). https://www.lua.org/manual/5.4/manual.html#3.1
14. detekt, "Comments rule set." https://detekt.dev/docs/rules/comments
15. Clippy lint list (`missing_errors_doc`, `missing_panics_doc`, `missing_safety_doc`,
    `undocumented_unsafe_blocks`). https://rust-lang.github.io/rust-clippy/master/
16. Ruff, pydocstyle rules and the `convention` setting. https://docs.astral.sh/ruff/rules/#pydocstyle-d
17. LuaLS, "Diagnostics" (`missing-global-doc`, `missing-local-export-doc`,
    `incomplete-signature-doc`). https://luals.github.io/wiki/diagnostics/
