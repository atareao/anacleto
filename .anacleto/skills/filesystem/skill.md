---
name: filesystem
description: Perform atomic structured filesystem operations (read, write, edit, list, delete)
metadata:
  version: "1.0"
  category: system
  risk: medium
---

# Filesystem skill

Perform structured, atomic filesystem operations on the current workspace. Use this
skill when you need to read, write, edit, list, or delete files without relying on
ad-hoc shell commands.

## Operations

The skill supports five operations, each driven by a JSON object passed as the `task`
argument:

### read

Read a file's contents and return them as a string. Errors if the file does not exist.

```json
{"op":"read","path":"src/main.rs"}
```

### write

Write `content` to a file, creating any missing parent directories automatically.
Requires a `content` field.

```json
{"op":"write","path":"src/foo.rs","content":"fn main() {}"}
```

### edit

Replace **all** occurrences of `old` with `new` in a file. Errors if `old` is not
found. Requires both `old` and `new` fields.

```json
{"op":"edit","path":"src/foo.rs","old":"old_text","new":"new_text"}
```

### list

List the entries of a directory, sorted alphabetically. Directories are suffixed
with a trailing `/`.

```json
{"op":"list","path":"src"}
```

### delete

Delete a file. Errors if the file does not exist.

```json
{"op":"delete","path":"src/foo.rs"}
```

## Rules

- Always provide the `task` argument as a **JSON object string** (as shown above).
- Use `read` before `edit` to confirm the file's current contents.
- `edit` replaces **all** occurrences of `old`, not just the first one.
- `write` creates parent directories automatically; you do not need to create them first.
- `list` returns entry names only (not full paths); directories end with `/`.

## Output

A human-readable confirmation string describing the result of the operation, or the
file contents for `read` / the sorted entry list for `list`.
