//! Batch file patching for the `apply_patch` tool.
//!
//! The LLM sends a JSON batch of operations (`add` / `update` / `delete`) that
//! are applied to files relative to the agent's workspace. All operations are
//! validated up-front (path traversal is rejected) and applied sequentially
//! while preserving each existing file's encoding (UTF-8 BOM and CRLF vs LF).

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// The kind of a single patch operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PatchOpKind {
    /// Create a new file (creating parent directories as needed).
    Add,
    /// Replace the contents of an existing file.
    Update,
    /// Delete an existing file.
    Delete,
}

/// A single patch operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchOp {
    /// The operation to perform.
    pub op: PatchOpKind,
    /// File path relative to the workspace.
    pub path: String,
    /// New file contents (required for `add`/`update`, ignored for `delete`).
    #[serde(default)]
    pub content: Option<String>,
}

/// A batch of patch operations as sent by the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchBatch {
    /// The operations to apply, in order.
    pub operations: Vec<PatchOp>,
}

/// The detected encoding of an existing file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileEncoding {
    /// Whether the file starts with a UTF-8 BOM (`EF BB BF`).
    pub has_bom: bool,
    /// Whether the file uses CRLF (`\r\n`) line endings.
    pub crlf: bool,
}

/// Parse a patch batch from the raw JSON arguments of a tool call.
///
/// Accepts either an object with an `operations` array or a bare array of
/// operations, for robustness.
pub fn parse_patch_batch(json: &str) -> Result<PatchBatch, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("Invalid patch JSON: {e}"))?;

    let ops = match &value {
        serde_json::Value::Array(arr) => arr.clone(),
        serde_json::Value::Object(map) => map
            .get("operations")
            .and_then(|v| v.as_array())
            .cloned()
            .ok_or_else(|| "Patch batch must contain an 'operations' array".to_string())?,
        _ => {
            return Err(
                "Patch batch must be an object with 'operations' or a bare array".to_string(),
            );
        }
    };

    let operations: Vec<PatchOp> = ops
        .into_iter()
        .map(|v| serde_json::from_value(v).map_err(|e| format!("Invalid patch operation: {e}")))
        .collect::<Result<_, _>>()?;

    if operations.is_empty() {
        return Err("Patch batch contains no operations".to_string());
    }

    Ok(PatchBatch { operations })
}

