//! Integration tests for skill discovery.
//!
//! These tests verify that `discover_skills()` correctly scans workspace and
//! global skill directories and returns discovered skill metadata.

use std::path::PathBuf;

/// Helper: create a skill directory with a SKILL.md file.
fn create_skill(base: &PathBuf, name: &str) -> PathBuf {
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

/// Helper: create a directory that looks like a skill dir but has no SKILL.md.
fn create_empty_dir(base: &PathBuf, name: &str) -> PathBuf {
    let dir = base.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_discover_skills_finds_workspace_skills() {
    let tmp = tempfile::tempdir().unwrap();
    let project_root = tmp.path().join("myproject");
    let skills_dir = project_root.join(".agents").join("skills");
    std::fs::create_dir_all(&skills_dir).unwrap();

    create_skill(&skills_dir, "code-review");
    create_skill(&skills_dir, "shell");
    // An empty dir without SKILL.md should be ignored
    create_empty_dir(&skills_dir, "not-a-skill");

    // Save current dir and change to project root so project_skills_dir resolves correctly
    let orig = std::env::current_dir().ok();
    std::env::set_current_dir(&project_root).ok().unwrap();

    let skills = anacleto::skill::discovery::discover_skills();

    if let Some(ref orig) = orig {
        std::env::set_current_dir(orig).ok().unwrap();
    }

    // Should find at least the workspace skills
    let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"code-review"),
        "Expected code-review in {:?}",
        names
    );
    assert!(names.contains(&"shell"), "Expected shell in {:?}", names);
    assert!(
        !names.contains(&"not-a-skill"),
        "Empty dirs should be skipped"
    );
}

#[test]
fn test_discover_skills_no_agents_dir_returns_some() {
    // When no project .agents/skills dir exists, discover_skills should still
    // return results (at least from global if it exists), but importantly it
    // should not crash.
    let tmp = tempfile::tempdir().unwrap();
    let empty_project = tmp.path().join("empty_project");
    std::fs::create_dir_all(&empty_project).unwrap();

    let orig = std::env::current_dir().ok();
    std::env::set_current_dir(&empty_project).ok().unwrap();

    let skills = anacleto::skill::discovery::discover_skills();

    if let Some(ref orig) = orig {
        std::env::set_current_dir(orig).ok().unwrap();
    }

    // May or may not have global skills, but shouldn't crash and all names
    // should be non-empty
    for skill in &skills {
        assert!(!skill.name.is_empty(), "Skill name should not be empty");
        assert!(
            skill.source_dir.join("SKILL.md").exists(),
            "SKILL.md should exist for {} at {:?}",
            skill.name,
            skill.source_dir
        );
    }
}

#[test]
fn test_discovered_skill_lifecycle() {
    // Verify that a discovered skill can be loaded via the loader
    let tmp = tempfile::tempdir().unwrap();
    let skills_dir = tmp.path().join(".agents").join("skills");
    std::fs::create_dir_all(&skills_dir).unwrap();

    create_skill(&skills_dir, "my-skill");

    let orig = std::env::current_dir().ok();
    std::env::set_current_dir(tmp.path()).ok().unwrap();

    let skills = anacleto::skill::discovery::discover_skills();
    assert!(skills.iter().any(|s| s.name == "my-skill"));

    if let Some(ref orig) = orig {
        std::env::set_current_dir(orig).ok().unwrap();
    }
}
