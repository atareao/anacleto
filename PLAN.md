# Skill Loading Fixes — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix two startup errors caused by skill loading: (1) the frontmatter parser crashes on nested YAML values under `metadata`, and (2) the `tool-discovery` skill path referenced in `root.md` does not exist.

**Architecture:** Bug 1 is fixed by changing the deserialization strategy in `src/skill/loader.rs` — parse `metadata` as `serde_yaml::Value` and extract only string-valued entries, keeping the public `Skill.metadata` type (`HashMap<String, String>`) unchanged. Bug 2 is fixed by creating the missing skill directory and `SKILL.md` file modeled after existing skills.

**Tech Stack:** Rust, serde_yaml 0.9 (already a dependency), Markdown + YAML frontmatter

**Global Constraints**

- `Skill.metadata` field type in `src/skill/types.rs` must remain `HashMap<String, String>` — no public API changes
- The `blog-avoid-ai` skill is at `$HOME/.agents/skills/blog-avoid-ai/SKILL.md` (global, not in the repo)
- The `tool-discovery` skill goes in `.agents/skills/tool-discovery/` (project-local, in the repo)
- All existing tests must continue to pass
- `cargo fmt --check && cargo clippy && cargo test` must pass before each commit

---

## File Structure

| File | Responsibility |
|---|---|
| `src/skill/loader.rs` (modify) | Change `Frontmatter.metadata` deserialization from `HashMap<String, String>` to `serde_yaml::Value` with string-only extraction |
| `.agents/skills/tool-discovery/SKILL.md` (create) | New skill file for tool-discovery workflow |

---

### Task 1: Make frontmatter metadata parsing resilient to nested values

**Goal:** The `Frontmatter` struct in `loader.rs` deserializes `metadata` directly as `HashMap<String, String>`, which fails when a skill's frontmatter has nested YAML (e.g. `metadata.openclaw` is a map). Fix by parsing as `serde_yaml::Value` and extracting only string-valued entries.

**Files:**
- Modify: `src/skill/loader.rs:52-63`

**Interfaces:**
- Changes: `Frontmatter.metadata` field type changes from `HashMap<String, String>` to `serde_yaml::Value` (internal struct only)
- Preserves: `Skill.metadata` remains `HashMap<String, String>` — the conversion happens in the `Skill` constructor

- [ ] **Step 1: Change the `Frontmatter` struct's `metadata` field to `serde_yaml::Value`**

  Replace lines 56-57 in `src/skill/loader.rs`:

  ```rust
      #[serde(default)]
      metadata: std::collections::HashMap<String, String>,
  ```

  With:

  ```rust
      #[serde(default)]
      metadata: serde_yaml::Value,
  ```

  This allows `serde_yaml` to deserialize any valid YAML value (string, map, sequence, etc.) without failing.

- [ ] **Step 2: Update the `Skill` construction to extract only string values**

  Replace lines 65-71 in `src/skill/loader.rs`:

  ```rust
      Ok(Skill {
          name: frontmatter.name,
          description: frontmatter.description,
          instructions,
          metadata: frontmatter.metadata,
          hooks: frontmatter.hooks,
      })
  ```

  With:

  ```rust
      let metadata = match frontmatter.metadata {
          serde_yaml::Value::Mapping(map) => {
              let mut result = std::collections::HashMap::new();
              for (key, value) in map {
                  if let (Some(k), serde_yaml::Value::String(v)) =
                      (key.as_str(), &value)
                  {
                      result.insert(k.to_string(), v.clone());
                  }
              }
              result
          }
          _ => std::collections::HashMap::new(),
      };

      Ok(Skill {
          name: frontmatter.name,
          description: frontmatter.description,
          instructions,
          metadata,
          hooks: frontmatter.hooks,
      })
  ```

  This iterates over the YAML mapping and only keeps entries where the value is a string. Nested maps, sequences, numbers, booleans, and nulls are silently skipped.

- [ ] **Step 3: Add a test for nested metadata (the `blog-avoid-ai` case)**

  Add to the `mod tests` block in `src/skill/loader.rs` (after `test_parse_skill_with_metadata` at line 175):

  ```rust
      #[test]
      fn test_parse_skill_with_nested_metadata() {
          let content = r#"---
  name: blog-avoid-ai
  description: Audit and rewrite content
  metadata:
    author: Conor Bronsdon
    tags: writing editing voice quality
    openclaw:
      emoji: "\u270D\uFE0F"
  ---
  Instructions here
  "#;
          let skill = parse_skill(content).unwrap();
          // String values are preserved
          assert_eq!(skill.metadata.get("author").unwrap(), "Conor Bronsdon");
          assert_eq!(skill.metadata.get("tags").unwrap(), "writing editing voice quality");
          // Nested map value is skipped (not a string)
          assert!(skill.metadata.get("openclaw").is_none());
      }
  ```

