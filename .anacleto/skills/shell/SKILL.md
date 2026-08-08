---
name: shell
description: Execute shell commands in the workspace environment
metadata:
  version: "1.0"
  category: system
  risk: high
---

# Shell skill

Execute shell commands and scripts within the current workspace environment.
Use this skill when you need to:

- Run build, test, or lint commands (`cargo build`, `npm test`, etc.)
- Inspect the filesystem (`ls`, `stat`, `git status`)
- Run project-specific tooling
- Execute multi-step scripts for automation

## Security constraints

- Commands run with the permissions of the Anacleto process.
- `sudo` commands are denied by default unless explicitly permitted in the agent config.
- Long-running commands may time out based on the configured execution limit.
- Output is captured and returned; interactive commands are not supported.

## Usage

When calling this tool, provide a `task` describing exactly what shell commands to run
and what outcome you expect. The skill will execute the commands and return stdout + stderr.

### Examples

```yaml
task: "Run the test suite: cargo test"
```

```yaml
task: |
  Check the current git status and list untracked files:
    git status --short
    git status
```

## Preferencia de herramientas

Cuando estén disponibles, prefiere las herramientas modernas escritas en Rust sobre sus
equivalentes clásicos de GNU:

- `bat` en lugar de `cat`
- `lsd` en lugar de `ls`
- `fd` en lugar de `find`
- `rg` en lugar de `grep`
- `sd` en lugar de `sed`
- `procs` en lugar de `ps`
- `duf` en lugar de `df`
- `dust` en lugar de `du`
- `tldr` en lugar de `man`
- También: `jq`, `yq`, `fzf`, `hyperfine`, `watchexec`

Las herramientas disponibles y las que faltan se reportan en el contexto de la herramienta
(inventario de herramientas). Si una herramienta moderna no está disponible, usa su
equivalente clásico.

## Output

The raw stdout and stderr from the command execution. If the command exits with a
non-zero status code, the error output is included.