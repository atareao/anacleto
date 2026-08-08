---
name: code-review
description: Review code for quality, correctness, and adherence to project standards
metadata:
  version: "1.0"
  category: development
  risk: low
---

# Code Review skill

Analyze source code for potential bugs, style violations, performance issues,
security vulnerabilities, and adherence to project conventions.

## Review dimensions

### Correctness
- Logic errors, off-by-one, missing edge cases
- Error handling: are Result types used properly?
- Are unsafe blocks justified?

### Performance
- Unnecessary allocations or cloning
- Async blockers (calling sync I/O in async context)
- Inefficient algorithms or data structures

### Style & conventions
- Does the code follow the project's coding standards?
- Are naming conventions consistent (snake_case for Rust, camelCase for TypeScript)?
- Is the code readable and well-structured?

### Security
- Input validation
- Command injection risks
- Secrets exposure

## Usage

Pass the code to review as a `task` string describing what files or changes
to examine. Include file paths and relevant context.

### Example

```yaml
task: |
  Review the following Rust module:
  File: src/engine/orchestrator.rs
  
  Focus on:
  - Error handling patterns
  - Proper use of async/await
  - Whether the agent lifecycle is correctly managed
```

## Output

A structured review report with severity levels (🔴 Critical, 🟡 Warning, 🔵 Suggestion)
and a clear verdict: APPROVED, CHANGES_REQUESTED, or REJECTED.