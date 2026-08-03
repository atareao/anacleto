//! Atomic structured filesystem operations for the `filesystem` skill.
//!
//! This module provides a small set of well-defined filesystem operations
//! (read, write, edit, list, delete) driven by a JSON task string. It is the
//! backend for the `filesystem` skill, giving the agent structured file access
//! without relying on ad-hoc shell commands.

use std::path::PathBuf;

use serde::Deserialize;

/// The set of filesystem operations the skill can perform.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FsOp {
    /// Read a file's contents to a string.
    Read,
    /// Write `content` to a file, creating parent directories as needed.
    Write,
    /// Replace all occurrences of `old` with `new` in a file.
    Edit,
    /// List the entries of a directory.
    List,
    /// Delete a file.
    Delete,
}

/// A parsed filesystem operation request.
#[derive(Debug, Clone, Deserialize)]
pub struct FsRequest {
    /// The operation to perform.
    pub op: FsOp,
    /// The target file or directory path.
    pub path: PathBuf,
    /// Content to write (used by `Write`).
    pub content: Option<String>,
    /// Text to find (used by `Edit`).
    pub old: Option<String>,
    /// Replacement text (used by `Edit`).
    pub new: Option<String>,
}

/// Parse a task string (a JSON object) into an [`FsRequest`].
///
/// Returns a helpful error message when the task is not valid JSON or does not
/// deserialize into an [`FsRequest`].
pub fn parse_request(task: &str) -> Result<FsRequest, String> {
    serde_json::from_str(task).map_err(|e| {
        format!(
            "Invalid filesystem task. Expected a JSON object like \
             {{\"op\":\"read\",\"path\":\"...\"}}. Parse error: {e}"
        )
    })
}

/// Execute a parsed filesystem request.
///
/// Returns a human-readable confirmation string on success, or an error string
/// on failure.
pub async fn execute(req: FsRequest) -> Result<String, String> {
    match req.op {
        FsOp::Read => execute_read(&req.path).await,
        FsOp::Write => execute_write(&req.path, req.content).await,
        FsOp::Edit => execute_edit(&req.path, req.old, req.new).await,
        FsOp::List => execute_list(&req.path).await,
        FsOp::Delete => execute_delete(&req.path).await,
    }
}

/// Whether this operation modifies the filesystem.
///
/// Returns `true` for `Write`, `Edit`, and `Delete`; `false` for `Read` and
/// `List`.
pub fn is_write_op(op: &FsOp) -> bool {
    matches!(op, FsOp::Write | FsOp::Edit | FsOp::Delete)
}

async fn execute_read(path: &PathBuf) -> Result<String, String> {
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| format!("Failed to read '{}': {e}", path.display()))?;
    Ok(content)
}

async fn execute_write(path: &PathBuf, content: Option<String>) -> Result<String, String> {
    let content = content.ok_or_else(|| {
        format!(
            "Write operation requires a 'content' field for path '{}'",
            path.display()
        )
    })?;
    let len = content.len();

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create parent dirs for '{}': {e}", path.display()))?;
    }

    tokio::fs::write(path, content)
        .await
        .map_err(|e| format!("Failed to write '{}': {e}", path.display()))?;
    Ok(format!("Wrote {len} bytes to '{}'", path.display()))
}

async fn execute_edit(
    path: &PathBuf,
    old: Option<String>,
    new: Option<String>,
) -> Result<String, String> {
    let old = old.ok_or_else(|| {
        format!(
            "Edit operation requires an 'old' field for path '{}'",
            path.display()
        )
    })?;
    let new = new.ok_or_else(|| {
        format!(
            "Edit operation requires a 'new' field for path '{}'",
            path.display()
        )
    })?;

    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| format!("Failed to read '{}': {e}", path.display()))?;

    if !content.contains(&old) {
        return Err(format!(
            "The text to replace was not found in '{}'",
            path.display()
        ));
    }

    let count = content.matches(&old).count();
    let updated = content.replace(&old, &new);

    tokio::fs::write(path, updated)
        .await
        .map_err(|e| format!("Failed to write '{}': {e}", path.display()))?;

    Ok(format!(
        "Replaced {count} occurrence(s) of the old text in '{}'",
        path.display()
    ))
}

