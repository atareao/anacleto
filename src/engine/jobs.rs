use std::collections::HashMap;

use tokio::task::JoinHandle;

/// Registry of background jobs (dynamic `task` tool delegations).
///
/// Each job is keyed by its `task_id` and holds the `JoinHandle` of the
/// spawned tokio task so callers can query whether a job is still running.
#[derive(Default)]
pub struct JobRegistry {
    jobs: HashMap<String, JoinHandle<()>>,
}

impl JobRegistry {
    /// Create an empty job registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a background job under the given `task_id`.
    pub fn register(&mut self, task_id: String, handle: JoinHandle<()>) {
        self.jobs.insert(task_id, handle);
    }

    /// Remove and return the handle for a job, if present.
    pub fn remove(&mut self, task_id: &str) -> Option<JoinHandle<()>> {
        self.jobs.remove(task_id)
    }

    /// Whether a job with the given `task_id` is currently registered.
    pub fn is_running(&self, task_id: &str) -> bool {
        self.jobs.contains_key(task_id)
    }

    /// The ids of all currently registered jobs.
    pub fn running_ids(&self) -> Vec<String> {
        self.jobs.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_remove_is_running() {
        let mut reg = JobRegistry::new();
        assert!(!reg.is_running("t1"));

        // A completed task handle is still a valid JoinHandle to store.
        let handle = tokio::spawn(async {});
        reg.register("t1".to_string(), handle);
        assert!(reg.is_running("t1"));
        assert_eq!(reg.running_ids(), vec!["t1".to_string()]);

        let removed = reg.remove("t1");
        assert!(removed.is_some());
        assert!(!reg.is_running("t1"));
        assert!(reg.running_ids().is_empty());
    }

    #[test]
    fn test_remove_missing_returns_none() {
        let mut reg = JobRegistry::new();
        assert!(reg.remove("nope").is_none());
    }
}
