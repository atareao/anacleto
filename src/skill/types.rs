use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::hook::HookActionConfig;

/// A skill definition loaded from a Markdown file with YAML frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// Skill name (from frontmatter).
    pub name: String,

    /// Skill description (from frontmatter).
    pub description: String,

    /// The instruction body (Markdown content after frontmatter).
    pub instructions: String,

    /// Optional metadata.
    #[serde(default)]
    pub metadata: HashMap<String, String>,

    /// Hooks declared in the skill's frontmatter.
    /// Key is the hook point string (e.g. "after_apply"), value is a list of actions.
    #[serde(default)]
    pub hooks: HashMap<String, Vec<HookActionConfig>>,
}

/// Result of executing a skill.
#[derive(Debug, Clone)]
pub struct SkillResult {
    /// The skill that was executed.
    pub skill_name: String,

    /// The output produced by the skill.
    pub output: String,

    /// Whether the skill completed successfully.
    pub success: bool,

    /// Error message if failed.
    pub error: Option<String>,
}

/// Trait for skill execution backends.
#[async_trait::async_trait]
pub trait SkillExecutor: Send + Sync {
    /// Execute a skill with the given input.
    async fn execute(&self, skill: &Skill, input: &str) -> SkillResult;
}
