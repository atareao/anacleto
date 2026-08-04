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
    /// Read or write a file or directory outside the workspace.
    #[serde(rename = "fs.external")]
    FsExternal,
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
            "fs.external" => Ok(Permission::FsExternal),
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
        // fs.external is an opt-in permission: it is never granted by
        // allow-by-default. It must be explicitly listed in `allow`.
        if *permission == Permission::FsExternal {
            return self.allowed.contains(permission);
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

    /// Intersect two permission sets, producing the effective permissions for
    /// a child that must satisfy both its own rules and its parent's.
    ///
    /// Semantics:
    /// - A permission is denied if either set denies it (deny propagates down).
    /// - A permission is explicitly allowed only if both sets allow it.
    /// - Allow-by-default only holds if both sets allow by default.
    pub fn intersection(&self, other: &Permissions) -> Permissions {
        let denied: HashSet<Permission> = self.denied.union(&other.denied).cloned().collect();
        let allowed: HashSet<Permission> =
            self.allowed.intersection(&other.allowed).cloned().collect();
        let allow_by_default = self.allow_by_default && other.allow_by_default;
        Permissions {
            denied,
            allowed,
            allow_by_default,
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
            Permission::FsExternal,
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
        assert_eq!(variants.len(), 8);
        let unique: std::collections::HashSet<&Permission> = variants.iter().collect();
        assert_eq!(unique.len(), 8);
    }

    #[test]
    fn test_intersection_propagates_deny() {
        // Parent denies command.run; child allows everything by default.
        let parent = Permissions::from_config(&PermissionConfig {
            deny: vec!["command.run".into()],
            allow: vec![],
        });
        let child = Permissions::default();
        let effective = parent.intersection(&child);
        // The parent's deny propagates to the child.
        assert!(!effective.is_allowed(&Permission::CommandRun));
        // Other permissions remain allowed by default.
        assert!(effective.is_allowed(&Permission::FsRead));
        assert!(effective.is_allowed(&Permission::NetHttp));
    }

    #[test]
    fn test_intersection_allow_lists_intersect() {
        let parent = Permissions::from_config(&PermissionConfig {
            deny: vec![],
            allow: vec!["fs.read".into(), "skill.use".into()],
        });
        let child = Permissions::from_config(&PermissionConfig {
            deny: vec![],
            allow: vec!["fs.read".into(), "net.http".into()],
        });
        let effective = parent.intersection(&child);
        // Only the intersection of the allow lists is granted.
        assert!(effective.is_allowed(&Permission::FsRead));
        assert!(!effective.is_allowed(&Permission::SkillUse));
        assert!(!effective.is_allowed(&Permission::NetHttp));
        // Deny-by-default holds because neither side allows by default.
        assert!(!effective.is_allowed(&Permission::CommandRun));
    }

    #[test]
    fn test_intersection_deny_overrides_allow() {
        let parent = Permissions::from_config(&PermissionConfig {
            deny: vec!["fs.read".into()],
            allow: vec!["fs.read".into()],
        });
        let child = Permissions::default();
        let effective = parent.intersection(&child);
        assert!(!effective.is_allowed(&Permission::FsRead));
    }

    fn permission_strategy() -> impl Strategy<Value = Permission> {
        prop_oneof![
            Just(Permission::FsRead),
            Just(Permission::FsWrite),
            Just(Permission::FsExternal),
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
