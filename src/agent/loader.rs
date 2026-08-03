use std::path::{Path, PathBuf};

use crate::agent::types::AgentRole;
use crate::config::types::{AgentConfig, PermissionConfig};
use crate::error::{Error, Result};

/// Parse an agent from a Markdown string with YAML frontmatter.
///
/// Format:
/// ```markdown
/// ---
/// name: root
/// description: Senior engineering agent
/// role: root
/// model: deepseek/deepseek-v4-flash
/// skills:
///   - .anacleto/skills/shell/
/// mcps: []
/// permissions:
///   deny: []
/// subagents:
///   - reviewer
/// ---
///
/// You are **Anacleto**...   ← system prompt (Markdown body)
/// ```
///
/// The Markdown body becomes the agent's `system_prompt`. Returns an error
/// if the file has no frontmatter or is missing the required `name`.
pub fn parse_agent(content: &str, default_max_steps: u32) -> Result<AgentConfig> {
    let content = content.trim();

    // Check for frontmatter delimiters
    if !content.starts_with("---") {
        return Err(Error::Agent(
            "Agent file must start with YAML frontmatter (---)".into(),
        ));
    }

    // Find the closing ---
    let end_frontmatter = content[3..]
        .find("\n---")
        .map(|pos| pos + 3) // +3 for the \n--- we matched
        .ok_or_else(|| Error::Agent("Missing closing --- in frontmatter".into()))?;

    let frontmatter_str = &content[3..end_frontmatter].trim();
    let system_prompt = content[end_frontmatter + 4..].trim().to_string();

    // Parse frontmatter as YAML
    #[derive(serde::Deserialize)]
    struct Frontmatter {
        name: String,
        description: String,
        #[serde(default)]
        role: Option<AgentRole>,
        #[serde(default = "default_model")]
        model: String,
        #[serde(default)]
        skills: Vec<PathBuf>,
        #[serde(default)]
        mcps: Vec<String>,
        #[serde(default)]
        permissions: PermissionConfig,
        #[serde(default)]
        subagents: Vec<String>,
        #[serde(default)]
        max_steps: Option<u32>,
    }

    let frontmatter: Frontmatter = serde_yaml::from_str(frontmatter_str)
        .map_err(|e| Error::Agent(format!("Invalid frontmatter: {e}")))?;

    Ok(AgentConfig {
        name: frontmatter.name,
        description: frontmatter.description,
        role: frontmatter.role.unwrap_or(AgentRole::SubAgent),
        model: frontmatter.model,
        skills: frontmatter.skills,
        mcps: frontmatter.mcps,
        permissions: frontmatter.permissions,
        subagents: frontmatter.subagents,
        system_prompt,
        max_steps: frontmatter.max_steps.unwrap_or(default_max_steps),
    })
}

fn default_model() -> String {
    "claude-sonnet-4-20250514".to_string()
}

/// Load all agents from a directory (non-recursive).
///
/// Files that fail to parse are skipped with a warning on stderr, mirroring
/// `load_skills_from_dir`.
pub fn load_agents_from_dir(dir: &Path, default_max_steps: u32) -> Vec<AgentConfig> {
    let mut agents = Vec::new();

    if !dir.exists() {
        return agents;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!(
                "Warning: Failed to read agent directory {}: {e}",
                dir.display()
            );
            return agents;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "md") {
            match std::fs::read_to_string(&path) {
                Ok(content) => match parse_agent(&content, default_max_steps) {
                    Ok(agent) => agents.push(agent),
                    Err(e) => {
                        eprintln!("Warning: Failed to load agent {}: {e}", path.display());
                    }
                },
                Err(e) => {
                    eprintln!("Warning: Failed to read agent {}: {e}", path.display());
                }
            }
        }
    }

    agents
}

/// Load all agents from the global and project agent directories, merging
/// them by name (project overrides global) and validating that exactly one
/// agent declares `role: root`.
///
/// `default_max_steps` is the fallback for agents that don't declare
/// `max_steps` in their frontmatter (from `config.yaml` → `session.max_steps`).
/// `explicit_root` is an optional project root (derived from the `--config`
/// flag); when `None`, the project root is discovered by walking up from the
/// current working directory.
pub fn load_agents(
    default_max_steps: u32,
    explicit_root: Option<&Path>,
) -> Result<Vec<AgentConfig>> {
    let global = load_agents_from_dir(&global_agents_dir(), default_max_steps);
    let project = load_agents_from_dir(&project_agents_dir(explicit_root), default_max_steps);
    let mut merged = merge_agents(global, project)?;

    // Resolve relative skill paths against the project root so skills load
    // regardless of the process's current working directory (e.g. when the
    // binary is launched from inside `.anacleto/`).
    let root = crate::config::paths::project_root(explicit_root);
    for agent in &mut merged {
        agent.skills = agent
            .skills
            .iter()
            .map(|p| resolve_skill_path(p, &root))
            .collect();
    }

    Ok(merged)
}

