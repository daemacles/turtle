# Session Retrospective — Phase 3 V1 Specs & Model Evaluation Setup

> **10-Second TL;DR**
> - **Validated:** Marked **Phase 3 V1** complete and user-validated in [README.md](README.md) and [LEDGER.md](LEDGER.md).
> - **Created Two-Stage Benchmark:** Separated requirements into [TURTLE_SPEC.md](TURTLE_SPEC.md) (clean-room functional spec) and [TURTLE_P3V1_ABI.md](TURTLE_P3V1_ABI.md) (drop-in host–script ABI conformance spec) to evaluate another LLM in two stages.
> - **Kept Clean:** Verified zero Rust crate/version leaks in both specs and confirmed `cargo test` passes (`13/13`).
> - **Next Up:** Optionally run the external model benchmark using both specs, or start planning **Phase 3 V2 (ECS)**.

---

## 1. Session Narrative: What Happened, What Worked, What Didn't

### How the Work Unfolded

**Arc 1 — Onboarding & Repo Audit.**
You opened by asking to catch up on the `.md` files in the project. Rather than stopping at root-level files, I also read the nested documentation and rule files (`docs/COMMENTING.md`, `.agents/rules/jim-defaults.md`, `.agents/rules/project-instructions.md`) and ran `git log` / `git status` as instructed by [LEDGER.md](LEDGER.md). That quick check surfaced two updates made between sessions: a configured `origin/main` remote and a documentation commit (`e76905c`) on [world.lua](world.lua).

**Arc 2 — From Single Spec Request to Two-Stage Evaluation.**
You confirmed Phase 3 V1 worked and asked for `SPEC.md` (later renamed [TURTLE_SPEC.md](TURTLE_SPEC.md)): a library- and version-agnostic synthesis of Phase 3 V1 requirements to evaluate another LLM, keeping only Rust, embedded Lua, Linux terminal execution, and automated verification non-negotiable. After reading every Rust module and Lua sample world in the repository, I saw four open design choices—most importantly, whether the other model should invent its own Lua API or match our exact scripts. A four-question interview unlocked the **two-stage evaluation** design:
- **Stage 1 ([TURTLE_SPEC.md](TURTLE_SPEC.md)):** Tests independent architecture and UX design from high-level functional goals across all three world archetypes plus a V2 ECS seam.
- **Stage 2 ([TURTLE_P3V1_ABI.md](TURTLE_P3V1_ABI.md)):** Tests whether the model can adapt its engine to run unmodified [world.lua](world.lua), [examples/life.lua](examples/life.lua), and [examples/simple_room.lua](examples/simple_room.lua).

Before proposing a plan, I verified the exact 16 case-sensitive ANSI color strings directly from the local crate source without leaking crate names into either spec, then confirmed with you that [README.md](README.md) and [LEDGER.md](LEDGER.md) should be updated too.

**Arc 3 — Authoring & Verification.**
Once you approved the plan, I created [TURTLE_SPEC.md](TURTLE_SPEC.md) and [TURTLE_P3V1_ABI.md](TURTLE_P3V1_ABI.md), updated [README.md](README.md) and [LEDGER.md](LEDGER.md) (recording Phase 3 V1 validation, `origin/main`, and Session 5), ran `grep` leak checks to ensure zero crate/version/dimension prescriptions slipped in, ran `cargo test` (`13 passed; 0 failed`), and checked `git diff` to confirm only the four intended Markdown files changed.

---

### What Worked Well

- **Stating your goal and negative boundaries upfront.** Explaining *why* you wanted `SPEC.md` (now [TURTLE_SPEC.md](TURTLE_SPEC.md)) and adding `"no specific libraries or versions specified"` immediately prevented implementation quirks (specific crates, versions, or fixed `78×23` dimensions) from contaminating the clean-room spec.
- **Structured interview before drafting.** Asking four targeted multiple-choice questions surfaced the key insight—splitting [TURTLE_SPEC.md](TURTLE_SPEC.md) from [TURTLE_P3V1_ABI.md](TURTLE_P3V1_ABI.md)—without guessing or rewriting large documents afterward.
- **Grounding specs in source code, not memory or summaries.** Reading `src/*.rs`, all three Lua scripts, and the exact color-string parser ensured [TURTLE_P3V1_ABI.md](TURTLE_P3V1_ABI.md) captured real edge cases (playerless `"$cursor"` intents, positional vs. named palette tables, transparent entity backgrounds, contrast substitution, and `"player"` auto-promotion).
- **Plan-before-execute + automated checks.** Sharing the concrete plan caught the open question about updating [README.md](README.md) and [LEDGER.md](LEDGER.md), and the `grep` leak checks plus `cargo test` proved both clean separation and zero regressions.

