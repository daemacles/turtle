#!/usr/bin/env python3
r"""Report how hard a file's doc comments are to read, by the sentence rules of COMMENTING.md.

The commenting guide's section 10 ("Voice") limits what one sentence of a doc comment may ask
a reader to hold in mind. This script counts the things that section names and prints one
warning per sentence that breaks a rule, then a score per file. It judges form only. It cannot
tell whether a comment is true, whether the purpose is stated, or whether the caller's contract
is complete: a reader has to judge those, with this report as input.

Usage:
    python3 comment_load.py [--fail-over SCORE] FILE [FILE ...]

Doc comments recognized: `/** ... */` (Kotlin), `///` and `//!` runs (Rust), triple-quoted
docstrings (Python), `---` runs (Lua). Other file types are reported as SKIP.

Warnings (the number in brackets is the rule in section 10):
    W1 [10.2.1]  first sentence contains a code name, a number or a path
    W2 [10.2.3]  more than two code names in one sentence
    W3 [10.2.4]  parenthetical inside a sentence
    W4 [10.2.5]  a value copied into prose (a code span containing a digit)
    W5 [10.3]    points to another file or to a numbered section
    W6 [10.1]    names a private member (Kotlin `private`, Python leading underscore)

The score is warnings per 100 sentences. The report is advisory: a warning can be right to
leave in place (W4 on a file name or data format, which the guide keeps in doc comments).
Exit code is 0, or 1 when `--fail-over` is given and any file's score exceeds it.
Requires Python 3.9 or newer.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# Per file suffix: a pattern whose group 1 is one doc comment's raw text.
DOC_PATTERNS = {
    ".kt": re.compile(r"/\*\*(.*?)\*/", re.S),
    ".kts": re.compile(r"/\*\*(.*?)\*/", re.S),
    ".rs": re.compile(r"((?:^[ \t]*//[/!][^\n]*\n)+)", re.M),
    ".py": re.compile(r'^[ \t]*[rR]?"""(.*?)"""', re.S | re.M),
    ".lua": re.compile(r"((?:^[ \t]*---[^\n]*\n)+)", re.M),
}
LINE_PREFIX = re.compile(r"^\s*(?:\*|//[/!]|---)? ?")
TAG_LINE = re.compile(r"^\s*(?:[@\\]\w+|---@|\w[\w ]*:\s*$|#+ )")
CODE_NAME = re.compile(r"`[^`\n]+`|\[[A-Za-z_][\w.]*\]")
PRIVATE_DECL = re.compile(r"\bprivate\s+(?:[a-z]+\s+)*(?:val|var|fun)\s+([A-Za-z_]\w*)")
CHECKS = {
    "W1": "first sentence contains a code name, number or path",
    "W2": "more than two code names in one sentence",
    "W3": "parenthetical inside a sentence",
    "W4": "value copied into prose",
    "W5": "points to another file or numbered section",
    "W6": "names a private member",
}


def prose_lines(raw: str) -> list[tuple[int, str]]:
    """Return a doc comment's prose as (line offset, text) pairs.

    Comment markers are stripped. Tag lines, the indented lines of a tagged section, and
    fenced or indented code examples are left out: the sentence rules apply to prose only.
    A tag line starts with an at-sign or backslash tag, or is a heading.
    """
    out, fenced, in_tags = [], False, False
    for offset, line in enumerate(raw.splitlines()):
        text = LINE_PREFIX.sub("", line, count=1).rstrip()
        if text.strip().startswith("```"):
            fenced = not fenced
            continue
        if fenced or text.startswith("    "):
            continue
        if TAG_LINE.match(text):
            in_tags = True
            continue
        if not text.strip():
            in_tags = False
        if not in_tags:
            out.append((offset, text.strip()))
    return out


def sentences(lines: list[tuple[int, str]]) -> list[tuple[int, str]]:
    """Join prose lines into sentences, each with the line offset where it starts.

    A sentence ends at a full stop, colon or semicolon followed by a space, at a blank line,
    or where a list item starts. Fragments of fewer than three words are dropped.
    """
    out, current, start = [], [], 0
    for offset, text in lines:
        if not text or re.match(r"[-*] |\d+\. ", text):
            if current:
                out.append((start, " ".join(current)))
            current = []
            text = re.sub(r"^(?:[-*]|\d+\.) ", "", text)
        for part in re.split(r"(?<=[.:;])\s+", text):
            if not part:
                continue
            if not current:
                start = offset
            current.append(part)
            if part[-1] in ".:;":
                out.append((start, " ".join(current)))
                current = []
    if current:
        out.append((start, " ".join(current)))
    return [(o, s) for o, s in out if len(s.split()) >= 3]


def check_sentence(text: str, first: bool, private: set[str]) -> list[str]:
    """Return the ids of the checks one sentence fails; see the module docstring for the ids."""
    names = CODE_NAME.findall(text)
    found = []
    if first and (names or re.search(r"\d|[\w-]+/[\w./-]+", text)):
        found.append("W1")
    if len(names) > 2:
        found.append("W2")
    if re.search(r"(?<!\w)\([^)]+\)", CODE_NAME.sub("x", text)):
        found.append("W3")
    if any(re.search(r"\d", n) for n in names if n.startswith("`")):
        found.append("W4")
    if re.search(r"[\w./-]+\.(?:md|kt|kts|rs|py|lua|toml|json)\b|§", text):
        found.append("W5")
    bare = {n.strip("`[]").split(".")[-1].rstrip("()") for n in names}
    if bare & private or any(b.startswith("_") and not b.startswith("__") for b in bare):
        found.append("W6")
    return found


def report(path: Path) -> tuple[int, int, list[str]] | None:
    """Check every doc comment in one file.

    Returns:
        The number of sentences, the number of warnings, and the warning lines to print; or
        None if the file type is not supported.
    """
    pattern = DOC_PATTERNS.get(path.suffix)
    if pattern is None:
        return None
    source = path.read_text(encoding="utf-8")
    private = set(PRIVATE_DECL.findall(source)) if path.suffix in (".kt", ".kts") else set()
    total, lines = 0, []
    for match in pattern.finditer(source):
        first_line = source.count("\n", 0, match.start(1)) + 1
        for index, (offset, text) in enumerate(sentences(prose_lines(match.group(1)))):
            total += 1
            for check in check_sentence(text, index == 0, private):
                excerpt = text if len(text) <= 70 else text[:67] + "..."
                lines.append(f"{path}:{first_line + offset}: {check} {CHECKS[check]}: {excerpt}")
    return total, len(lines), lines


def main() -> int:
    """Print warnings and a score for each file named on the command line."""
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("files", nargs="+", type=Path)
    ap.add_argument("--fail-over", type=float, metavar="SCORE",
                    help="exit 1 if any file scores above SCORE warnings per 100 sentences")
    args = ap.parse_args()
    worst = 0.0
    for path in args.files:
        result = report(path)
        if result is None:
            print(f"SKIP     {path}  (unsupported file type)")
            continue
        total, count, lines = result
        if lines:
            print("\n".join(lines))
        score = 100.0 * count / total if total else 0.0
        worst = max(worst, score)
        print(f"SCORE    {path}  {score:.0f} warnings per 100 sentences ({count} in {total})")
    return 1 if args.fail_over is not None and worst > args.fail_over else 0


if __name__ == "__main__":
    sys.exit(main())
