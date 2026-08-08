# Skill.md → SKILL.md Migration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename all `skill.md` files to `SKILL.md` (uppercase) following the Claude Code / OpenCode ecosystem convention, and update all code and documentation references.

**Architecture:** 11 skill files across `.anacleto/skills/<name>/` directories need physical rename. The Rust loader in `src/skill/loader.rs` currently accepts any `.md` file — it must be narrowed to only `SKILL.md`. Three skill files (`find-skills`, `skill-creator`, `agent-creator`) contain internal references to `skill.md` as example paths and commands. The user guide at `docs/user-guide.md` also references the old lowercase paths. All references must be updated to `SKILL.md`.

**Tech Stack:** Rust (cargo), git, bash

---

## File Inventory

### Files to rename (11)

| Old path | New path |
|---|---|
| `.anacleto/skills/agent-creator/skill.md` | `.anacleto/skills/agent-creator/SKILL.md` |
| `.anacleto/skills/code-review/skill.md` | `.anacleto/skills/code-review/SKILL.md` |
| `.anacleto/skills/filesystem/skill.md` | `.anacleto/skills/filesystem/SKILL.md` |
| `.anacleto/skills/find-skills/skill.md` | `.anacleto/skills/find-skills/SKILL.md` |
| `.anacleto/skills/planning/skill.md` | `.anacleto/skills/planning/SKILL.md` |
| `.anacleto/skills/python-best-practices/skill.md` | `.anacleto/skills/python-best-practices/SKILL.md` |
| `.anacleto/skills/rust-dev/skill.md` | `.anacleto/skills/rust-dev/SKILL.md` |
| `.anacleto/skills/shell/skill.md` | `.anacleto/skills/shell/SKILL.md` |
| `.anacleto/skills/skill-creator/skill.md` | `.anacleto/skills/skill-creator/SKILL.md` |
| `.anacleto/skills/version-control/skill.md` | `.anacleto/skills/version-control/SKILL.md` |
| `.anacleto/skills/web-research/skill.md` | `.anacleto/skills/web-research/SKILL.md` |

### Files to modify (5)

| File | Change |
|---|---|
| `src/skill/loader.rs:88` | Filter by `SKILL.md` filename instead of `.md` extension |
| `.anacleto/skills/find-skills/SKILL.md` | 11 references to `skill.md` → `SKILL.md` |
| `.anacleto/skills/skill-creator/SKILL.md` | 2 references to `skill.md` → `SKILL.md` |
| `.anacleto/skills/agent-creator/SKILL.md` | 2 references to `skill.md` → `SKILL.md` |
| `docs/user-guide.md:195-197` | 3 references to `skill.md` → `SKILL.md` |

### Files that need NO changes (verified)

| File | Reason |
|---|---|
| `src/skill/loader.rs:102-113` (`load_single_or_dir`) | Accepts both files and dirs; dirs delegate to `load_skills_from_dir` which will be fixed |
| `src/skill/loader.rs:116-125` (`load_agent_skills`) | Passes paths to `load_single_or_dir` — no direct filename logic |
| All tests in `src/skill/loader.rs:127-229` | `test_installed_ok_skills_load` loads from directories, not filenames; `parse_skill` tests operate on strings, not files |

---

## Tasks

### Task 1: Rename all 11 `skill.md` files to `SKILL.md`

**Files:**
- Rename: 11 files listed in the inventory above

- [ ] **Step 1: Rename all files with a single command**

```bash
for dir in agent-creator code-review filesystem find-skills planning python-best-practices rust-dev shell skill-creator version-control web-research; do
  git mv ".anacleto/skills/$dir/skill.md" ".anacleto/skills/$dir/SKILL.md"
done
```

Expected: no output (git mv is silent on success).

- [ ] **Step 2: Verify the rename**

```bash
ls -la .anacleto/skills/*/SKILL.md
```

Expected: 11 files listed, all named `SKILL.md`. No `skill.md` files remain:

```
.anacleto/skills/agent-creator/SKILL.md
.anacleto/skills/code-review/SKILL.md
.anacleto/skills/filesystem/SKILL.md
.anacleto/skills/find-skills/SKILL.md
.anacleto/skills/planning/SKILL.md
.anacleto/skills/python-best-practices/SKILL.md
.anacleto/skills/rust-dev/SKILL.md
.anacleto/skills/shell/SKILL.md
.anacleto/skills/skill-creator/SKILL.md
.anacleto/skills/version-control/SKILL.md
.anacleto/skills/web-research/SKILL.md
```

- [ ] **Step 3: Verify no lowercase `skill.md` files remain**