async fn execute_list(path: &PathBuf) -> Result<String, String> {
    let mut entries = tokio::fs::read_dir(path)
        .await
        .map_err(|e| format!("Failed to list '{}': {e}", path.display()))?;

    let mut names = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|e| {
        format!(
            "Failed to read directory entry in '{}': {e}",
            path.display()
        )
    })? {
        let file_type = entry
            .file_type()
            .await
            .map_err(|e| format!("Failed to read entry type in '{}': {e}", path.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if file_type.is_dir() {
            names.push(format!("{name}/"));
        } else {
            names.push(name);
        }
    }

    names.sort();
    if names.is_empty() {
        Ok(format!("Directory '{}' is empty", path.display()))
    } else {
        Ok(names.join("\n"))
    }
}

async fn execute_delete(path: &PathBuf) -> Result<String, String> {
    tokio::fs::remove_file(path)
        .await
        .map_err(|e| format!("Failed to delete '{}': {e}", path.display()))?;
    Ok(format!("Deleted '{}'", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Create a unique temp directory for a test and clean it up on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock before epoch")
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "anacleto_fs_test_{}_{}",
                std::process::id(),
                nanos
            ));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            TempDir(dir)
        }

        fn path(&self) -> &PathBuf {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_parse_request_read() {
        let req = parse_request(r#"{"op":"read","path":"/tmp/foo.txt"}"#).unwrap();
        assert_eq!(req.op, FsOp::Read);
        assert_eq!(req.path, PathBuf::from("/tmp/foo.txt"));
        assert!(req.content.is_none());
    }

    #[test]
    fn test_parse_request_write() {
        let req = parse_request(r#"{"op":"write","path":"/tmp/foo.txt","content":"hi"}"#).unwrap();
        assert_eq!(req.op, FsOp::Write);
        assert_eq!(req.content.as_deref(), Some("hi"));
    }

    #[test]
    fn test_parse_request_edit() {
        let req =
            parse_request(r#"{"op":"edit","path":"/tmp/foo.txt","old":"a","new":"b"}"#).unwrap();
        assert_eq!(req.op, FsOp::Edit);
        assert_eq!(req.old.as_deref(), Some("a"));
        assert_eq!(req.new.as_deref(), Some("b"));
    }

    #[test]
    fn test_parse_request_list() {
        let req = parse_request(r#"{"op":"list","path":"/tmp"}"#).unwrap();
        assert_eq!(req.op, FsOp::List);
    }

    #[test]
    fn test_parse_request_delete() {
        let req = parse_request(r#"{"op":"delete","path":"/tmp/foo.txt"}"#).unwrap();
        assert_eq!(req.op, FsOp::Delete);
    }

    #[test]
    fn test_parse_request_invalid_json() {
        let err = parse_request("not json").unwrap_err();
        assert!(err.contains("Invalid filesystem task"));
    }

    #[test]
    fn test_parse_request_missing_op() {
        let err = parse_request(r#"{"path":"/tmp/foo.txt"}"#).unwrap_err();
        assert!(err.contains("Invalid filesystem task"));
    }

    #[tokio::test]
    async fn test_execute_read() {
        let dir = TempDir::new();
        let file = dir.path().join("read.txt");
        std::fs::write(&file, "hello world").unwrap();

        let req = FsRequest {
            op: FsOp::Read,
            path: file.clone(),
            content: None,
            old: None,
            new: None,
        };
        let out = execute(req).await.unwrap();
        assert_eq!(out, "hello world");
    }

    #[tokio::test]
    async fn test_execute_read_not_found() {
        let dir = TempDir::new();
        let req = FsRequest {
            op: FsOp::Read,
            path: dir.path().join("missing.txt"),
            content: None,
            old: None,
            new: None,
        };
        assert!(execute(req).await.is_err());
    }

    #[tokio::test]
    async fn test_execute_write_creates_parent_dirs() {
        let dir = TempDir::new();
        let file = dir.path().join("nested").join("deep").join("file.txt");

        let req = FsRequest {
            op: FsOp::Write,
            path: file.clone(),
            content: Some("content here".into()),
            old: None,
            new: None,
        };
        let out = execute(req).await.unwrap();
        assert!(out.contains("Wrote"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "content here");
    }

    #[tokio::test]
    async fn test_execute_write_missing_content() {
        let dir = TempDir::new();
        let req = FsRequest {
            op: FsOp::Write,
            path: dir.path().join("x.txt"),
            content: None,
            old: None,
            new: None,
        };
        let err = execute(req).await.unwrap_err();
        assert!(err.contains("content"));
    }

    #[tokio::test]
    async fn test_execute_edit_replaces_all_occurrences() {
        let dir = TempDir::new();
        let file = dir.path().join("edit.txt");
        std::fs::write(&file, "foo foo bar foo").unwrap();

        let req = FsRequest {
            op: FsOp::Edit,
            path: file.clone(),
            content: None,
            old: Some("foo".into()),
            new: Some("baz".into()),
        };
        let out = execute(req).await.unwrap();
        assert!(out.contains("3 occurrence(s)"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "baz baz bar baz");
    }

    #[tokio::test]
    async fn test_execute_edit_old_missing() {
        let dir = TempDir::new();
        let file = dir.path().join("edit.txt");
        std::fs::write(&file, "hello").unwrap();

        let req = FsRequest {
            op: FsOp::Edit,
            path: file.clone(),
            content: None,
            old: Some("zzz".into()),
            new: Some("yyy".into()),
        };
        let err = execute(req).await.unwrap_err();
        assert!(err.contains("not found"));
        // File unchanged.
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "hello");
    }

    #[tokio::test]
    async fn test_execute_list() {
        let dir = TempDir::new();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();
        std::fs::write(dir.path().join("b.txt"), "").unwrap();
        std::fs::write(dir.path().join("a.txt"), "").unwrap();

        let req = FsRequest {
            op: FsOp::List,
            path: dir.path().clone(),
            content: None,
            old: None,
            new: None,
        };
        let out = execute(req).await.unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines, vec!["a.txt", "b.txt", "subdir/"]);
    }

    #[tokio::test]
    async fn test_execute_delete() {
        let dir = TempDir::new();
        let file = dir.path().join("del.txt");
        std::fs::write(&file, "x").unwrap();

        let req = FsRequest {
            op: FsOp::Delete,
            path: file.clone(),
            content: None,
            old: None,
            new: None,
        };
        let out = execute(req).await.unwrap();
        assert!(out.contains("Deleted"));
        assert!(!file.exists());
    }

    #[tokio::test]
    async fn test_execute_delete_not_found() {
        let dir = TempDir::new();
        let req = FsRequest {
            op: FsOp::Delete,
            path: dir.path().join("missing.txt"),
            content: None,
            old: None,
            new: None,
        };
        assert!(execute(req).await.is_err());
    }

    #[test]
    fn test_is_write_op() {
        assert!(is_write_op(&FsOp::Write));
        assert!(is_write_op(&FsOp::Edit));
        assert!(is_write_op(&FsOp::Delete));
        assert!(!is_write_op(&FsOp::Read));
        assert!(!is_write_op(&FsOp::List));
    }
}
