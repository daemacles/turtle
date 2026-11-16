---
name: "annotate-code"
description: "Retrofit existing source files with purpose-focused comments that follow the project's COMMENTING.md, changing comments only and verifying that the diff is comment-only. Use when asked to annotate, document, comment, or \"add comments to\" existing code, or to bring older generated code up to the commenting guide."
---

# Annotate existing code (comment-only retrofit)

Goal: after this pass, a reader can understand each file by reading only its comments and
signatures (the "skim test" in COMMENTING.md). No behavior changes.

This skill runs in two settings:
- **Installed in a project** (`.agents/skills/annotate-code/`, linked from `.claude/skills/`):
  the checker script is at `scripts/comment_diff_check.py` next to this file.
- **As an account skill:** there is no scripts folder. Use the copy of the checker embedded at
  the end of this file.

## Steps

1. **Find and read the standard.** Look for the project's commenting guide at
   `docs/COMMENTING.md`, then `COMMENTING.md` in the repo root, or wherever the project's
   `AGENTS.md` or `CLAUDE.md` points. Read it in full. If the project has none, **stop** and ask
   the user to install one. The master copy is `instructions/COMMENTING.md` in the dexiotites
   repo (`github.com/daemacles/dexiotites`). Never comment against a remembered or improvised standard.

2. **Scope.** Work only on the files or directories the user named. If none were named, ask.
   Do one file at a time and finish it before starting the next.

3. **Check the starting point.** The checker compares files against git `HEAD`, so target
   files must be tracked and have no uncommitted code changes. Run `python3 <checker> --strict <files>`
   (step 9) on them *before* editing:
   - If any file reports `FAIL` or `NEW`, stop and ask the user to commit a baseline first.
   - If any file reports `SKIP` (unsupported language: the checker supports Kotlin `.kt`/`.kts`,
     Rust `.rs`, Python `.py`, and Lua `.lua`), tell the user automated comment-only verification
     does not support that file type, and ask whether to proceed using manual `git diff`
     inspection + build verification instead.
   - If the project isn't a git repository, tell the user the change can't be verified and ask
     whether to proceed.

4. **Understand before writing.** For each file:
   - Read the whole file, plus the types it uses and its callers (search for the
     type/function names). Comments about units, frames, call frequency and threading must
     come from how the code is actually used, not from names.
   - Read the parts of the project's `AGENTS.md`, rules files and `docs/` that bear on the file,
     so you can *cite* design decisions instead of guessing them.

5. **Write the comments** required by section 2 of the guide: file header, every non-trivial
   type, every non-trivial function (summary, each parameter, return, plus side effects, failure
   behavior, and calling contract where present/non-obvious), fields where units or invariants
   aren't obvious, and map/why comments inside long bodies. Respect the guide's rules on
   `override`/trait-impl inheritance and test files, and never pad pure functions with `"None"`
   boilerplate.

6. **Rewrite existing comments that break section 3.** Remove copied constant values (link the
   constant instead), restated code, and spec slogans. Correct any comment the code contradicts.

7. **Never invent rationale (section 4).** Cite a source, derive the reason from code, or write
   a `RATIONALE-UNKNOWN: <question>` comment (in the file's comment syntax). Before saving,
   re-check every factual claim you wrote (who calls this, where a value is displayed, what a
   flag means) against the code.

8. **Do not change code.** No renames, import changes, or "small fixes". Splitting a parameter
   or argument list across lines inside its `(...)` to attach per-parameter comments is fine and
   accepted by the checker; in Kotlin it rejects any other new line break, because a break
   outside brackets can end a statement. If you find a likely bug, dead code, or a mismatch
   between code and spec, **report it to the user in your summary** and leave the code
   untouched.

9. **Verify.** Locate the checker: `scripts/comment_diff_check.py` next to this file if it
   exists; otherwise write the embedded copy below to a temporary file **outside the project**
   (e.g. `/tmp/comment_diff_check.py`). From the project root, run:
   ```
   python3 <checker> --strict <the files you edited>
   ```
   Every supported file must report `OK (comment-only)` (exit code 0). If any report `FAIL`, undo
   the code change and rerun. Then confirm the build still compiles, because a comment can break
   compilation (a stray `/*` inside a Kotlin comment, a doctest in Rust, a `]]` inside a Lua
   `--[[ ]]` comment). Use the project's build command, e.g. `./gradlew compileDebugKotlin` or
   `cargo test --doc`. If you can't run the build, say so.

   Then run the project's **doc linter** if it has one (the commenting guide's section 8 lists the
   standard commands: detekt, `cargo clippy`/`cargo doc`, `ruff check --select D`,
   `lua-language-server --check`). Fix every finding in the files you edited, using comment
   changes only, and rerun the checker afterwards. Findings in files you didn't touch are not
   yours to fix; mention their count in the summary.