---

### What Didn't Work as Smoothly

- **Under-specified scope on Turn 1.** Asking to read `"the .md files in this folder to catch yourself up"` left directory depth (root-only vs. recursive) and expected action (summarize vs. edit) unspecified, forcing silent assumptions.
- **Four open forks in the initial spec prompt.** Phrases like `"the requirements for Phase 3 V1"` and `"automatic testing (verification)"` left Lua API strictness, sample-world count, UX specificity, and the automated pass bar unstated, adding an interview round-trip before planning.
- **Unstated file boundary on existing docs.** Marking Phase 3 V1 validated implied [README.md](README.md) and [LEDGER.md](LEDGER.md) might need updates, but that wasn't stated until I asked before executing.
- **Unflagged repo drift between sessions.** [LEDGER.md](LEDGER.md) still said `"no remote yet"` while `origin/main` and commit `e76905c` were already on disk; flagging between-session edits upfront prevents confusion.
- **Trailing constraint conflict in the retrospective request.** Putting `"Don't write any code or make any changes to the filesystem"` at the end of a prompt asking to `"create a RETROSPECTIVE.md"` read like a contradiction until clarified to apply only to Section 3's simulated replies.
- **Minor assistant tool hiccup.** My first search for the color enum scanned the entire Cargo registry and timed out to the background before I narrowed the path directly to the graphics crate.

---

## 2. Terms of Art Glossary

### Group A — Specification & Evaluation

| Term of Art | Plain-English Meaning | How It Appeared in This Session |
|---|---|---|
| **Clean-room specification** *(functional spec)* | A requirements doc describing *what* a system must do without copying or dictating *how* a reference implementation built it. | [TURTLE_SPEC.md](TURTLE_SPEC.md) gives another LLM the functional goals and world archetypes while leaving crate choices, Lua API design, and screen layout open. |
| **Non-functional requirements (NFRs)** *(technical constraints)* | Platform, language, verification, or quality constraints that bound *how* a solution is built rather than what features it has. | Your four non-negotiables: Rust, embedded Lua, Linux terminal execution, and automated verification. |
| **Host–script ABI** *(Application Binary Interface / contract)* | The exact agreed-upon data shapes, callback signatures, naming rules, and runtime conventions between a host program and guest scripts. | [TURTLE_P3V1_ABI.md](TURTLE_P3V1_ABI.md) pins `world.init`, `world.step`, `world.view`, palette entries, and the 16 color strings. |
| **Conformance test** | Checking whether an independent implementation complies with an external standard or ABI using unmodified test inputs. | Stage 2 of your evaluation: seeing if the other model can adapt its engine to run unmodified `world.lua`, `life.lua`, and `simple_room.lua`. |
| **Acceptance criteria / evaluation rubric** | Explicit pass/fail checks and qualitative dimensions used by a human reviewer to score work consistently. | Section 6 of [TURTLE_SPEC.md](TURTLE_SPEC.md) (the Human Validation Checklist) and Section 8 of [TURTLE_P3V1_ABI.md](TURTLE_P3V1_ABI.md). |

### Group B — Architecture, Verification & Prompting

