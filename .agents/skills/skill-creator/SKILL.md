---
name: skill-creator
description: |
  Create new skills, modify and improve existing skills, and measure skill performance.
  Use when users want to create a skill from scratch, edit, or optimize an existing skill,
  run evals to test a skill, or benchmark skill performance.
metadata:
  version: "1.0"
  category: development
  risk: low
---

# Skill Creator

A skill for creating new skills and iteratively improving them, adapted from [anthropics/skills](https://github.com/anthropics/skills).

At a high level, the process of creating a skill goes like this:

- Decide what you want the skill to do and roughly how it should do it
- Write a draft of the skill
- Create a few test prompts and run the agent with access to the skill on them
- Help the user evaluate the results both qualitatively and quantitatively
- Rewrite the skill based on feedback from the user's evaluation of the results
- Repeat until you're satisfied
- Expand the test set and try again at larger scale

Your job when using this skill is to figure out where the user is in this process and then jump in and help them progress through these stages.

---

## Communicating with the user

The skill creator is liable to be used by people across a wide range of familiarity with technical jargon. Pay attention to context cues to understand how to phrase your communication.

It's OK to briefly explain terms if you're in doubt, and feel free to clarify terms with a short definition if you're unsure if the user will get it.

---

## Creating a skill

### 1. Capture Intent

Start by understanding the user's intent. If the conversation already contains a workflow the user wants to capture (e.g., they say "turn this into a skill"), extract answers from the conversation history: the tools used, the sequence of steps, corrections the user made, input/output formats observed. Fill gaps with user input and confirm before proceeding.

Clarify these points:

1. What should this skill enable the agent to do?
2. When should this skill trigger? (what user phrases/contexts)
3. What's the expected output format?
4. What tools or external resources does it need?
5. What are the boundaries? (what should it NOT do)
6. What constitutes a success vs. a failure?

### 2. Write a Draft

Based on the captured intent, create a draft skill following the Anacleto skill format:

```markdown
---
name: <skill-name>
description: <brief description>
metadata:
  version: "1.0"
  category: <system|development|research|productivity>
  risk: <low|medium|high>
---

# <Skill Name>

Instructions for the agent...

## Usage

When to use this skill...

## Examples

Example scenarios...
```

Place the skill at `.agents/skills/<name>/SKILL.md`.

### 3. Create Test Prompts

Write 3-5 test prompts that represent real use cases for the skill. These should cover:

- **Happy path**: typical usage scenario
- **Edge case**: unusual but valid input
- **Boundary**: something that should NOT trigger the skill
- **Complex**: multi-step request requiring reasoning

Create these as a separate test file at `.agents/skills/<name>/tests.md`.

### 4. Evaluate

Run the test prompts and evaluate results:

- **Qualitative**: Does the output look right? Is it useful? Is the tone appropriate?
- **Quantitative**: Measure success rate, completion time, or other relevant metrics.

Show results to the user and ask for feedback.

### 5. Iterate

Based on feedback:

- Refine the skill instructions
- Add missing edge cases
- Clarify ambiguous language
- Improve the description for better triggering
- Re-run tests to verify improvements

### 6. Optimize Description

Once the skill is functionally solid, optimize the `description` field in the frontmatter. A good description:

- Starts with a strong action verb
- Mentions the domain explicitly
- Lists key capabilities
- Includes trigger phrases users would naturally say

---

## Modifying an existing skill

### 1. Understand the current state

Read the existing skill file and understand its intent, structure, and current limitations.

### 2. Identify gaps

Compare the skill against user feedback or observed behavior:

- Missing instructions for common edge cases
- Poor triggering (description not matching user intent)
- Outdated information or patterns
- Insufficient examples

### 3. Apply changes

Edit the skill file with targeted improvements. After each change:

- Run existing tests to ensure no regressions
- Add new tests for the fixed scenarios
- Show the diff to the user

---

## Skill structure for Anacleto

Anacleto skills follow this directory convention:

```
.agents/skills/<name>/
  SKILL.md          # Main skill file (required)
  tests.md          # Test prompts (optional)
  scripts/          # Supporting scripts (optional)
```

The frontmatter fields are:

| Field | Description |
|---|---|
| `name` | Unique skill identifier (kebab-case) |
| `description` | Short description for triggering (use | literal for multi-line) |
| `metadata.version` | Skill version (semver) |
| `metadata.category` | One of: system, development, research, productivity |
| `metadata.risk` | One of: low, medium, high |

---

## When NOT to use this skill

- The user wants a quick answer, not a reusable skill
- The requested functionality is too narrow or one-off

---

## Examples

### Example 1: Creating a skill from scratch

**User**: "I want a skill that reviews my git commits and suggests better messages."

**Process**:
1. Capture intent: Git commit message reviewer
2. Draft skill with rules for conventional commits, imperative mood, summary length
3. Create test prompts: good commit, bad commit, merge commit edge case
4. Run evaluations and iterate based on results
5. Register the skill in the agent config

### Example 2: Improving an existing skill

**User**: "The web-research skill keeps searching even when I just want a quick definition."

**Process**:
1. Read current skill
2. Add a "Quick lookup mode" section that limits depth when user asks for a definition
3. Update description to clarify when deep vs shallow search triggers
4. Test both modes