10. **Summarize for the user:** files annotated, comments corrected because the code
    contradicted them, every `RATIONALE-UNKNOWN` you added (these are questions for the user),
    and suspected bugs you noticed but did not fix.

## Embedded checker

This block must stay identical to `scripts/comment_diff_check.py`. In the dexiotites repo, the
sync-skills-repo skill regenerates that file from this block, so edit the script here.

<!-- embedded-file: scripts/comment_diff_check.py -->
```python
#!/usr/bin/env python3
"""Verify that a source change touched only comments.

Compares each changed source file against a git revision after stripping comments
(and, for Python, module/class/function/attribute docstrings) from both versions
and tokenizing code so whitespace and line-wrapping around comments do not count
as differences. String and character literals are preserved verbatim (with comments
stripped inside Kotlin `${...}` template expressions). Any remaining difference
means code changed.

Line breaks are ignored except where the language gives them meaning: in Kotlin, a
line break outside `(...)` and `[...]` can end a statement, so it is kept as a token
there and moving code across such a break counts as a code change.

Usage:
    python3 comment_diff_check.py                         # all files changed vs HEAD
    python3 comment_diff_check.py --base main             # compare against another revision
    python3 comment_diff_check.py --strict path/to/A.kt   # fail on NEW or SKIP files too

Exit status: 0 if every checked file is comment-only (and none were NEW/SKIP under
--strict), 1 if code changed (or NEW/SKIP under --strict), 2 on usage or git errors.

Supported: Kotlin (.kt, .kts), Rust (.rs), Python (.py), Lua (.lua). Other files
report SKIP. New (untracked or added) files have no base version and report NEW.
Requires Python 3.9 or newer.
"""

from __future__ import annotations

import argparse
import ast
import re
import subprocess
import sys
from pathlib import Path

C_LIKE = {".kt", ".kts", ".rs"}
LUA = {".lua"}

# Separates identifiers/numbers from punctuation so wrapping `f(a, b)` across lines
# to comment individual parameters keeps the exact same token stream, while preserving
# multi-character operators (`==`, `->`, `?.`, `+=`, `..=`, `~=`, etc.).
TOKEN_RE = re.compile(
    r"\w+|===|!==|==|!=|~=|<=|>=|&&|\|\||->|=>|::|\.\.\.|\.\.=?|<<=?|>>=?|\?[.:]|!!|[+\-*/%&|^]=|\+\+|--|\S"
)

# Token standing for a meaningful (Kotlin statement-separating) line break. No code or
# literal token can equal it: TOKEN_RE never matches whitespace, and literals keep quotes.
NL = "\n"


def strip_c_like(src: str, rust: bool) -> list[str]:
    """Return the code of a Kotlin or Rust file as tokens with comments removed.

    Args:
        src: Full source text of the file.
        rust: True for Rust lexical rules (raw `r#""#`/`br#""#`/`cr#""#` strings,
            lifetimes); False for Kotlin rules (triple-quoted raw strings, `${...}` and
            multi-dollar string templates, meaningful line breaks).

    Returns:
        Ordered list of code and literal tokens. Block comments nest in both languages.
        For Kotlin, each run of meaningful line breaks becomes one `NL` token, so adding
        comment lines between statements does not change the stream.
    """
    tokens, _ = _scan_c_like(src, 0, rust=rust, stop_on_rbrace=False)
    if rust:
        return tokens
    collapsed: list[str] = []
    for tok in tokens:
        if tok == NL and (not collapsed or collapsed[-1] == NL):
            continue
        collapsed.append(tok)
    while collapsed and collapsed[-1] == NL:
        collapsed.pop()
    return collapsed


def _scan_c_like(src: str, i: int, rust: bool, stop_on_rbrace: bool) -> tuple[list[str], int]:
    """Scan Kotlin/Rust code from index `i`, optionally stopping at a matching `}`.

    For Kotlin, tracks open `(`, `[` and `{` so a line break emits an `NL` token only
    where Kotlin treats it as a possible statement end: at top level or directly inside
    `{...}` (class, function and lambda bodies), but not inside `(...)` or `[...]`.
    """
    tokens: list[str] = []
    buf: list[str] = []
    n = len(src)
    brace_depth = 1 if stop_on_rbrace else 0
    open_brackets: list[str] = []                          # Kotlin only

    def flush() -> None:
        if buf:
            tokens.extend(TOKEN_RE.findall("".join(buf)))
            buf.clear()

    def track(ch: str) -> None:
        if ch in "([{":
            open_brackets.append(ch)
        elif ch in ")]}" and open_brackets:
            open_brackets.pop()

    while i < n:
        c = src[i]
        two = src[i:i + 2]
        if two == "//":                                   # line comment (incl. ///, //!)
            j = src.find("\n", i)
            i = n if j < 0 else j
            buf.append(" ")
        elif two == "/*":                                 # nested block comment
            depth, i = 1, i + 2
            while i < n and depth:
                if src.startswith("/*", i):
                    depth, i = depth + 1, i + 2
                elif src.startswith("*/", i):
                    depth, i = depth - 1, i + 2
                else:
                    i += 1
            buf.append(" ")
        elif stop_on_rbrace and c == "{":
            brace_depth += 1
            track(c)
            buf.append(c)
            i += 1
        elif stop_on_rbrace and c == "}":
            brace_depth -= 1
            if brace_depth == 0:
                flush()
                return tokens, i + 1
            track(c)
            buf.append(c)
            i += 1
        elif not rust and c == "\n" and (not open_brackets or open_brackets[-1] == "{"):
            flush()                                       # Kotlin: possible statement end
            tokens.append(NL)
            i += 1
        elif not rust and c == "$":                       # Kotlin multi-dollar string `$$"..."`
            k = i
            while k < n and src[k] == "$":
                k += 1
            is_ident_tail = i > 0 and (src[i - 1].isalnum() or src[i - 1] == "_")
            if k < n and src[k] == '"' and not is_ident_tail:
                flush()
                str_tokens, i = _scan_kotlin_string(src, i, k, k - i)
                tokens.extend(str_tokens)
            else:
                buf.append(src[i:k])
                i = k
        elif not rust and c == '"':                       # Kotlin string or raw `"""` string
            flush()
            str_tokens, i = _scan_kotlin_string(src, i, i, 1)
            tokens.extend(str_tokens)
        elif rust and _rust_raw_prefix_len(src, i) > 0:   # Rust r#"..."#, br#"..."#, cr#"..."#
            j = _rust_raw_end(src, i)
            flush()
            tokens.append(src[i:j])
            i = j
        elif rust and c == '"':                           # Rust ordinary string
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            j = min(n, j + 1)
            flush()
            tokens.append(src[i:j])
            i = j
        elif c == "'":                                    # char literal or Rust lifetime
            j = _char_literal_end(src, i)
            if j > i:
                flush()
                tokens.append(src[i:j])
                i = j
            else:
                buf.append(c)
                i += 1
        else:
            if not rust:
                track(c)
            buf.append(c)
            i += 1

    flush()
    return tokens, i


def _scan_kotlin_string(
    src: str, prefix_start: int, quote_idx: int, interp_dollars: int
) -> tuple[list[str], int]:
    """Scan a Kotlin string literal, preserving literal text and tokenizing `${...}` blocks."""
    n = len(src)
    triple = src.startswith('"""', quote_idx)
    j = quote_idx + (3 if triple else 1)
    seg_start = prefix_start
    out: list[str] = []

    while j < n:
        if triple and src.startswith('"""', j):
            j += 3
            while j < n and src[j] == '"':                # trailing quotes in `"""foo""""`
                j += 1
            out.append(src[seg_start:j])
            return out, j
        if not triple and src[j] == '"':
            j += 1
            out.append(src[seg_start:j])
            return out, j
        if not triple and src[j] == "\\":
            j += 2
            continue
        if src[j] == "$":
            k = j
            while k < n and src[k] == "$":
                k += 1
            if k < n and src[k] == "{" and (k - j) >= interp_dollars:
                # Any extra leading `$` characters before the `interp_dollars` opener stay literal.
                out.append(src[seg_start:k + 1])
                expr_tokens, j = _scan_c_like(src, k + 1, rust=False, stop_on_rbrace=True)
                out.extend(expr_tokens)
                seg_start = j - 1                         # starts closing/next segment at `}`
                continue
            j = k
            continue
        j += 1

    out.append(src[seg_start:n])
    return out, n


