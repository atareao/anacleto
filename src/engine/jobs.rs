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

    /// Whether a job with the given `task_id` is currently running.
    ///
    /// A job is only considered running while its [`JoinHandle`] is not
    /// finished. If the handle has finished (the task completed, panicked, or
    /// was aborted without a successful [`Self::remove`]), it is pruned from
    /// the registry and reported as not running, so a completed job whose
    /// `remove()` failed is never reported as active indefinitely.
    pub fn is_running(&mut self, task_id: &str) -> bool {
        if let Some(handle) = self.jobs.get(task_id) {
            if handle.is_finished() {
                self.jobs.remove(task_id);
                return false;
            }
            return true;
        }
        false
    }

    /// The ids of all currently running jobs.
    ///
    /// Finished handles (whose tasks completed, panicked, or were aborted
    /// without a successful [`Self::remove`]) are pruned from the registry and
    /// excluded from the result.
    pub fn running_ids(&mut self) -> Vec<String> {
        self.jobs.retain(|_, handle| !handle.is_finished());
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

    #[tokio::test]
    async fn test_finished_handle_is_pruned_from_is_running() {
        let mut reg = JobRegistry::new();
        // Spawn a task that completes immediately.
        let handle = tokio::spawn(async {});
        reg.register("done".to_string(), handle);

        // Give the task a moment to finish.
        tokio::task::yield_now().await;

        // Once the handle is finished, it must no longer be reported as
        // running, and it must be pruned from the registry.
        assert!(!reg.is_running("done"));
        assert!(reg.running_ids().is_empty());
    }

    #[tokio::test]
    async fn test_finished_handle_is_pruned_from_running_ids() {
        let mut reg = JobRegistry::new();
        let handle = tokio::spawn(async {});
        reg.register("done".to_string(), handle);

        tokio::task::yield_now().await;

        // running_ids() prunes finished handles and excludes them.
        assert!(reg.running_ids().is_empty());
        // The finished handle is no longer present at all.
        assert!(!reg.is_running("done"));
    }

    #[tokio::test]
    async fn test_panicked_handle_is_pruned() {
        let mut reg = JobRegistry::new();
        // A task that panics still finishes its JoinHandle; it must be pruned
        // so it is not reported as active indefinitely.
        let handle = tokio::spawn(async { panic!("boom") });
        reg.register("crashed".to_string(), handle);

        // Let the panicking task run to completion.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        assert!(!reg.is_running("crashed"));
        assert!(reg.running_ids().is_empty());
    }

    #[tokio::test]
    async fn test_running_handle_is_not_pruned() {
        let mut reg = JobRegistry::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
        let handle = tokio::spawn(async move {
            // Wait until the test signals completion.
            let _ = rx.recv().await;
        });
        reg.register("active".to_string(), handle);

        // The task is still running, so it must be reported as active.
        assert!(reg.is_running("active"));
        assert_eq!(reg.running_ids(), vec!["active".to_string()]);

        // Signal completion and let the task finish.
        let _ = tx.send(()).await;
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        assert!(!reg.is_running("active"));
        assert!(reg.running_ids().is_empty());
    }
}