| Term of Art | Plain-English Meaning | How It Appeared in This Session |
|---|---|---|
| **ECS seam** *(decoupling boundary)* | Keeping entity storage isolated behind a clean module interface so internals can upgrade to an Entity Component System without rewriting I/O or scripting. | Documented in [LEDGER.md](LEDGER.md) and carried into [TURTLE_SPEC.md](TURTLE_SPEC.md) §2 as the V2 extensibility note. |
| **Atomic Shadow-VM hot-reload** | Compiling and initializing a modified script in a brand-new candidate Lua VM before replacing the live VM, freezing valid state on error. | Core Phase 3 V1 behavior preserved functionally in [TURTLE_SPEC.md](TURTLE_SPEC.md) and exact-contractually in [TURTLE_P3V1_ABI.md](TURTLE_P3V1_ABI.md). |
| **Non-interactive smoke test** | A bounded run (`--steps N`) that proves the binary boots, steps, renders, and exits `0` without a human or live TTY. | Required automated verification check alongside `cargo test` in both specs. |
| **Leak check** | An automated search (`grep`) confirming forbidden details did not accidentally slip into an output file. | Verified zero crate names, versions, or `78×23` dimensions leaked into [TURTLE_SPEC.md](TURTLE_SPEC.md) (and zero crate names in [TURTLE_P3V1_ABI.md](TURTLE_P3V1_ABI.md)). |
| **Invariant** | A property that must *always* stay true across normal execution, errors, and exits. | Terminal restoration on quit/`Ctrl-C`/EOF/panic, and keeping active state undisturbed when a reloaded script fails. |
| **Vendored dependency** | Bundling a native library's source inside the build so compiling doesn't fail if system headers aren't installed. | Stated generically in [TURTLE_SPEC.md](TURTLE_SPEC.md) §1 without naming a crate. |
| **POSIX raw mode / terminal hygiene** | Disabling line buffering and echo on Linux stdin for instant keypresses while guaranteeing cooked mode and cursor visibility return on exit. | Covered under terminal safety invariants in [TURTLE_SPEC.md](TURTLE_SPEC.md) §3.5. |
| **Worked example** | A complete, step-by-step idealized walkthrough used in learning to show good structure without trial-and-error noise. | Section 3 below replays our session using idealized prompts. |
| **Mocked / simulated output** | Realistic stand-in replies written in prose without running tools—distinct from **hallucination**, which is unintended LLM fabrication of false facts. | Used throughout Section 3 below to simulate the ideal conversation cleanly. |

---

## 3. Ideal Conversation (Worked Example)

> **Note:** All assistant replies in this section are **simulated (mocked) in prose only**—no tools, code writes, or filesystem checks were run for these examples.

### Step 1 — Scoped Onboarding & Repo Audit

**Ideal User Prompt:**
> Read all `.md` files in this repository—including `docs/` and `.agents/rules/`, excluding `target/`—and check `git log` / `git status` as `LEDGER.md` instructs. Note that I added `origin/main` and committed documentation edits to `world.lua` (`e76905c`) since the last ledger entry. Summarize where the project stands and wait for my next task without editing any files.

*Fixes from the real session:* Removes ambiguity about directory depth, flags out-of-band git changes upfront, and sets a clear read-only stop condition.

**Simulated Assistant Reply:**
> Caught up across root and nested `.md` files (`README.md`, `LEDGER.md`, `AGENTS.md`, `CLAUDE.md`, `PROJECT_INSTRUCTIONS.md`, `docs/COMMENTING.md`, `.agents/rules/`) plus `git log`/`git status`.
>
> **Status:** Phases 0–2 are validated; Phase 3 V1 (`world.lua`, `examples/life.lua`, `examples/simple_room.lua`) is built and tested, awaiting your validation before Phase 3 V2. Working tree is clean on `main` tracking `origin/main`, with recent commit `e76905c` updating docs in `world.lua`. Waiting for your next instruction.
>
> **Prompt feedback:**
> - **Worked:** Specifying recursive scope (excluding `target/`), flagging your out-of-band git changes, and stating a read-only output eliminated guesses.
> - **Sharpen:** none.

---

### Step 2 — Complete Two-Stage Spec & Doc-Update Plan Request

