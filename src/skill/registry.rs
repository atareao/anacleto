use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::error::Result;
use crate::skill::loader::load_single_or_dir;
use crate::skill::types::Skill;

/// A central registry of all loaded skills, keyed by name (lowercase).
///
/// Skills are loaded once from their source paths and cached. The registry
/// supports hot-reload via `reload()` and O(1) lookup by name.
pub struct SkillRegistry {
    /// Loaded skills keyed by lowercase name.
    skills: HashMap<String, Skill>,
    /// Source paths for each skill (keyed by lowercase name).
    sources: HashMap<String, PathBuf>,
}

impl SkillRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
            sources: HashMap::new(),
        }
    }

    /// Load skills from a list of paths (files or directories).
    /// Clears any previously loaded skills and sources.
    pub fn load_from_paths(&mut self, paths: &[PathBuf]) -> Result<()> {
        self.skills.clear();
        self.sources.clear();

        for path in paths {
            let loaded = load_single_or_dir(path)?;
            for skill in loaded {
                let key = skill.name.to_lowercase();
                self.skills.insert(key.clone(), skill);
                self.sources.insert(key, path.clone());
            }
        }

        Ok(())
    }

    /// Get a skill by name (case-insensitive).
    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.get(&name.to_lowercase())
    }

    /// Insert a skill into the registry, keyed by lowercase name.
    pub fn insert(&mut self, skill: Skill) {
        let key = skill.name.to_lowercase();
        self.skills.insert(key, skill);
    }

    /// List all loaded skills.
    pub fn list(&self) -> Vec<&Skill> {
        self.skills.values().collect()
    }

    /// Reload all skills from their source paths.
    pub fn reload(&mut self) -> Result<()> {
        let paths: Vec<PathBuf> = self.sources.values().cloned().collect();
        // Deduplicate paths
        let mut unique_paths: Vec<PathBuf> = Vec::new();
        for p in paths {
            if !unique_paths.contains(&p) {
                unique_paths.push(p);
            }
        }
        self.load_from_paths(&unique_paths)
    }

    /// Returns the set of skill names (lowercase) that the registry contains.
    pub fn skill_names(&self) -> HashSet<&str> {
        self.skills.keys().map(|s| s.as_str()).collect()
    }

    /// Check if a skill with the given name exists (case-insensitive).
    pub fn contains(&self, name: &str) -> bool {
        self.skills.contains_key(&name.to_lowercase())
    }

    /// Number of loaded skills.
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// Returns true if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe wrapper type for shared access to a SkillRegistry.
pub type SharedSkillRegistry = Arc<RwLock<SkillRegistry>>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn create_test_skill_file(base: &Path, dir_name: &str, skill_name: &str) -> PathBuf {
        let skill_dir = base.join(dir_name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        let path = skill_dir.join("SKILL.md");
        let content = format!(
            r#"---
name: {}
description: A test skill
---
Instructions for {} here.
"#,
            skill_name, skill_name
        );
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_empty_registry() {
        let reg = SkillRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn test_load_skills_from_paths() {
        let dir = tempfile::tempdir().unwrap();
        create_test_skill_file(dir.path(), "review", "code-review");
        create_test_skill_file(dir.path(), "shell", "shell");

        let mut reg = SkillRegistry::new();
        reg.load_from_paths(&[dir.path().join("review"), dir.path().join("shell")])
            .unwrap();

        assert_eq!(reg.len(), 2);
        assert!(reg.contains("code-review"));
        assert!(reg.contains("shell"));
        assert!(reg.get("code-review").is_some());
        assert!(reg.get("CODE-REVIEW").is_some()); // case-insensitive
    }

    #[test]
    fn test_reload() {
        let dir = tempfile::tempdir().unwrap();
        let review_dir = create_test_skill_file(dir.path(), "review", "code-review");
        // Remove parent from path so we test the dir, not the file
        let _ = review_dir;

        let mut reg = SkillRegistry::new();
        reg.load_from_paths(&[dir.path().join("review")]).unwrap();
        assert_eq!(reg.len(), 1);

        // Modify the skill file on disk
        let skill_path = dir.path().join("review").join("SKILL.md");
        let new_content = r#"---
name: code-review
description: Updated description
---
Updated instructions.
"#;
        std::fs::write(&skill_path, new_content).unwrap();

        reg.reload().unwrap();
        assert_eq!(reg.len(), 1);
        let skill = reg.get("code-review").unwrap();
        assert_eq!(skill.description, "Updated description");
    }

    #[test]
    fn test_list() {
        let dir = tempfile::tempdir().unwrap();
        create_test_skill_file(dir.path(), "review", "code-review");
        create_test_skill_file(dir.path(), "shell", "shell");

        let mut reg = SkillRegistry::new();
        reg.load_from_paths(&[dir.path().join("review"), dir.path().join("shell")])
            .unwrap();

        let names: Vec<&str> = reg.list().iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"code-review"));
        assert!(names.contains(&"shell"));
    }

    #[test]
    fn test_skill_names() {
        let dir = tempfile::tempdir().unwrap();
        create_test_skill_file(dir.path(), "review", "code-review");

        let mut reg = SkillRegistry::new();
        reg.load_from_paths(&[dir.path().join("review")]).unwrap();

        let names = reg.skill_names();
        assert!(names.contains("code-review"));
    }
}
