use crate::error::Result;
use crate::permissions::types::{Permission, Permissions};

/// Check if an agent has permission to perform an action.
pub fn check_permission(permissions: &Permissions, action: &Permission) -> Result<()> {
    permissions.require(action)
}

/// Check if an agent has permission to read a file.
pub fn check_fs_read(permissions: &Permissions) -> Result<()> {
    check_permission(permissions, &Permission::FsRead)
}

/// Check if an agent has permission to write a file.
pub fn check_fs_write(permissions: &Permissions) -> Result<()> {
    check_permission(permissions, &Permission::FsWrite)
}

/// Check if an agent has permission to read or write a file or directory
/// outside the workspace.
pub fn check_fs_external(permissions: &Permissions) -> Result<()> {
    check_permission(permissions, &Permission::FsExternal)
}

/// Check if an agent has permission to make HTTP requests.
pub fn check_net_http(permissions: &Permissions) -> Result<()> {
    check_permission(permissions, &Permission::NetHttp)
}

/// Check if an agent has permission to run a command.
pub fn check_command_run(permissions: &Permissions) -> Result<()> {
    check_permission(permissions, &Permission::CommandRun)
}

/// Check if an agent has permission to use an MCP tool.
pub fn check_mcp_use(permissions: &Permissions) -> Result<()> {
    check_permission(permissions, &Permission::McpUse)
}

/// Check if an agent has permission to use a skill.
pub fn check_skill_use(permissions: &Permissions) -> Result<()> {
    check_permission(permissions, &Permission::SkillUse)
}

/// Check if an agent has permission to read environment variables.
pub fn check_env_read(permissions: &Permissions) -> Result<()> {
    check_permission(permissions, &Permission::EnvRead)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PermissionConfig;

    #[test]
    fn test_check_fs_read_allowed() {
        let config = PermissionConfig {
            deny: vec![],
            allow: vec![],
        };
        let perms = Permissions::from_config(&config);
        assert!(check_fs_read(&perms).is_ok());
    }

    #[test]
    fn test_check_command_run_denied() {
        let config = PermissionConfig {
            deny: vec!["command.run".into()],
            allow: vec![],
        };
        let perms = Permissions::from_config(&config);
        assert!(check_command_run(&perms).is_err());
    }

    #[test]
    fn test_check_fs_external_denied_by_default() {
        // Allow-by-default does NOT grant fs.external unless explicitly allowed.
        let config = PermissionConfig {
            deny: vec![],
            allow: vec![],
        };
        let perms = Permissions::from_config(&config);
        assert!(check_fs_external(&perms).is_err());
    }

    #[test]
    fn test_check_fs_external_allowed_when_explicit() {
        let config = PermissionConfig {
            deny: vec![],
            allow: vec!["fs.external".into()],
        };
        let perms = Permissions::from_config(&config);
        assert!(check_fs_external(&perms).is_ok());
    }

    #[test]
    fn test_check_fs_external_denied_when_explicitly_denied() {
        let config = PermissionConfig {
            deny: vec!["fs.external".into()],
            allow: vec!["fs.external".into()],
        };
        let perms = Permissions::from_config(&config);
        assert!(check_fs_external(&perms).is_err());
    }
}