```bash
find .anacleto/skills -name 'skill.md'
```

Expected: no output (empty result).

---

### Task 2: Modify `load_skills_from_dir` to filter by `SKILL.md` filename

**Files:**
- Modify: `src/skill/loader.rs:88`

- [ ] **Step 1: Replace the extension check with a filename check**

Change line 88 from:
```rust
        if path.extension().is_some_and(|ext| ext == "md") {
```
to:
```rust
        if path.file_name().is_some_and(|name| name == "SKILL.md") {
```

Use this edit command:
```bash
sed -i 's/if path.extension().is_some_and(|ext| ext == "md")/if path.file_name().is_some_and(|name| name == "SKILL.md")/' src/skill/loader.rs
```

Or manually edit the file.

- [ ] **Step 2: Verify the change**

```bash
grep -n 'SKILL.md' src/skill/loader.rs
```

Expected output (line 88):
```
88:         if path.file_name().is_some_and(|name| name == "SKILL.md") {
```

Also verify the old pattern is gone:
```bash
grep -n 'ext == "md"' src/skill/loader.rs
```

Expected: no output (empty result).

---

### Task 3: Update references in `find-skills/SKILL.md`

**Files:**
- Modify: `.anacleto/skills/find-skills/SKILL.md`

This file has 11 references to `skill.md` that must become `SKILL.md`. They fall into three categories:

**A. Path references in tables and prose (lines 30, 31, 121, 128):**
- Line 30: `` | `.anacleto/skills/<name>/skill.md` | Proyecto | `` → `` | `.anacleto/skills/<name>/SKILL.md` | Proyecto | ``
- Line 31: `` | `~/.config/anacleto/skills/<name>/skill.md` | Global | `` → `` | `~/.config/anacleto/skills/<name>/SKILL.md` | Global | ``
- Line 121: `` 2. Examina su `SKILL.md` (o `skill.md`) para entender qué hace `` → `` 2. Examina su `SKILL.md` para entender qué hace ``
- Line 128: `` # Crear el fichero skill.md con el frontmatter adaptado `` → `` # Crear el fichero SKILL.md con el frontmatter adaptado ``

**B. Command examples with `skill.md` as a filename argument (lines 42, 45, 61, 68, 91, 98, 129):**
- Line 42: `` fd skill.md .anacleto/skills/ --full-path `` → `` fd SKILL.md .anacleto/skills/ --full-path ``
- Line 45: `` fd skill.md ~/.config/anacleto/skills/ --full-path `` → `` fd SKILL.md ~/.config/anacleto/skills/ --full-path ``
- Line 61: `` head -20 .anacleto/skills/<nombre>/skill.md `` → `` head -20 .anacleto/skills/<nombre>/SKILL.md ``
- Line 68: `` for f in .anacleto/skills/*/skill.md; do `` → `` for f in .anacleto/skills/*/SKILL.md; do ``
- Line 91: `` rg -il "testing|test" .anacleto/skills/*/skill.md `` → `` rg -il "testing|test" .anacleto/skills/*/SKILL.md ``
- Line 98: `` rg -il "testing|test" ~/.config/anacleto/skills/*/skill.md 2>/dev/null `` → `` rg -il "testing|test" ~/.config/anacleto/skills/*/SKILL.md 2>/dev/null ``
- Line 129: `` cat > .anacleto/skills/<nombre>/skill.md << 'EOF' `` → `` cat > .anacleto/skills/<nombre>/SKILL.md << 'EOF' ``

- [ ] **Step 1: Replace all `skill.md` with `SKILL.md` in the file**

Use `sed` to replace all occurrences (this handles all 11 references at once):

```bash
sed -i 's/skill\.md/SKILL.md/g' .anacleto/skills/find-skills/SKILL.md
```

- [ ] **Step 2: Verify the replacements**

```bash
grep -n 'SKILL.md' .anacleto/skills/find-skills/SKILL.md | wc -l
```

Expected: 11 (or 12 if the line 121 mention of `SKILL.md` was already uppercase — count should match).

Also verify no lowercase remain:
```bash
grep -n 'skill\.md' .anacleto/skills/find-skills/SKILL.md
```

Expected: no output (empty result).

---

### Task 4: Update references in `skill-creator/SKILL.md`

**Files:**
- Modify: `.anacleto/skills/skill-creator/SKILL.md`

Two references to update:

- Line 81: `` Place the skill at `.anacleto/skills/<name>/skill.md`. `` → `` Place the skill at `.anacleto/skills/<name>/SKILL.md`. ``
- Line 155: ``   skill.md          # Main skill file (required) `` → ``   SKILL.md          # Main skill file (required) ``