/// Resolve a skill path against the project root if it is relative.
/// Absolute paths are returned unchanged. A leading `~` is expanded to the
/// user's home directory before resolution.
fn resolve_skill_path(path: &Path, root: &Path) -> PathBuf {
    let path = crate::config::paths::expand_tilde(path);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

/// Path to global agents directory: ~/.config/anacleto/agents
fn global_agents_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("anacleto")
        .join("agents")
}

/// Path to project agents directory: <project_root>/.anacleto/agents
fn project_agents_dir(explicit_root: Option<&Path>) -> PathBuf {
    crate::config::paths::project_agents_dir(explicit_root)
}

/// Merge global and project agents by name (project overrides global) and
/// validate that exactly one agent declares `role: root`.
fn merge_agents(global: Vec<AgentConfig>, project: Vec<AgentConfig>) -> Result<Vec<AgentConfig>> {
    let mut merged: Vec<AgentConfig> = global;
    for agent in project {
        if let Some(pos) = merged.iter().position(|a| a.name == agent.name) {
            merged[pos] = agent;
        } else {
            merged.push(agent);
        }
    }

    let root_count = merged.iter().filter(|a| a.role == AgentRole::Root).count();
    if root_count == 0 {
        return Err(Error::Agent(
            "No root agent found. Exactly one agent must declare `role: root`.".into(),
        ));
    }
    if root_count > 1 {
        return Err(Error::Agent(format!(
            "Multiple root agents found ({root_count}). Exactly one agent must declare `role: root`."
        )));
    }

    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_frontmatter() -> &'static str {
        r#"---
name: root
description: Senior engineering agent
role: root
model: deepseek/deepseek-v4-flash
skills:
  - .anacleto/skills/shell/
mcps:
  - filesystem
permissions:
  deny:
    - command.run.sudo
subagents:
  - reviewer
---

You are **Anacleto**, a senior engineering agent.
"#
    }

    #[test]
    fn test_parse_agent_full_frontmatter() {
        let agent = parse_agent(full_frontmatter(), 60).unwrap();
        assert_eq!(agent.name, "root");
        assert_eq!(agent.description, "Senior engineering agent");
        assert_eq!(agent.role, AgentRole::Root);
        assert_eq!(agent.model, "deepseek/deepseek-v4-flash");
        assert_eq!(agent.skills, vec![PathBuf::from(".anacleto/skills/shell/")]);
        assert_eq!(agent.mcps, vec!["filesystem".to_string()]);
        assert_eq!(agent.permissions.deny, vec!["command.run.sudo".to_string()]);
        assert_eq!(agent.subagents, vec!["reviewer".to_string()]);
        assert!(agent.system_prompt.contains("senior engineering agent"));
    }

    #[test]
    fn test_parse_agent_no_frontmatter() {
        let result = parse_agent("Just some text without frontmatter", 60);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_agent_missing_name() {
        let content = r#"---
description: No name here
---

Some prompt
"#;
        let result = parse_agent(content, 60);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_agent_role_defaults_to_subagent() {
        let content = r#"---
name: reviewer
description: Reviews code
---

Review the code.
"#;
        let agent = parse_agent(content, 60).unwrap();
        assert_eq!(agent.role, AgentRole::SubAgent);
    }

    #[test]
    fn test_parse_agent_max_steps() {
        // Explicit max_steps in frontmatter overrides the default
        let content = r#"---
name: root
description: Root
role: root
max_steps: 10
---

Prompt
"#;
        let agent = parse_agent(content, 60).unwrap();
        assert_eq!(agent.max_steps, 10);

        // No max_steps in frontmatter -> uses the passed default
        let content = r#"---
name: root
description: Root
role: root
---

Prompt
"#;
        let agent = parse_agent(content, 60).unwrap();
        assert_eq!(agent.max_steps, 60);

        // A different passed default is honored
        let agent = parse_agent(content, 25).unwrap();
        assert_eq!(agent.max_steps, 25);
    }

    #[test]
    fn test_parse_agent_empty_body() {
        let content = r#"---
name: root
description: Root
role: root
---

"#;
        let agent = parse_agent(content, 60).unwrap();
        assert_eq!(agent.role, AgentRole::Root);
        assert!(agent.system_prompt.is_empty());
    }

    #[test]
    fn test_load_agents_from_dir_skips_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let valid = dir.path().join("root.md");
        std::fs::write(&valid, full_frontmatter()).unwrap();
        let invalid = dir.path().join("broken.md");
        std::fs::write(&invalid, "no frontmatter here").unwrap();

        let agents = load_agents_from_dir(dir.path(), 60);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "root");
    }

    #[test]
    fn test_merge_agents_project_overrides_global() {
        let global = vec![AgentConfig {
            name: "root".into(),
            description: "global root".into(),
            role: AgentRole::Root,
            model: "llama2".into(),
            skills: vec![],
            mcps: vec![],
            permissions: PermissionConfig::default(),
            subagents: vec![],
            system_prompt: "global prompt".into(),
            max_steps: 60,
        }];
        let project = vec![AgentConfig {
            name: "root".into(),
            description: "project root".into(),
            role: AgentRole::Root,
            model: "llama3.2".into(),
            skills: vec![],
            mcps: vec![],
            permissions: PermissionConfig::default(),
            subagents: vec!["reviewer".into()],
            system_prompt: "project prompt".into(),
            max_steps: 60,
        }];

        let merged = merge_agents(global, project).unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].model, "llama3.2");
        assert_eq!(merged[0].subagents, vec!["reviewer".to_string()]);
        assert_eq!(merged[0].system_prompt, "project prompt");
    }

    #[test]
    fn test_merge_agents_combines_distinct_names() {
        let global = vec![AgentConfig {
            name: "root".into(),
            description: "root".into(),
            role: AgentRole::Root,
            model: "m".into(),
            skills: vec![],
            mcps: vec![],
            permissions: PermissionConfig::default(),
            subagents: vec![],
            system_prompt: "root prompt".into(),
            max_steps: 60,
        }];
        let project = vec![AgentConfig {
            name: "reviewer".into(),
            description: "reviewer".into(),
            role: AgentRole::SubAgent,
            model: "m".into(),
            skills: vec![],
            mcps: vec![],
            permissions: PermissionConfig::default(),
            subagents: vec![],
            system_prompt: "reviewer prompt".into(),
            max_steps: 60,
        }];

        let merged = merge_agents(global, project).unwrap();
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn test_merge_agents_requires_exactly_one_root() {
        // No root
        let no_root = vec![AgentConfig {
            name: "reviewer".into(),
            description: "reviewer".into(),
            role: AgentRole::SubAgent,
            model: "m".into(),
            skills: vec![],
            mcps: vec![],
            permissions: PermissionConfig::default(),
            subagents: vec![],
            system_prompt: "p".into(),
            max_steps: 60,
        }];
        assert!(merge_agents(no_root, vec![]).is_err());

        // Two roots
        let two_roots = vec![
            AgentConfig {
                name: "a".into(),
                description: "a".into(),
                role: AgentRole::Root,
                model: "m".into(),
                skills: vec![],
                mcps: vec![],
                permissions: PermissionConfig::default(),
                subagents: vec![],
                system_prompt: "p".into(),
                max_steps: 60,
            },
            AgentConfig {
                name: "b".into(),
                description: "b".into(),
                role: AgentRole::Root,
                model: "m".into(),
                skills: vec![],
                mcps: vec![],
                permissions: PermissionConfig::default(),
                subagents: vec![],
                system_prompt: "p".into(),
                max_steps: 60,
            },
        ];
        assert!(merge_agents(two_roots, vec![]).is_err());

        // Exactly one root
        let one_root = vec![AgentConfig {
            name: "root".into(),
            description: "root".into(),
            role: AgentRole::Root,
            model: "m".into(),
            skills: vec![],
            mcps: vec![],
            permissions: PermissionConfig::default(),
            subagents: vec![],
            system_prompt: "p".into(),
            max_steps: 60,
        }];
        assert!(merge_agents(one_root, vec![]).is_ok());
    }

    #[test]
    fn test_resolve_skill_path_relative_and_absolute() {
        let root = Path::new("/proj");
        // Relative paths are joined to the project root.
        assert_eq!(
            resolve_skill_path(Path::new(".anacleto/skills/shell/"), root),
            PathBuf::from("/proj/.anacleto/skills/shell/")
        );
        // Absolute paths are returned unchanged.
        assert_eq!(
            resolve_skill_path(Path::new("/abs/skill"), root),
            PathBuf::from("/abs/skill")
        );
    }
}
