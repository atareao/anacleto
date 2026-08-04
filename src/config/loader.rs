use std::path::Path;

use crate::config::Config;
use crate::error::{Error, Result};

/// Load and merge configuration from global and project paths.
///
/// `explicit_config` is an optional path to a project config file (from the
/// `--config` CLI flag). When provided, it overrides auto-detection and its
/// parent directory becomes the project root for agent discovery.
pub fn load_config(explicit_config: Option<&Path>) -> Result<Config> {
    let global_path = global_config_path();

    // Determine the project root. If an explicit config was given, its parent
    // directory is the project root; otherwise walk up from the CWD.
    let explicit_root = explicit_config.and_then(|p| p.parent());
    let project_path = match explicit_config {
        Some(p) => p.to_path_buf(),
        None => crate::config::paths::project_config_path(None),
    };

    let mut config = Config::default();

    // Load global config first
    if global_path.exists() {
        let global: Config = load_yaml(&global_path)?;
        merge_configs(&mut config, global);
    }

    // Load project config on top (overrides)
    if project_path.exists() {
        let project: Config = load_yaml(&project_path)?;
        merge_configs(&mut config, project);
    }

    // Agents are defined exclusively as Markdown files with YAML frontmatter.
    // Load them from the global and project agents/ directories (merged by name).
    // `session.max_steps` is the default for agents that don't declare it.
    config.agents = crate::agent::loader::load_agents(config.session.max_steps, explicit_root)?;

    // Expand a leading `~` in the database path to the user's home directory
    // so we don't create a literal `~` directory in the CWD.
    config.session.database_path =
        crate::config::paths::expand_tilde(&config.session.database_path);

    Ok(config)
}

/// Path to global config: ~/.config/anacleto/config.yaml
fn global_config_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("anacleto")
        .join("config.yaml")
}

/// Load a YAML file and deserialize it, expanding `${VAR}` env vars first.
fn load_yaml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| Error::Config(format!("Failed to read {}: {}", path.display(), e)))?;
    let expanded = expand_env_vars(&contents);
    let value: T = serde_yaml::from_str(&expanded)
        .map_err(|e| Error::Config(format!("Failed to parse {}: {}", path.display(), e)))?;
    Ok(value)
}

/// Replace `${VAR_NAME}` patterns with the value of the corresponding environment variable.
/// Leaves unmatched variables as-is (so missing keys don't crash, they'll fail at auth).
fn expand_env_vars(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(dollar) = rest.find("${") {
        result.push_str(&rest[..dollar]);
        rest = &rest[dollar + 2..];
        if let Some(close) = rest.find('}') {
            let var_name = &rest[..close];
            let value = std::env::var(var_name).unwrap_or_else(|_| format!("${{{}}}", var_name));
            result.push_str(&value);
            rest = &rest[close + 1..];
        } else {
            // Unclosed ${ — keep literal
            result.push_str("${");
            break;
        }
    }
    result.push_str(rest);
    result
}

/// Merge `override_config` into `base_config`. Project values take precedence.
pub fn merge_configs(base: &mut Config, override_cfg: Config) {
    // Merge models
    if let Some(anthropic) = override_cfg.models.anthropic {
        base.models.anthropic = Some(anthropic);
    }
    if let Some(openai) = override_cfg.models.openai {
        base.models.openai = Some(openai);
    }
    if let Some(openrouter) = override_cfg.models.openrouter {
        base.models.openrouter = Some(openrouter);
    }
    if let Some(ollama) = override_cfg.models.ollama {
        base.models.ollama = Some(ollama);
    }

    // Merge MCPs
    base.mcps.extend(override_cfg.mcps);

    // Merge session
    if override_cfg.session.history_limit_percent != 50.0 {
        base.session.history_limit_percent = override_cfg.session.history_limit_percent;
    }
    if override_cfg.session.database_path != default_db_path() {
        base.session.database_path = override_cfg.session.database_path;
    }
    if override_cfg.session.max_steps != default_max_steps() {
        base.session.max_steps = override_cfg.session.max_steps;
    }

    // Merge keymap overrides (project wins over global).
    if override_cfg.keymap.is_some() {
        base.keymap = override_cfg.keymap;
    }

    // Merge editor override.
    if override_cfg.editor.is_some() {
        base.editor = override_cfg.editor;
    }

    // Merge model picker favorites.
    if !override_cfg.model_picker.favorites.is_empty() {
        base.model_picker.favorites = override_cfg.model_picker.favorites;
    }

    // NOTE: Agents are NOT merged here. They are defined exclusively as
    // Markdown files and loaded by crate::agent::loader::load_agents().
}

fn default_db_path() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("anacleto")
        .join("sessions.db")
}

fn default_max_steps() -> u32 {
    90
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.agents.is_empty());
        assert!(config.mcps.is_empty());
        assert_eq!(config.session.history_limit_percent, 50.0);
    }

    #[test]
    fn test_parse_minimal_yaml() {
        let yaml = r#"
models:
  ollama:
    base_url: "http://localhost:11434"
    model: "llama3.2"
agents: []
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(config.agents.is_empty());
        assert_eq!(config.models.ollama.unwrap().model, "llama3.2");
    }

    #[test]
    fn test_parse_agent_yaml_ignored() {
        // Agents are no longer defined in YAML. The `agents:` key must be
        // ignored (Config.agents is #[serde(skip)] and populated by the loader).
        let yaml = r#"
agents:
  - name: root
    description: "agents/root.md"
    model: claude-sonnet-4
    skills:
      - "skills/shell/"
    mcps: [filesystem]
    permissions:
      deny: []
    subagents: [reviewer]
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(config.agents.is_empty());
    }

    #[test]
    fn test_expand_env_vars() {
        unsafe { std::env::set_var("ANACLETO_TEST_KEY", "sk-secret-123") };
        assert_eq!(expand_env_vars("${ANACLETO_TEST_KEY}"), "sk-secret-123");
        assert_eq!(
            expand_env_vars("prefix-${ANACLETO_TEST_KEY}-suffix"),
            "prefix-sk-secret-123-suffix"
        );
        // Unmatched vars stay literal
        assert_eq!(expand_env_vars("${MISSING_VAR_XYZ}"), "${MISSING_VAR_XYZ}");
        // Plain text passes through
        assert_eq!(expand_env_vars("no vars here"), "no vars here");
        unsafe { std::env::remove_var("ANACLETO_TEST_KEY") };
    }
}