/// Resolve a workspace-relative path, rejecting any path that escapes the
/// workspace (absolute paths, `..` traversal, or symlink escapes).
pub fn resolve_within_workspace(workspace: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel_path = Path::new(rel);

    if rel_path.is_absolute() {
        return Err(format!("Absolute paths are not allowed: {rel}"));
    }

    for comp in rel_path.components() {
        match comp {
            Component::ParentDir => {
                return Err(format!("Path escapes workspace: {rel}"));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("Path escapes workspace: {rel}"));
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }

    let workspace_canon = workspace
        .canonicalize()
        .map_err(|e| format!("Invalid workspace '{}': {e}", workspace.display()))?;

    let full = workspace_canon.join(rel_path);

    // Canonicalize the nearest existing ancestor of the parent directory to
    // resolve any symlinks or `..` that could otherwise escape the workspace,
    // then verify containment. The immediate parent may not exist yet (e.g. an
    // "add" operation that creates nested directories), so we walk up to the
    // closest existing ancestor before canonicalizing.
    let parent = full.parent().unwrap_or(&workspace_canon);
    let mut probe = parent.to_path_buf();
    while !probe.exists() {
        match probe.parent() {
            Some(p) => probe = p.to_path_buf(),
            None => break,
        }
    }
    let anchor = if probe.exists() {
        probe
            .canonicalize()
            .map_err(|e| format!("Invalid parent path for '{rel}': {e}"))?
    } else {
        workspace_canon.clone()
    };

    if !anchor.starts_with(&workspace_canon) {
        return Err(format!("Path escapes workspace: {rel}"));
    }

    Ok(full)
}

/// Detect the encoding (BOM + line endings) of an existing file's bytes.
pub fn detect_encoding(bytes: &[u8]) -> FileEncoding {
    let has_bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
    let crlf = bytes.windows(2).any(|w| w == b"\r\n");
    FileEncoding { has_bom, crlf }
}

/// Encode new content using a detected file encoding.
///
/// Re-adds the UTF-8 BOM if the original file had one, and converts LF line
/// endings to CRLF if the original file used CRLF. New files use the default
/// encoding (no BOM, LF).
pub fn encode_content(content: &str, enc: &FileEncoding) -> Vec<u8> {
    let mut out = Vec::new();
    if enc.has_bom {
        out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    if enc.crlf {
        // Normalize any existing CRLF first so we never double the `\r`.
        let normalized = content.replace("\r\n", "\n");
        let with_crlf = normalized.replace('\n', "\r\n");
        out.extend_from_slice(with_crlf.as_bytes());
    } else {
        out.extend_from_slice(content.as_bytes());
    }
    out
}

/// Resolve a patch path.
///
/// When `allow_external` is `false`, paths must stay within the workspace
/// (absolute paths and `..` traversal are rejected). When `allow_external` is
/// `true`, absolute paths and traversal are permitted, so the caller must have
/// already verified the `fs.external` permission.
pub fn resolve_patch_path(
    workspace: &Path,
    rel: &str,
    allow_external: bool,
) -> Result<PathBuf, String> {
    if allow_external {
        let p = Path::new(rel);
        if p.is_absolute() {
            Ok(p.to_path_buf())
        } else {
            Ok(workspace.join(p))
        }
    } else {
        resolve_within_workspace(workspace, rel)
    }
}

/// Apply a batch of patch operations to the workspace.
///
/// All paths are validated before any change is made; if any path escapes the
/// workspace the whole batch is rejected without touching the filesystem.
/// When `allow_external` is `true`, paths may escape the workspace (the caller
/// is responsible for having granted the `fs.external` permission).
pub fn apply_patch_batch(
    workspace: &Path,
    batch: &PatchBatch,
    allow_external: bool,
) -> Result<Vec<String>, String> {
    // Validate every path up-front so a bad path aborts the whole batch before
    // any file is modified.
    let mut resolved = Vec::with_capacity(batch.operations.len());
    for op in &batch.operations {
        let full = resolve_patch_path(workspace, &op.path, allow_external)?;
        resolved.push(full);
    }

    let mut results = Vec::with_capacity(batch.operations.len());
    for (op, full) in batch.operations.iter().zip(resolved.iter()) {
        match op.op {
            PatchOpKind::Add => {
                if full.exists() {
                    return Err(format!("Cannot add, file already exists: {}", op.path));
                }
                if let Some(parent) = full.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        format!("Failed to create directories for '{}': {e}", op.path)
                    })?;
                }
                let content = op.content.as_deref().unwrap_or("");
                // New files use the default encoding (no BOM, LF).
                std::fs::write(full, content.as_bytes())
                    .map_err(|e| format!("Failed to write '{}': {e}", op.path))?;
                results.push(format!("Added {}", op.path));
            }
            PatchOpKind::Update => {
                if !full.exists() {
                    return Err(format!("Cannot update, file does not exist: {}", op.path));
                }
                let bytes = std::fs::read(full)
                    .map_err(|e| format!("Failed to read '{}': {e}", op.path))?;
                let enc = detect_encoding(&bytes);
                let content = op.content.as_deref().unwrap_or("");
                let encoded = encode_content(content, &enc);
                std::fs::write(full, encoded)
                    .map_err(|e| format!("Failed to write '{}': {e}", op.path))?;
                results.push(format!("Updated {}", op.path));
            }
            PatchOpKind::Delete => {
                if !full.exists() {
                    return Err(format!("Cannot delete, file does not exist: {}", op.path));
                }
                std::fs::remove_file(full)
                    .map_err(|e| format!("Failed to delete '{}': {e}", op.path))?;
                results.push(format!("Deleted {}", op.path));
            }
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_workspace() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "anacleto_apply_patch_test_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_batch_object() {
        let json = r#"{"operations":[{"op":"add","path":"a.txt","content":"hi"}]}"#;
        let batch = parse_patch_batch(json).unwrap();
        assert_eq!(batch.operations.len(), 1);
        assert_eq!(batch.operations[0].op, PatchOpKind::Add);
        assert_eq!(batch.operations[0].path, "a.txt");
        assert_eq!(batch.operations[0].content.as_deref(), Some("hi"));
    }

    #[test]
    fn parse_batch_bare_array() {
        let json = r#"[{"op":"delete","path":"b.txt"}]"#;
        let batch = parse_patch_batch(json).unwrap();
        assert_eq!(batch.operations[0].op, PatchOpKind::Delete);
    }

    #[test]
    fn parse_batch_invalid_json() {
        assert!(parse_patch_batch("not json").is_err());
    }

    #[test]
    fn parse_batch_empty() {
        assert!(parse_patch_batch(r#"{"operations":[]}"#).is_err());
    }

    #[test]
    fn apply_add_update_delete() {
        let ws = temp_workspace();

        // Add
        let add = parse_patch_batch(
            r#"{"operations":[{"op":"add","path":"dir/nested/f.txt","content":"hello"}]}"#,
        )
        .unwrap();
        let results = apply_patch_batch(&ws, &add, false).unwrap();
        assert_eq!(results, vec!["Added dir/nested/f.txt".to_string()]);
        let file = ws.join("dir/nested/f.txt");
        assert!(file.exists());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "hello");

        // Update
        let upd = parse_patch_batch(
            r#"{"operations":[{"op":"update","path":"dir/nested/f.txt","content":"world"}]}"#,
        )
        .unwrap();
        let results = apply_patch_batch(&ws, &upd, false).unwrap();
        assert_eq!(results, vec!["Updated dir/nested/f.txt".to_string()]);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "world");

        // Delete
        let del =
            parse_patch_batch(r#"{"operations":[{"op":"delete","path":"dir/nested/f.txt"}]}"#)
                .unwrap();
        let results = apply_patch_batch(&ws, &del, false).unwrap();
        assert_eq!(results, vec!["Deleted dir/nested/f.txt".to_string()]);
        assert!(!file.exists());

        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn apply_add_creates_parent_dirs() {
        let ws = temp_workspace();
        let batch =
            parse_patch_batch(r#"{"operations":[{"op":"add","path":"a/b/c.txt","content":"x"}]}"#)
                .unwrap();
        apply_patch_batch(&ws, &batch, false).unwrap();
        assert!(ws.join("a/b/c.txt").exists());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn apply_update_preserves_crlf() {
        let ws = temp_workspace();
        let file = ws.join("crlf.txt");
        std::fs::write(&file, b"line1\r\nline2\r\n").unwrap();

        let batch = parse_patch_batch(
            r#"{"operations":[{"op":"update","path":"crlf.txt","content":"new1\nnew2\n"}]}"#,
        )
        .unwrap();
        apply_patch_batch(&ws, &batch, false).unwrap();

        let bytes = std::fs::read(&file).unwrap();
        assert_eq!(bytes, b"new1\r\nnew2\r\n");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn apply_update_preserves_bom() {
        let ws = temp_workspace();
        let file = ws.join("bom.txt");
        let mut original = vec![0xEF, 0xBB, 0xBF];
        original.extend_from_slice(b"hello");
        std::fs::write(&file, &original).unwrap();

        let batch = parse_patch_batch(
            r#"{"operations":[{"op":"update","path":"bom.txt","content":"world"}]}"#,
        )
        .unwrap();
        apply_patch_batch(&ws, &batch, false).unwrap();

        let bytes = std::fs::read(&file).unwrap();
        assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert_eq!(&bytes[3..], b"world");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn apply_update_preserves_bom_and_crlf() {
        let ws = temp_workspace();
        let file = ws.join("both.txt");
        let mut original = vec![0xEF, 0xBB, 0xBF];
        original.extend_from_slice(b"a\r\nb\r\n");
        std::fs::write(&file, &original).unwrap();

        let batch = parse_patch_batch(
            r#"{"operations":[{"op":"update","path":"both.txt","content":"x\ny\n"}]}"#,
        )
        .unwrap();
        apply_patch_batch(&ws, &batch, false).unwrap();

        let bytes = std::fs::read(&file).unwrap();
        assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert_eq!(&bytes[3..], b"x\r\ny\r\n");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn apply_add_uses_default_encoding() {
        let ws = temp_workspace();
        let batch = parse_patch_batch(
            r#"{"operations":[{"op":"add","path":"new.txt","content":"a\nb\n"}]}"#,
        )
        .unwrap();
        apply_patch_batch(&ws, &batch, false).unwrap();
        let bytes = std::fs::read(ws.join("new.txt")).unwrap();
        assert!(!bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert_eq!(bytes, b"a\nb\n");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn reject_path_traversal() {
        let ws = temp_workspace();
        let batch = parse_patch_batch(
            r#"{"operations":[{"op":"add","path":"../escape.txt","content":"x"}]}"#,
        )
        .unwrap();
        let err = apply_patch_batch(&ws, &batch, false).unwrap_err();
        assert!(err.contains("escapes workspace"));
        // Nothing should have been created.
        assert!(!ws.join("../escape.txt").exists());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn reject_absolute_path() {
        let ws = temp_workspace();
        let batch = parse_patch_batch(
            r#"{"operations":[{"op":"add","path":"/etc/passwd","content":"x"}]}"#,
        )
        .unwrap();
        assert!(apply_patch_batch(&ws, &batch, false).is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn external_path_requires_allow_external() {
        let ws = temp_workspace();
        let outside = std::env::temp_dir().join(format!(
            "anacleto_apply_patch_outside_{}",
            uuid::Uuid::new_v4()
        ));

        // Without allow_external, an absolute external path is rejected.
        let batch = parse_patch_batch(&format!(
            r#"{{"operations":[{{"op":"add","path":"{}","content":"x"}}]}}"#,
            outside.display()
        ))
        .unwrap();
        assert!(apply_patch_batch(&ws, &batch, false).is_err());

        // With allow_external, it is permitted.
        let results = apply_patch_batch(&ws, &batch, true).unwrap();
        assert_eq!(results.len(), 1);
        assert!(outside.exists());

        std::fs::remove_dir_all(&ws).unwrap();
        std::fs::remove_file(&outside).unwrap();
    }

    #[test]
    fn reject_nested_traversal() {
        let ws = temp_workspace();
        let batch = parse_patch_batch(
            r#"{"operations":[{"op":"add","path":"a/../../escape.txt","content":"x"}]}"#,
        )
        .unwrap();
        assert!(apply_patch_batch(&ws, &batch, false).is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn reject_update_missing_file() {
        let ws = temp_workspace();
        let batch = parse_patch_batch(
            r#"{"operations":[{"op":"update","path":"missing.txt","content":"x"}]}"#,
        )
        .unwrap();
        assert!(apply_patch_batch(&ws, &batch, false).is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn reject_add_existing_file() {
        let ws = temp_workspace();
        std::fs::write(ws.join("exists.txt"), "x").unwrap();
        let batch =
            parse_patch_batch(r#"{"operations":[{"op":"add","path":"exists.txt","content":"y"}]}"#)
                .unwrap();
        assert!(apply_patch_batch(&ws, &batch, false).is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[test]
    fn detect_encoding_helpers() {
        assert_eq!(
            detect_encoding(b"plain\n"),
            FileEncoding {
                has_bom: false,
                crlf: false
            }
        );
        assert_eq!(
            detect_encoding(b"a\r\nb"),
            FileEncoding {
                has_bom: false,
                crlf: true
            }
        );
        assert_eq!(
            detect_encoding(&[0xEF, 0xBB, 0xBF, b'a']),
            FileEncoding {
                has_bom: true,
                crlf: false
            }
        );
    }

    #[test]
    fn encode_content_does_not_double_crlf() {
        let enc = FileEncoding {
            has_bom: false,
            crlf: true,
        };
        // Content that already contains CRLF must not be doubled.
        let out = encode_content("a\r\nb\nc", &enc);
        assert_eq!(out, b"a\r\nb\r\nc");
    }
}
