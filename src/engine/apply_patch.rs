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
/// `true`, absolute paths and traversal are permitted.
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
/// When `allow_external` is `true`, paths may escape the workspace.
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

/// Build a unified-diff-style text representation of a patch batch, suitable
/// for display in the TUI diff viewer. Because `apply_patch` operates on whole
/// file contents (not line hunks), added/updated files are shown as full
/// additions and deleted files as a removal header.
pub fn batch_to_unified_diff(batch: &PatchBatch) -> String {
    let mut out = String::new();
    for op in &batch.operations {
        out.push_str(&format!("--- a/{}\n", op.path));
        out.push_str(&format!("+++ b/{}\n", op.path));
        match op.op {
            PatchOpKind::Add | PatchOpKind::Update => {
                let content = op.content.as_deref().unwrap_or("");
                let lines: Vec<&str> = content.lines().collect();
                let count = lines.len();
                out.push_str(&format!("@@ -0,0 +1,{} @@\n", count));
                for line in lines {
                    out.push_str(&format!("+{}\n", line));
                }
            }
            PatchOpKind::Delete => {
                out.push_str("@@ -1,0 +0,0 @@\n");
                out.push_str("-<file deleted>\n");
            }
        }
    }
    out
}