- [ ] **Step 4: Add a test for non-Mapping metadata (edge case)**

  Add after the nested metadata test:

  ```rust
      #[test]
      fn test_parse_skill_with_scalar_metadata() {
          // metadata as a scalar (not a map) — should not crash, returns empty
          let content = r#"---
  name: test
  description: A test skill
  metadata: just-a-string
  ---
  Body
  "#;
          let skill = parse_skill(content).unwrap();
          assert!(skill.metadata.is_empty());
      }
  ```

- [ ] **Step 5: Verify existing tests still pass**

  Run: `cargo test -p anacleto -- skill::loader::tests --nocapture`
  Expected: All tests PASS, including `test_parse_skill_with_metadata` (flat string values still work)

- [ ] **Step 6: Verify the `blog-avoid-ai` skill loads without error**

  Run: `cargo run` (or a targeted test that loads the global skill)
  Expected: No `Invalid frontmatter` error for `blog-avoid-ai/SKILL.md`

  Alternatively, write a quick one-off test:

  ```rust
  #[test]
  fn test_blog_avoid_ai_loads() {
      let path = home::home_dir()
          .unwrap()
          .join(".agents/skills/blog-avoid-ai/SKILL.md");
      if path.exists() {
          let skill = crate::skill::loader::load_skill(&path).unwrap();
          assert_eq!(skill.name, "blog-avoid-ai");
      }
  }
  ```

  Run: `cargo test -p anacleto -- skill::loader::tests::test_blog_avoid_ai_loads --nocapture`
  Expected: PASS (or skipped if the file doesn't exist on CI)

- [ ] **Step 7: Run full test suite**

  Run: `cargo test -p anacleto`
  Expected: All tests PASS

- [ ] **Step 8: Commit**

  ```bash
  git add src/skill/loader.rs
  git commit -m "fix(skill): resilient frontmatter metadata parsing for nested YAML values"
  ```

---

### Task 2: Create the missing `tool-discovery` skill

**Goal:** The root agent references `.agents/skills/tool-discovery/` in its skill list (line 18 of `.agents/agents/root.md`), but the directory does not exist, producing `Warning: Skill path does not exist: .../tool-discovery/`. Create the missing skill with a proper SKILL.md file.

**Files:**
- Create: `.agents/skills/tool-discovery/SKILL.md`

**Interfaces:**
- Produces: A skill named `tool-discovery` that describes the tool-discovery workflow (audit which skill/MCP/subagent to use for a given task before execution)

- [ ] **Step 1: Create the directory and SKILL.md file**

  Create `.agents/skills/tool-discovery/SKILL.md`:

  ```markdown
  ---
  name: tool-discovery
  description: |
    Audits and recommends which skill, MCP server, or subagent to use for a
    given task. Must be invoked before Execute to ensure the right tool is
    selected for the job. Prevents using generic tools when a specialized
    one exists.
  metadata:
    version: "1.0"
    category: system
    risk: low
  ---

  # Tool Discovery (Anacleto)

  This skill helps you **audit which tool to use** before executing a task.
  It prevents the common mistake of reaching for a generic tool (`shell`,
  `filesystem`) when a specialized skill, MCP, or subagent exists.

  **When to use:** Before every `Execute` step in your workflow. The root
  agent's workflow (step 3) mandates: "Before executing, invoke the
  `tool-discovery` skill to audit which skills, MCPs, and subagents are
  best suited for this task."

  ---

  ## How it works

  Given a task description, tool-discovery evaluates:

  1. **Skills** — Is there a dedicated skill for this domain?
     (e.g. `code-review` for reviewing code, `rust-dev` for Rust development)
  2. **MCP servers** — Does an MCP provide structured access to the needed data?
     (e.g. `codegraph` for code intelligence, `filesystem` for file operations)
  3. **Subagents** — Should this be delegated to a specialist subagent?
     (e.g. `reviewer` for code review, `tech-writer` for technical articles)

  ---

  ## Audit checklist

  For each task, answer these questions in order:

  ### 1. Is there a dedicated skill?

  ```bash
  # List all available skills
  ls .agents/skills/ 2>/dev/null
  ls ~/.agents/skills/ 2>/dev/null

  # Search skill descriptions for keywords
  rg -l "keyword" .agents/skills/*/SKILL.md ~/.agents/skills/*/SKILL.md 2>/dev/null
  ```

  If a dedicated skill exists, **use it**. Do not fall back to `shell` or
  `filesystem` for tasks that have their own skill.

  ### 2. Is there a relevant MCP server?

  Check the agent's configured MCPs (from the agent's frontmatter `mcps` list).
  Each MCP provides structured access to specific data:

  | MCP | Best for |
  |---|---|
  | `codegraph` | Code structure, symbol lookup, impact analysis |
  | `filesystem` | File read/write operations |
  | `context7` | Library documentation lookups |
  | `mis-notas` | Personal notes and knowledge base |

  ### 3. Should a subagent handle this?

  Subagents are specialists. Delegate when:

  - **reviewer** — Code review, quality checks, linting
  - **writer** / **tech-writer** — Documentation, articles, READMEs
  - **rust-dev** — Rust implementation, compilation, testing
  - **python-dev** — Python implementation, testing

  ---

  ## Output format

  After running the audit, produce a recommendation like:

  ```
  ## Tool Discovery Audit

  Task: <brief description>

  Recommended:
  - Skill: <skill-name> — <why this skill>
  - MCP: <mcp-name> — <what data it provides>
  - Subagent: <subagent-name> — <what it handles>

  Rationale: <one-line explanation of why these tools are the right choice>
  ```

  ---

  ## Examples

  | Task | Recommended Skill | Why |
  |---|---|---|
  | Review a PR for correctness | `code-review` | Dedicated code review skill with structured output |
  | Write a Rust function | `rust-dev` | Handles compilation, clippy, and testing |
  | Search for a symbol in code | `codegraph` MCP | Structured code intelligence, not raw grep |
  | Create project documentation | `writer` subagent | Technical writing specialist |
  | Find a skill for a task | `find-skills` | Searches local and remote skill registries |
  | Execute a shell command | `shell` | General-purpose command execution |
  ```

  ---

  ## Important notes

  1. **Must be invoked before Execute.** Do not skip this step — it prevents
     using generic tools when a specialized one exists.

  2. **Not a replacement for thinking.** Use this skill as a structured check,
     not as a substitute for reasoning about the task.

  3. **Skills take priority.** If a dedicated skill exists for the task, it
     should almost always be preferred over a generic tool or direct MCP access.
  ```

- [ ] **Step 2: Verify the path resolves without warning**

  Run: `cargo run` (or `cargo check` followed by inspecting startup output)
  Expected: No `Warning: Skill path does not exist: .../tool-discovery/` message

  The warning originates from `src/skill/loader.rs` line 121 (`load_agent_skills`):
  ```rust
  Err(e) => eprintln!("Warning: {e}"),
  ```
  which is triggered by `load_single_or_dir` at line 108-110:
  ```rust
  Err(Error::Skill(format!("Skill path does not exist: {}", path.display())))
  ```

  After creating the directory with `SKILL.md`, `load_single_or_dir` will find it
  as a directory and call `load_skills_from_dir`, which will find `SKILL.md` and
  load it successfully.

- [ ] **Step 3: Run full test suite**

  Run: `cargo test -p anacleto`
  Expected: All tests PASS

- [ ] **Step 4: Commit**

  ```bash
  git add .agents/skills/tool-discovery/SKILL.md
  git commit -m "fix(skill): create missing tool-discovery skill referenced by root agent"
  ```

---

### Task 3: Verify both original errors are resolved

**Goal:** Confirm that both startup errors are gone by running the full test suite and checking startup output.

- [ ] **Step 1: Run full test suite**

  Run: `cargo test -p anacleto`
  Expected: All tests PASS

- [ ] **Step 2: Run clippy and fmt checks**

  Run: `cargo fmt --check && cargo clippy`
  Expected: Clean output, no warnings or errors

- [ ] **Step 3: Verify `blog-avoid-ai` skill loads without error**

  The `blog-avoid-ai` skill is at `$HOME/.agents/skills/blog-avoid-ai/SKILL.md` (global).
  It has nested YAML under `metadata.openclaw` which previously caused:
  ```
  metadata.openclaw: invalid type: map, expected a string
  ```

  After Task 1, this should load silently. Verify by running the test added in
  Task 1 Step 6, or by running the application and checking stderr for the absence
  of the error message.

- [ ] **Step 4: Verify `tool-discovery` path resolves without warning**

  The warning was:
  ```
  Warning: Skill path does not exist: .../tool-discovery/
  ```

  After Task 2, the directory and `SKILL.md` exist, so `load_single_or_dir` will
  find it as a valid directory and load the skill. Verify by running the application
  and checking stderr for the absence of the warning.

- [ ] **Step 5: Final commit (if any fixes needed)**

  If any of the above steps reveal issues, fix them before the final commit.
  Otherwise, no additional commit is needed — the two commits from Tasks 1 and 2
  are sufficient.

---

## Self-Review Checklist

**1. Spec coverage:**
- ✅ Bug 1: Frontmatter metadata parsing handles nested YAML values — Task 1
- ✅ Bug 1: `Skill.metadata` type remains `HashMap<String, String>` — unchanged in `types.rs`
- ✅ Bug 1: Tests for nested metadata and scalar metadata edge cases — Task 1 Steps 3-4
- ✅ Bug 2: Missing `tool-discovery` skill created — Task 2
- ✅ Bug 2: Skill modeled after existing skills (frontmatter + body) — Task 2 Step 1
- ✅ Verification that both errors are gone — Task 3

**2. Placeholder scan:** All code blocks contain real, compilable Rust code or valid Markdown. No TBDs, TODOs, or placeholders.

**3. Type consistency:** `Frontmatter.metadata` is internal to `loader.rs` — changing it to `serde_yaml::Value` does not affect the public `Skill` type. The conversion logic extracts only string values, matching the `HashMap<String, String>` contract.

---

## Execution Handoff

Plan complete and saved to `PLAN.md`. Two execution options:

**1. Subagent-Driven (recommended)** — Dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** — Execute tasks in this session with checkpoints

**Which approach?**