def _char_literal_end(src: str, i: int) -> int:
    """Return index past closing `'` if `src[i:]` starts a char literal, else `i`."""
    n = len(src)
    if i + 2 >= n or src[i] != "'":
        return i
    if src[i + 1] == "\\":
        j = i + 3                                         # past opening `'`, `\`, and escaped char
        while j < n and src[j] not in ("'", "\n") and j - i <= 14:
            j += 1
        return j + 1 if j < n and src[j] == "'" else i
    if src[i + 1] not in ("'", "\n") and src[i + 2] == "'":
        return i + 3
    return i


def _rust_raw_prefix_len(src: str, i: int) -> int:
    """Length of a Rust raw string prefix (`r`, `br`, `cr`) at `i`, or 0 if not one."""
    if i > 0 and (src[i - 1].isalnum() or src[i - 1] == "_"):
        return 0
    plen = 2 if src.startswith(("br", "cr"), i) else (1 if src[i] == "r" else 0)
    if plen == 0:
        return 0
    j = i + plen
    while j < len(src) and src[j] == "#":
        j += 1
    return plen if j < len(src) and src[j] == '"' else 0


def _rust_raw_end(src: str, i: int) -> int:
    """Index just past the Rust raw string literal starting at `i`."""
    j = i + _rust_raw_prefix_len(src, i)
    hashes = 0
    while src[j] == "#":
        hashes, j = hashes + 1, j + 1
    end = src.find('"' + "#" * hashes, j + 1)
    return len(src) if end < 0 else end + 1 + hashes