- [ ] **Step 1: Replace all `skill.md` with `SKILL.md` in the file**

```bash
sed -i 's/skill\.md/SKILL.md/g' .anacleto/skills/skill-creator/SKILL.md
```

- [ ] **Step 2: Verify the replacements**

```bash
grep -n 'SKILL.md' .anacleto/skills/skill-creator/SKILL.md
```

Expected: two lines showing `SKILL.md`. Verify no lowercase remain:
```bash
grep -n 'skill\.md' .anacleto/skills/skill-creator/SKILL.md
```

Expected: no output.

---

### Task 5: Update references in `agent-creator/SKILL.md`

**Files:**
- Modify: `.anacleto/skills/agent-creator/SKILL.md`

Two references to update:

- Line 267: `` head -5 .anacleto/skills/<name>/skill.md `` → `` head -5 .anacleto/skills/<name>/SKILL.md ``
- Line 535: `` 1. Verify `filesystem` skill exists: `ls .anacleto/skills/filesystem/skill.md` `` → `` 1. Verify `filesystem` skill exists: `ls .anacleto/skills/filesystem/SKILL.md` ``

- [ ] **Step 1: Replace all `skill.md` with `SKILL.md` in the file**

```bash
sed -i 's/skill\.md/SKILL.md/g' .anacleto/skills/agent-creator/SKILL.md
```

- [ ] **Step 2: Verify the replacements**

```bash
grep -n 'SKILL.md' .anacleto/skills/agent-creator/SKILL.md
```

Expected: two lines showing `SKILL.md`. Verify no lowercase remain:
```bash
grep -n 'skill\.md' .anacleto/skills/agent-creator/SKILL.md
```

Expected: no output.

---

### Task 6: Update references in `docs/user-guide.md`

**Files:**
- Modify: `docs/user-guide.md:195-197`

Three table rows to update:

- Line 195: `` | `.anacleto/skills/shell/skill.md` | Execute shell commands | `` → `` | `.anacleto/skills/shell/SKILL.md` | Execute shell commands | ``
- Line 196: `` | `.anacleto/skills/web-research/skill.md` | Fetch and analyze web content | `` → `` | `.anacleto/skills/web-research/SKILL.md` | Fetch and analyze web content | ``
- Line 197: `` | `.anacleto/skills/code-review/skill.md` | Review code for quality and correctness | `` → `` | `.anacleto/skills/code-review/SKILL.md` | Review code for quality and correctness | ``

- [ ] **Step 1: Replace all `skill.md` with `SKILL.md` in the file**

```bash
sed -i 's/skill\.md/SKILL.md/g' docs/user-guide.md
```

- [ ] **Step 2: Verify the replacements**

```bash
grep -n 'SKILL.md' docs/user-guide.md
```

Expected: three lines showing `SKILL.md` in the table. Verify no lowercase remain:
```bash
grep -n 'skill\.md' docs/user-guide.md
```

Expected: no output.

---

### Task 7: Verify with cargo build, test, clippy

- [ ] **Step 1: Run `cargo build`**

```bash
cargo build 2>&1
```

Expected: `Compiling anacleto v0.1.0 ... Finished \`dev\` profile [unoptimized + debuginfo]` — no errors.

- [ ] **Step 2: Run `cargo test`**

```bash
cargo test 2>&1
```

Expected: all tests pass, including `test_installed_ok_skills_load` which loads skills from the renamed directories.

- [ ] **Step 3: Run `cargo clippy`**

```bash
cargo clippy 2>&1
```

Expected: no warnings or errors.

- [ ] **Step 4: Run `cargo fmt --check`**

```bash
cargo fmt --check 2>&1
```

Expected: no formatting issues.

- [ ] **Step 5: Final check — no stale references anywhere**

```bash
# Search for any remaining lowercase skill.md references in the entire repo
# (excluding .git, target/, and this PLAN.md)
rg -g '!.git' -g '!target' -g '!PLAN.md' 'skill\.md' .
```

Expected: no output (empty result — all references have been migrated).

---

## Rollback plan

If something goes wrong, revert all changes with:

```bash
# Revert code changes
git checkout src/skill/loader.rs docs/user-guide.md

# Revert skill file renames (back to lowercase)
for dir in agent-creator code-review filesystem find-skills planning python-best-practices rust-dev shell skill-creator version-control web-research; do
  git mv ".anacleto/skills/$dir/SKILL.md" ".anacleto/skills/$dir/skill.md"
done

# Revert skill content changes
git checkout .anacleto/skills/*/SKILL.md
```