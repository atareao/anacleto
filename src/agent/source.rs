use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// A typed source of context that can be loaded and refreshed.
///
/// `Source<A>` abstracts over any external context that an agent consumes
/// (files, databases, remote documents, ...). The associated type `A` is the
/// loaded value produced by [`Source::load`] / [`Source::refresh`].
pub trait Source<A> {
    /// Load the source and return its current value.
    fn load(&self) -> Result<A>;

    /// Re-read the source and return its (possibly updated) value.
    fn refresh(&mut self) -> Result<A>;
}

/// A [`Source`] backed by a plain text file on disk.
///
/// `load` reads the file contents as a `String`. `refresh` re-reads the file,
/// so callers can pick up edits made after the initial load.
#[derive(Debug, Clone)]
pub struct FileSource {
    /// Absolute path to the file to read.
    pub path: PathBuf,
}

impl FileSource {
    /// Create a new [`FileSource`] for the given path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl Source<String> for FileSource {
    fn load(&self) -> Result<String> {
        std::fs::read_to_string(&self.path).map_err(|e| {
            Error::Io(std::io::Error::new(
                e.kind(),
                format!("Failed to read source file '{}': {e}", self.path.display()),
            ))
        })
    }

    fn refresh(&mut self) -> Result<String> {
        self.load()
    }
}

/// The set of workspace instruction files that are loaded as initial context.
///
/// These are conventional files that describe how to work in a repository.
/// They are detected by name and injected as System messages when present.
pub const WORKSPACE_INSTRUCTION_FILES: [&str; 3] = ["AGENTS.md", "CLAUDE.md", "CONTEXT.md"];

/// Load the workspace instruction files that exist under `workspace`.
///
/// Returns a list of `(file_name, contents)` pairs for every instruction file
/// found in the workspace root. Files that do not exist are skipped silently.
pub fn load_workspace_instructions(workspace: &Path) -> Vec<(String, String)> {
    let mut loaded = Vec::new();
    for name in WORKSPACE_INSTRUCTION_FILES {
        let path = workspace.join(name);
        if path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                loaded.push((name.to_string(), content));
            }
        }
    }
    loaded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_source_loads_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("note.txt");
        std::fs::write(&path, "hello world").expect("write");
        let source = FileSource::new(&path);
        assert_eq!(source.load().expect("load"), "hello world");
    }

    #[test]
    fn file_source_refresh_picks_up_changes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("note.txt");
        std::fs::write(&path, "v1").expect("write");
        let mut source = FileSource::new(&path);
        assert_eq!(source.load().expect("load"), "v1");
        std::fs::write(&path, "v2").expect("write");
        assert_eq!(source.refresh().expect("refresh"), "v2");
    }

    #[test]
    fn file_source_missing_file_errors() {
        let source = FileSource::new("/nonexistent/anacleto/definitely-missing.txt");
        assert!(source.load().is_err());
    }

    #[test]
    fn load_workspace_instructions_detects_existing_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("AGENTS.md"), "# Agents").expect("write");
        std::fs::write(dir.path().join("CONTEXT.md"), "context").expect("write");
        // CLAUDE.md intentionally absent.

        let loaded = load_workspace_instructions(dir.path());
        let names: Vec<&str> = loaded.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"AGENTS.md"));
        assert!(names.contains(&"CONTEXT.md"));
        assert!(!names.contains(&"CLAUDE.md"));
    }

    #[test]
    fn load_workspace_instructions_empty_when_none_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let loaded = load_workspace_instructions(dir.path());
        assert!(loaded.is_empty());
    }
}
