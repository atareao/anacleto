//! Path resolution for Anacleto's project and global configuration.
//!
//! Project paths (`.agents/`) are resolved relative to the *project root*,
//! which is discovered by walking up from the current working directory until
//! a directory containing `.agents/` is found. This makes the binary robust
//! to the directory from which it is invoked — running `anacleto` from a
//! subdirectory of the project works exactly as running it from the root.

use std::path::{Path, PathBuf};

/// Determine the project root directory.
///
/// Walks up from the current working directory looking for a `.agents/`
/// directory. If an explicit project root is provided (e.g. derived from the
/// `--config` flag), that takes precedence. Falls back to the CWD when no
/// `.agents/` directory is found.
pub fn project_root(explicit: Option<&Path>) -> PathBuf {
    if let Some(root) = explicit {
        return root.to_path_buf();
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    walk_up_to_project_root(&cwd)
}

/// Walk up from `start` looking for the nearest ancestor containing a
/// `.agents/` directory. Falls back to `start` itself when none is found.
fn walk_up_to_project_root(start: &Path) -> PathBuf {
    let mut dir = Some(start);
    while let Some(d) = dir {
        if d.join(".agents").is_dir() {
            return d.to_path_buf();
        }
        dir = d.parent();
    }
    start.to_path_buf()
}

/// Path to the project config file: `<project_root>/.agents/config.yaml`.
pub fn project_config_path(explicit_root: Option<&Path>) -> PathBuf {
    project_root(explicit_root)
        .join(".agents")
        .join("config.yaml")
}

/// Path to the project agents directory: `<project_root>/.agents/agents`.
pub fn project_agents_dir(explicit_root: Option<&Path>) -> PathBuf {
    project_root(explicit_root).join(".agents").join("agents")
}

/// Path to the global plugins directory: `~/.config/anacleto/plugins`.
pub fn global_plugins_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("anacleto")
        .join("plugins")
}

/// Path to the global skills directory: `$HOME/.agents/skills`.
pub fn global_skills_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".agents")
        .join("skills")
}

/// Path to the project skills directory: `<project_root>/.agents/skills`.
pub fn project_skills_dir(explicit_root: Option<&Path>) -> PathBuf {
    project_root(explicit_root).join(".agents").join("skills")
}

/// Path to the Anacleto-managed skills directory: `~/.config/anacleto/skills`.
///
/// This is a third skill source, alongside `$HOME/.agents/skills` (global)
/// and `<project_root>/.agents/skills` (project). Skills placed here are
/// managed by Anacleto itself (e.g. installed via the skill manager).
pub fn anacleto_skills_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("anacleto")
        .join("skills")
}

/// Expand a leading `~` (or `~/`) in `path` to the user's home directory.
///
/// - `~` → home directory
/// - `~/foo/bar` → `<home>/foo/bar`
/// - Any other path (absolute, relative, or `~user/...`) is returned unchanged.
///
/// If `dirs::home_dir()` returns `None`, the path is returned unchanged.
pub fn expand_tilde(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if (s == "~" || s.starts_with("~/"))
        && let Some(home) = dirs::home_dir()
    {
        if s == "~" {
            return home;
        }
        return home.join(&s[2..]);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_root_walks_up_to_agents_dir() {
        // Create a temp tree: <tmp>/project/.agents and <tmp>/project/sub/deeper
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("project");
        std::fs::create_dir_all(project.join(".agents")).unwrap();
        let deeper = project.join("sub").join("deeper");
        std::fs::create_dir_all(&deeper).unwrap();

        // Walking up from a nested subdirectory finds the project root.
        assert_eq!(walk_up_to_project_root(&deeper), project);
    }

    #[test]
    fn test_walk_up_falls_back_to_start_when_no_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(walk_up_to_project_root(&nested), nested);
    }

    #[test]
    fn test_project_root_explicit_takes_precedence() {
        let tmp = tempfile::tempdir().unwrap();
        let explicit = tmp.path().join("elsewhere");
        std::fs::create_dir_all(&explicit).unwrap();
        assert_eq!(project_root(Some(&explicit)), explicit);
    }

    #[test]
    fn test_project_config_path_joins_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        std::fs::create_dir_all(root.join(".agents")).unwrap();
        let p = project_config_path(Some(&root));
        assert_eq!(p, root.join(".agents").join("config.yaml"));
    }

    #[test]
    fn test_project_agents_dir_joins_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        std::fs::create_dir_all(root.join(".agents")).unwrap();
        let p = project_agents_dir(Some(&root));
        assert_eq!(p, root.join(".agents").join("agents"));
    }

    #[test]
    fn expand_tilde_home_only() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_tilde(Path::new("~")), home);
    }

    #[test]
    fn expand_tilde_with_subpath() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_tilde(Path::new("~/foo/bar")), home.join("foo/bar"));
    }

    #[test]
    fn expand_tilde_absolute_unchanged() {
        let p = Path::new("/etc/hosts");
        assert_eq!(expand_tilde(p), p);
    }

    #[test]
    fn expand_tilde_relative_unchanged() {
        let p = Path::new("foo/bar");
        assert_eq!(expand_tilde(p), p);
    }

    #[test]
    fn expand_tilde_user_prefix_unchanged() {
        // `~user/...` is not expanded (rare case, not required).
        let p = Path::new("~user/foo");
        assert_eq!(expand_tilde(p), p);
    }

    #[test]
    fn test_project_skills_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        std::fs::create_dir_all(root.join(".agents")).unwrap();
        let p = project_skills_dir(Some(&root));
        assert_eq!(p, root.join(".agents").join("skills"));
    }
}
