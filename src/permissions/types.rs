use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::config::PermissionConfig;

/// A permission action that can be allowed or denied.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission {
    /// Read a file or directory.
    #[serde(rename = "fs.read")]
    FsRead,
    /// Write to a file or directory.
    #[serde(rename = "fs.write")]
    FsWrite,
    /// Make HTTP requests.
    #[serde(rename = "net.http")]
    NetHttp,
    /// Execute a command.
    #[serde(rename = "command.run")]
    CommandRun,
    /// Use an MCP tool.
    #[serde(rename = "mcp.use")]
    McpUse,
    /// Read environment variables.
    #[serde(rename = "env.read")]
    EnvRead,
    /// Use a skill.
    #[serde(rename = "skill.use")]
    SkillUse,
}

impl std::str::FromStr for Permission {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "fs.read" => Ok(Permission::FsRead),
            "fs.write" => Ok(Permission::FsWrite),
            "net.http" => Ok(Permission::NetHttp),
            "command.run" => Ok(Permission::CommandRun),
            "mcp.use" => Ok(Permission::McpUse),
            "env.read" => Ok(Permission::EnvRead),
            "skill.use" => Ok(Permission::SkillUse),
            _ => Err(format!("Unknown permission: {}", s)),
        }
    }
}

/// Permission set for an agent.
#[derive(Debug, Clone)]
pub struct Permissions {
    /// Explicitly denied permissions.
    denied: HashSet<Permission>,
    /// Explicitly allowed permissions (empty = all not denied are allowed).
    allowed: HashSet<Permission>,
    /// Whether to use allow-by-default or deny-by-default.
    allow_by_default: bool,
}

impl Permissions {
    /// Create permissions from config.
    pub fn from_config(config: &PermissionConfig) -> Self {
        let denied: HashSet<Permission> = config
            .deny
            .iter()
            .filter_map(|p| p.parse::<Permission>().ok())
            .collect();

        let allowed: HashSet<Permission> = config
            .allow
            .iter()
            .filter_map(|p| p.parse::<Permission>().ok())
            .collect();

        // If allow list is non-empty, use deny-by-default for those permissions
        let allow_by_default = allowed.is_empty();

        Self {
            denied,
            allowed,
            allow_by_default,
        }
    }

    /// Check if a permission is granted.
    pub fn is_allowed(&self, permission: &Permission) -> bool {
        if self.denied.contains(permission) {
            return false;
        }
        if self.allow_by_default {
            true
        } else {
            self.allowed.contains(permission)
        }
    }

    /// Check if a permission is explicitly denied.
    pub fn is_denied(&self, permission: &Permission) -> bool {
        self.denied.contains(permission)
    }

    /// Require a permission, returning an error if denied.
    pub fn require(&self, permission: &Permission) -> crate::error::Result<()> {
        if self.is_allowed(permission) {
            Ok(())
        } else {
            Err(crate::error::Error::PermissionDenied(format!(
                "{:?}",
                permission
            )))
        }
    }
}

impl Default for Permissions {
    fn default() -> Self {
        Self {
            denied: HashSet::new(),
            allowed: HashSet::new(),
            allow_by_default: true,
        }
    }
}

impl Permission {
    /// Returns all variants of the Permission enum.
    pub fn all_variants() -> &'static [Self] {
        &[
            Permission::FsRead,
            Permission::FsWrite,
            Permission::NetHttp,
            Permission::CommandRun,
            Permission::McpUse,
            Permission::EnvRead,
            Permission::SkillUse,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn test_allow_by_default() {
        let config = PermissionConfig {
            deny: vec!["command.run".into()],
            allow: vec![],
        };
        let perms = Permissions::from_config(&config);
        assert!(perms.is_allowed(&Permission::FsRead));
        assert!(perms.is_allowed(&Permission::NetHttp));
        assert!(!perms.is_allowed(&Permission::CommandRun));
    }

    #[test]
    fn test_deny_by_default() {
        let config = PermissionConfig {
            deny: vec![],
            allow: vec!["fs.read".into(), "skill.use".into()],
        };
        let perms = Permissions::from_config(&config);
        assert!(perms.is_allowed(&Permission::FsRead));
        assert!(perms.is_allowed(&Permission::SkillUse));
        assert!(!perms.is_allowed(&Permission::NetHttp));
        assert!(!perms.is_allowed(&Permission::CommandRun));
    }

    #[test]
    fn test_deny_overrides_allow() {
        let config = PermissionConfig {
            deny: vec!["fs.read".into()],
            allow: vec!["fs.read".into()],
        };
        let perms = Permissions::from_config(&config);
        assert!(!perms.is_allowed(&Permission::FsRead));
    }

    #[test]
    fn test_all_variants_returns_all() {
        let variants = Permission::all_variants();
        assert_eq!(variants.len(), 7);
        let unique: std::collections::HashSet<&Permission> = variants.iter().collect();
        assert_eq!(unique.len(), 7);
    }

    fn permission_strategy() -> impl Strategy<Value = Permission> {
        prop_oneof![
            Just(Permission::FsRead),
            Just(Permission::FsWrite),
            Just(Permission::NetHttp),
            Just(Permission::CommandRun),
            Just(Permission::McpUse),
            Just(Permission::EnvRead),
            Just(Permission::SkillUse),
        ]
    }

    proptest! {
        #[test]
        fn permission_serde_roundtrip(perm in permission_strategy()) {
            let yaml = serde_yaml::to_string(&perm).unwrap();
            let deserialized: Permission = serde_yaml::from_str(&yaml).unwrap();
            prop_assert_eq!(perm, deserialized);
        }

        #[test]
        fn permission_parse_roundtrip(perm in permission_strategy()) {
            let serialized = serde_yaml::to_string(&perm).unwrap();
            let trimmed = serialized.trim();
            let parsed: Permission = trimmed.parse().unwrap();
            prop_assert_eq!(perm, parsed);
        }

        #[test]
        fn random_strings_dont_panic(s in "\\PC*") {
            let result = s.parse::<Permission>();
            // Must not panic — either valid or Err, never panic
            if let Ok(perm) = result {
                let yaml = serde_yaml::to_string(&perm).unwrap();
                let reparsed: Permission = serde_yaml::from_str(&yaml).unwrap();
                prop_assert_eq!(perm, reparsed);
            }
        }
    }
}
