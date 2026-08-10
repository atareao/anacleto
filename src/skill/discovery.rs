//! Skill discovery — scan workspace and global directories for SKILL.md files.
//!
//! This module provides a standalone function to discover skills available on
//! disk, regardless of whether they are loaded into the registry. The edit
//! dialog uses this to present all installable skills to the user.

use std::path::PathBuf;

/// Metadata about a discovered skill.
#[derive(Debug, Clone)]
pub struct DiscoveredSkill {
    /// The skill name (derived from the directory name).
    pub name: String,
    /// The directory containing the SKILL.md file.
    pub source_dir: PathBuf,
}

/// Scan workspace and global skill directories for SKILL.md files.
///
/// Returns a list of skills found on disk. Scans two locations:
///
/// 1. `<project_root>/.agents/skills/` — workspace/project skills
/// 2. `$HOME/.agents/skills/` — global (user-wide) skills
///
/// Each subdirectory that contains a `SKILL.md` file is considered a skill.
/// The directory name is used as the skill name.
pub fn discover_skills() -> Vec<DiscoveredSkill> {
    let mut skills = Vec::new();

    // Scan project skills directory
    let project_dir = crate::config::paths::project_skills_dir(None);
    scan_skills_dir(&project_dir, &mut skills);

    // Scan global skills directory
    let global_dir = crate::config::paths::global_skills_dir();
    scan_skills_dir(&global_dir, &mut skills);

    skills
}

/// Scan a single directory for skill subdirectories containing SKILL.md.
fn scan_skills_dir(dir: &PathBuf, skills: &mut Vec<DiscoveredSkill>) {
    if !dir.is_dir() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("SKILL.md").is_file()
                && let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    skills.push(DiscoveredSkill {
                        name: name.to_string(),
                        source_dir: path,
                    });
                }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn create_skill(base: &Path, name: &str) -> PathBuf {
        let dir = base.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let content = format!(
            r#"---
name: {}
description: Test skill {}
---
Instructions for {}.
"#,
            name, name, name
        );
        std::fs::write(dir.join("SKILL.md"), content).unwrap();
        dir
    }

    fn create_empty_dir(base: &Path, name: &str) -> PathBuf {
        let dir = base.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_scan_skills_dir_finds_skills() {
        let tmp = tempfile::tempdir().unwrap();
        create_skill(tmp.path(), "code-review");
        create_skill(tmp.path(), "shell");
        create_empty_dir(tmp.path(), "not-a-skill");

        let mut skills = Vec::new();
        scan_skills_dir(&tmp.path().to_path_buf(), &mut skills);

        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"code-review"));
        assert!(names.contains(&"shell"));
        assert!(!names.contains(&"not-a-skill"));
    }

    #[test]
    fn test_scan_skills_dir_nonexistent_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let bad_dir = tmp.path().join("nonexistent");
        let mut skills = Vec::new();
        scan_skills_dir(&bad_dir, &mut skills);
        assert!(skills.is_empty());
    }

    #[test]
    fn test_discovered_skill_struct() {
        let skill = DiscoveredSkill {
            name: "test".to_string(),
            source_dir: PathBuf::from("/tmp/skills/test"),
        };
        assert_eq!(skill.name, "test");
        assert_eq!(skill.source_dir, PathBuf::from("/tmp/skills/test"));
    }

    #[test]
    fn test_discover_skills_integration() {
        // Create a project with skills and verify discover_skills finds them
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path().join("myproject");
        let skills_dir = project_root.join(".agents").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        create_skill(&skills_dir, "code-review");
        create_skill(&skills_dir, "shell");
        create_empty_dir(&skills_dir, "not-a-skill");

        let orig = std::env::current_dir().ok();
        std::env::set_current_dir(&project_root).ok().unwrap();

        let skills = discover_skills();

        if let Some(ref orig) = orig {
            std::env::set_current_dir(orig).ok().unwrap();
        }

        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"code-review"));
        assert!(names.contains(&"shell"));
    }
}
