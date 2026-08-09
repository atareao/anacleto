use crate::skill::types::{Skill, SkillExecutor, SkillResult};

/// The default skill executor that dispatches to built-in handlers
/// based on the skill name.
pub struct DefaultSkillExecutor;

#[async_trait::async_trait]
impl SkillExecutor for DefaultSkillExecutor {
    async fn execute(&self, skill: &Skill, task: &str) -> SkillResult {
        let skill_name_lower = skill.name.to_lowercase();

        let output = if skill_name_lower == "shell" {
            match execute_shell_command(task).await {
                Ok(result) => {
                    let prompt = crate::shell::inventory().to_prompt();
                    format!("{prompt}\n\n{result}")
                }
                Err(e) => {
                    return SkillResult {
                        skill_name: skill.name.clone(),
                        output: e.clone(),
                        success: false,
                        error: Some(e),
                    };
                }
            }
        } else if skill_name_lower.contains("web") || skill_name_lower.contains("research") {
            match execute_web_fetch(task).await {
                Ok(result) => result,
                Err(e) => {
                    return SkillResult {
                        skill_name: skill.name.clone(),
                        output: e.clone(),
                        success: false,
                        error: Some(e),
                    };
                }
            }
        } else if skill_name_lower == "filesystem" {
            match execute_filesystem_operation(task).await {
                Ok(result) => result,
                Err(e) => {
                    return SkillResult {
                        skill_name: skill.name.clone(),
                        output: e.clone(),
                        success: false,
                        error: Some(e),
                    };
                }
            }
        } else {
            format!(
                r#"📋 Loaded instructions from skill "{}". These are NOT the final result — they tell you HOW to fulfill the request.

Follow the instructions below carefully. You may need to use other tools (like `shell`, `webfetch`, etc.) to actually fetch data or perform actions.

--- Skill instructions for "{}" ---
{}
--- End of skill instructions ---

The original task was: {}"#,
                skill.name, skill.name, skill.instructions, task
            )
        };

        SkillResult {
            skill_name: skill.name.clone(),
            output,
            success: true,
            error: None,
        }
    }
}

/// Execute a shell command from a natural-language task description.
async fn execute_shell_command(task: &str) -> Result<String, String> {
    // Extract command from natural language or use as-is
    let cmd = if task.starts_with('`') && task.ends_with('`') {
        task.trim_matches('`').to_string()
    } else {
        task.to_string()
    };

    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .output()
        .await
        .map_err(|e| format!("Shell execution failed: {e}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

/// Execute a web fetch from a URL description.
async fn execute_web_fetch(task: &str) -> Result<String, String> {
    // Simple URL extraction from task description
    let url = task.trim();
    let response = reqwest::get(url)
        .await
        .map_err(|e| format!("Web fetch failed: {e}"))?;
    let text = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {e}"))?;
    Ok(text)
}

/// Execute a filesystem operation from a task description.
async fn execute_filesystem_operation(task: &str) -> Result<String, String> {
    // Delegate to the filesystem module
    let request: crate::filesystem::FsRequest =
        serde_json::from_str(task).map_err(|e| format!("Invalid filesystem request: {e}"))?;
    crate::filesystem::execute(request)
        .await
        .map_err(|e| format!("Filesystem operation failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::types::Skill;

    fn make_skill(name: &str, instructions: &str) -> Skill {
        Skill {
            name: name.to_string(),
            description: "test".to_string(),
            instructions: instructions.to_string(),
            metadata: std::collections::HashMap::new(),
            hooks: std::collections::HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_execute_fallback_skill() {
        let executor = DefaultSkillExecutor;
        let skill = make_skill("code-review", "Check for correctness.");
        let result = executor.execute(&skill, "Review this code").await;
        assert!(result.success);
        assert!(result.output.contains("Loaded instructions from skill"));
        assert!(result.output.contains("code-review"));
        assert!(result.output.contains("Check for correctness."));
    }

    #[tokio::test]
    async fn test_execute_unknown_skill() {
        let executor = DefaultSkillExecutor;
        let skill = make_skill("custom-tool", "Do something custom.");
        let result = executor.execute(&skill, "Do it").await;
        assert!(result.success);
        assert!(result.output.contains("Loaded instructions from skill"));
        assert!(result.output.contains("custom-tool"));
    }
}