**Ideal User Prompt:**
> I've validated Phase 3 V1. Before moving to Phase 3 V2, I want a two-stage evaluation benchmark for another LLM:
> 1. **`TURTLE_SPEC.md` (Stage 1 clean-room functional spec):** Non-negotiables only—Rust, in-process embedded Lua, interactive Linux terminal execution, and automated verification (`cargo test` passing plus a `--steps` non-interactive smoke run returning exit 0). Do not specify any crates, versions, exact Lua API schemas, or fixed screen dimensions. Cover high-level engine UX, all 3 sample world archetypes (*Tidekeep* hybrid archipelago, *Life* tick automaton, *Simple Room* turn puzzle), a V2 ECS decoupling note, and a Human Validation Checklist.
> 2. **`TURTLE_P3V1_ABI.md` (Stage 2 conformance spec):** Crate-agnostic exact ABI and runtime semantics derived from `src/` so the model can adapt its engine to run unmodified `world.lua`, `examples/life.lua`, and `examples/simple_room.lua`.
> 3. **Tracking docs:** Update `README.md` and `LEDGER.md` to mark Phase 3 V1 validated and link both new specs.
>
> Inspect the code as needed, then show me your plan before creating or editing any files.

*Fixes from the real session:* Eliminates the 4-question interview round-trip, specifies the exact pass bar for automated tests, names the Stage 1 vs. Stage 2 split directly, and includes the `README.md`/`LEDGER.md` update boundary upfront.

**Simulated Assistant Reply:**
> I read `src/*.rs`, all three Lua world scripts, and the exact color-name parser in the local cache. Here is my plan before touching any files:
>
> 1. **Create `TURTLE_SPEC.md`** (Stage 1 clean-room spec): Rust + embedded Lua + Linux terminal + automated verification (`cargo test` + non-interactive smoke run), zero crate/version/dimension leaks, V2 ECS modularity note, high-level UX, all 3 world archetypes, and a Human Validation Checklist.
> 2. **Create `TURTLE_P3V1_ABI.md`** (Stage 2 conformance spec): Crate-agnostic exact contract covering `[script.lua] [--steps N]`, isolated VM/`print` capture, `world.*` fields, positional/named style tables and exact 16 ANSI color names, `init`/`step`/`view` shapes, `$cursor` playerless semantics, pacing queues, and atomic shadow-VM reload.
> 3. **Update `README.md` and `LEDGER.md`:** Mark Phase 3 V1 validated (`2026-11-22`), update `origin/main` tracking, link the two specs, and log Session 5.
> 4. **Verify:** Run `grep` leak checks on both specs, run `cargo test`, and inspect `git diff` without committing.
>
> Confirm to proceed.
>
> **Prompt feedback:**
> - **Worked:** Defining Stage 1 vs. Stage 2 upfront, naming the automated verification bar, and explicitly authorizing `README.md`/`LEDGER.md` edits allowed a complete plan in one pass.
> - **Sharpen:** none.

---

### Step 3 — Approval & Verified Delivery

**Ideal User Prompt:**
> Confirmed; proceed with the plan, run the leak checks and `cargo test`, and leave the changes uncommitted.

*Fixes from the real session:* Clear approval with explicit git boundary (`leave uncommitted`).

**Simulated Assistant Reply:**
> Done. Created `TURTLE_SPEC.md` and `TURTLE_P3V1_ABI.md`, and updated `README.md` and `LEDGER.md` to record Phase 3 V1 validation and link both evaluation docs.
>
> **Verification (simulated):**
> - Leak checks: zero crate/version/dimension matches in `TURTLE_SPEC.md`; zero crate matches in `TURTLE_P3V1_ABI.md`.
> - `cargo test`: `13 passed; 0 failed`.
> - `git status`: only `TURTLE_SPEC.md`, `TURTLE_P3V1_ABI.md`, `README.md`, and `LEDGER.md` added/modified; nothing committed.

---

### Reusable 3-Point Prompting Checklist (For Future Tasks)
1. **Scope & recent edits:** Name exact folders/exclusions and mention any commits or file edits you made since the last session.
2. **Goal, boundaries & pass bar:** State what the deliverable is for, what must *stay out* (e.g., no crates/versions), and what passing verification looks like (`cargo test` + `--steps`).
3. **File boundaries & next step:** Say which existing files may change (e.g., `README.md`/`LEDGER.md`) and whether to stop at a plan, summarize read-only, or edit immediately.