def strip_lua(src: str) -> list[str]:
    r"""Return the code of a Lua file as tokens with comments removed.

    Follows the Lua 5.1-5.4 lexical rules: `--` line comments; `--[[...]]` and
    `--[==[...]==]` long comments, which do not nest; single- or double-quoted strings
    with backslash escapes (including `\z`, which skips the following whitespace); and
    `[[...]]` / `[==[...]==]` long strings. Strings are kept verbatim as single tokens.
    `//` is floor division (Lua 5.3+), never a comment. Line breaks carry no meaning in
    Lua, so line wrapping does not count as a change.

    Args:
        src: Full source text of the file.

    Returns:
        Ordered list of code and literal tokens.
    """
    tokens: list[str] = []
    buf: list[str] = []
    i, n = 0, len(src)

    def flush() -> None:
        if buf:
            tokens.extend(TOKEN_RE.findall("".join(buf)))
            buf.clear()

    while i < n:
        c = src[i]
        if src.startswith("--", i):
            level = _lua_long_bracket_level(src, i + 2)
            if level >= 0:                                # long comment --[==[ ... ]==]
                close = "]" + "=" * level + "]"
                j = src.find(close, i + 4 + level)
                i = n if j < 0 else j + len(close)
            else:                                         # line comment
                j = src.find("\n", i)
                i = n if j < 0 else j
            buf.append(" ")
        elif c == "[" and _lua_long_bracket_level(src, i) >= 0:
            level = _lua_long_bracket_level(src, i)      # long string [==[ ... ]==]
            close = "]" + "=" * level + "]"
            j = src.find(close, i + 2 + level)
            j = n if j < 0 else j + len(close)
            flush()
            tokens.append(src[i:j])
            i = j
        elif c in ("'", '"'):                             # short string
            j = i + 1
            while j < n and src[j] != c and src[j] != "\n":
                if src[j] == "\\":
                    if src.startswith("\\z", j):
                        j += 2
                        while j < n and src[j].isspace():
                            j += 1
                        continue
                    j += 2
                    continue
                j += 1
            j = min(n, j + 1)
            flush()
            tokens.append(src[i:j])
            i = j
        else:
            buf.append(c)
            i += 1

    flush()
    return tokens


def _lua_long_bracket_level(src: str, i: int) -> int:
    """Return the level of a Lua opening long bracket (`[`, `=` * level, `[`) at `i`, else -1."""
    if i >= len(src) or src[i] != "[":
        return -1
    j = i + 1
    while j < len(src) and src[j] == "=":
        j += 1
    return j - i - 1 if j < len(src) and src[j] == "[" else -1


def _is_str_expr(stmt: ast.stmt) -> bool:
    """True if `stmt` is a bare string literal statement (`Expr(Constant(str))`)."""
    return (
        isinstance(stmt, ast.Expr)
        and isinstance(getattr(stmt, "value", None), ast.Constant)
        and isinstance(stmt.value.value, str)
    )


