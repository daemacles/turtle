<!-- dexiotites:begin (managed by dexiotites install.py: rerun it to update; edits inside this block are overwritten) -->
## Code Comments (Purpose-First Documentation)
Full standard with examples: **`docs/COMMENTING.md`**. Read it before writing or editing comments. Core rules:
- **Reader:** an experienced engineer who can read any language but should not have to read function bodies to understand purpose. No explanations of language features; no analogies to other languages.
- **Required:** a header on every file; a doc comment on every non-trivial type (purpose, state, lifecycle, threading); a doc comment on every non-trivial function (one-line purpose, each parameter with units/coordinate frame/valid range/meaning of `null`, return value, plus side effects, failure behavior, and calling contract where present/non-obvious). Omit `"None"` boilerplate on pure, infallible, unrestricted helpers. `override` and trait-impl methods inherit interface docs; add only implementation-specific behavior or constraints. Test files need a file header and scenario/invariant comments, not full function contracts on self-explanatory `@Test`s.
- **Prohibited:** restating the code; copying constant values into prose (link the constant instead); change history; spec slogans.
- **Never invent rationale:** cite the source (`AGENTS.md §N`, `docs/…`), derive it from the code, or write `RATIONALE-UNKNOWN: <question>`.
- **Keep comments true:** any code change updates the comments describing it in the same edit.
- **Doc linter gate:** before finishing, run this project's doc linter (`docs/COMMENTING.md` §8) and fix every finding in the files you changed with real documentation, not placeholders or suppressions.
- **Retrofitting existing code:** use the `annotate-code` skill (`.agents/skills/annotate-code/`). Change comments only, and verify with its `comment_diff_check.py`.
<!-- dexiotites:end -->