def strip_python(src: str) -> list[str]:
    """Return canonical indented AST lines with docstrings removed.

    Strips module, class, and function docstrings (`body[0]`), plus PEP 257 /
    dataclass attribute docstrings (a bare string literal immediately following an
    assignment or annotated assignment in a module or class body). Comments never
    enter the AST.
    """
    tree = ast.parse(src)
    for node in ast.walk(tree):
        if isinstance(node, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)):
            orig = node.body
            allow_attr_docs = isinstance(node, (ast.Module, ast.ClassDef))
            cleaned: list[ast.stmt] = []
            for idx, stmt in enumerate(orig):
                if idx == 0 and _is_str_expr(stmt):
                    continue
                if (
                    allow_attr_docs
                    and idx > 0
                    and _is_str_expr(stmt)
                    and isinstance(orig[idx - 1], (ast.Assign, ast.AnnAssign))
                ):
                    continue
                cleaned.append(stmt)
            node.body = cleaned or [ast.Pass()]
    return ast.dump(tree, include_attributes=False, indent=2).splitlines()


def code_only(path: str, src: str) -> tuple[str, list[str]] | None:
    """Return `('tokens' | 'ast', items)` for supported files, or `None` if unsupported."""
    ext = Path(path).suffix
    if ext in C_LIKE:
        return ("tokens", strip_c_like(src, rust=(ext == ".rs")))
    if ext in LUA:
        return ("tokens", strip_lua(src))
    if ext == ".py":
        return ("ast", strip_python(src))
    return None


def git(*args: str) -> str:
    """Run a read-only git command and return stdout."""
    return subprocess.run(
        ["git", "--no-optional-locks", *args],
        check=True,
        capture_output=True,
        text=True,
    ).stdout


def first_divergence(kind: str, a: list[str], b: list[str]) -> str:
    """Format a short snippet around the first differing token or AST line."""
    def show(toks: list[str]) -> str:
        return " ".join("<line break>" if t == NL else t for t in toks)

    for k, (x, y) in enumerate(zip(a, b)):
        if x != y:
            if kind == "ast":
                return f"  base AST:    {x.strip()}\n  current AST: {y.strip()}"
            return (
                f"  base:    ...{show(a[max(0, k - 6):k + 6])}...\n"
                f"  current: ...{show(b[max(0, k - 6):k + 6])}..."
            )
    unit = "AST lines" if kind == "ast" else "tokens"
    return f"  {unit} differ in length ({len(a)} vs {len(b)}): code added or removed at the end"


def main() -> int:
    """Check each path (default: files changed vs `--base`); return the exit status in the module doc."""
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("paths", nargs="*", help="files to check (default: all changed vs base)")
    ap.add_argument("--base", default="HEAD", help="git revision to compare against")
    ap.add_argument(
        "--strict",
        action="store_true",
        help="fail if any checked path is NEW (no base version) or SKIP (unsupported file type)",
    )
    args = ap.parse_args()

    try:
        root = git("rev-parse", "--show-toplevel").strip()
        paths = args.paths or [p for p in git("diff", "--name-only", args.base).splitlines() if p]
    except subprocess.CalledProcessError as e:
        print(f"git error: {e.stderr.strip()}", file=sys.stderr)
        return 2

    if not paths:
        print("No changed files.")
        return 0

    failed = False
    for p in paths:
        rel = str(Path(p).resolve().relative_to(root)) if Path(p).exists() else p
        cur_path = Path(root) / rel
        if not cur_path.exists():
            print(f"DELETED  {rel}  (deleting a file is not comment-only)")
            failed = True
            continue
        current = cur_path.read_text(encoding="utf-8")
        try:
            base = git("show", f"{args.base}:{rel}")
        except subprocess.CalledProcessError:
            suffix = "  [FAIL under --strict]" if args.strict else ""
            print(f"NEW      {rel}  (no base version; not checked){suffix}")
            if args.strict:
                failed = True
            continue
        try:
            parsed_a, parsed_b = code_only(rel, base), code_only(rel, current)
        except SyntaxError as e:
            print(f"FAIL     {rel}  (does not parse: {e})")
            failed = True
            continue
        if parsed_a is None or parsed_b is None:
            suffix = "  [FAIL under --strict]" if args.strict else ""
            print(f"SKIP     {rel}  (unsupported file type){suffix}")
            if args.strict:
                failed = True
        elif parsed_a == parsed_b:
            print(f"OK       {rel}  (comment-only)")
        else:
            kind, a = parsed_a
            _, b = parsed_b
            print(f"FAIL     {rel}  (code changed)")
            print(first_divergence(kind, a, b))
            failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
```
