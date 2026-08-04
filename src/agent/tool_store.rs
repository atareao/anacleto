use std::collections::HashMap;
use std::collections::VecDeque;

/// Default maximum number of tool outputs retained in a [`ToolOutputStore`].
pub const DEFAULT_TOOL_STORE_CAPACITY: usize = 100;

/// Marker appended to truncated output, indicating the original length.
pub const TRUNCATION_MARKER: &str = "... (truncado en {N} caracteres)";

/// Truncate `content` to at most `max_chars` characters.
///
/// If the content is already within the limit it is returned unchanged.
/// Otherwise it is cut at `max_chars` and a marker noting the original length
/// is appended so callers know the output was shortened.
pub fn truncate_output(content: &str, max_chars: usize) -> String {
    if content.chars().count() <= max_chars {
        return content.to_string();
    }
    let truncated: String = content.chars().take(max_chars).collect();
    let marker = TRUNCATION_MARKER.replace("{N}", &content.chars().count().to_string());
    format!("{truncated}{marker}")
}

/// An in-memory store of full tool outputs keyed by `tool_call_id`.
///
/// The LLM receives a truncated version of a tool result, but the full output
/// is retained here so it can be re-queried later (e.g. for summarization or
/// follow-up). The store is bounded: when it exceeds [`DEFAULT_TOOL_STORE_CAPACITY`]
/// entries, the oldest entries are evicted (FIFO).
#[derive(Debug, Clone, Default)]
pub struct ToolOutputStore {
    /// tool_call_id -> full output.
    entries: HashMap<String, String>,
    /// Insertion order of tool_call_ids, used for FIFO eviction.
    order: VecDeque<String>,
    /// Maximum number of entries retained.
    capacity: usize,
}

impl ToolOutputStore {
    /// Create an empty store with the default capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_TOOL_STORE_CAPACITY)
    }

    /// Create an empty store with a custom capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            capacity,
        }
    }

    /// Store the full output for a tool call, evicting the oldest entries if
    /// the store is at capacity. Re-inserting an existing id refreshes its
    /// position in the FIFO order.
    pub fn insert(&mut self, tool_call_id: impl Into<String>, output: impl Into<String>) {
        let key = tool_call_id.into();
        let output = output.into();

        if self.entries.contains_key(&key) {
            // Refresh: remove the old position so it is treated as most recent.
            self.order.retain(|k| k != &key);
        }

        self.entries.insert(key.clone(), output);
        self.order.push_back(key);

        // Evict oldest entries beyond capacity.
        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }

    /// Retrieve the full output for a tool call, if present.
    pub fn get(&self, tool_call_id: &str) -> Option<&String> {
        self.entries.get(tool_call_id)
    }

    /// Number of entries currently stored.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Remove all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_output_keeps_short_content() {
        assert_eq!(truncate_output("hello", 100), "hello");
    }

    #[test]
    fn truncate_output_truncates_long_content() {
        let content = "a".repeat(500);
        let out = truncate_output(&content, 100);
        assert_eq!(out.chars().count(), 100 + TRUNCATION_MARKER.len());
        assert!(out.ends_with("... (truncado en 500 caracteres)"));
        assert!(out.starts_with(&"a".repeat(100)));
    }

    #[test]
    fn truncate_output_handles_unicode() {
        // 100 multi-byte chars; must not panic and must respect char boundaries.
        let content = "é".repeat(200);
        let out = truncate_output(&content, 50);
        assert!(out.starts_with(&"é".repeat(50)));
    }

    #[test]
    fn store_insert_and_get() {
        let mut store = ToolOutputStore::new();
        store.insert("call-1", "full output one");
        assert_eq!(
            store.get("call-1").map(|s| s.as_str()),
            Some("full output one")
        );
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn store_evicts_fifo_when_over_capacity() {
        let mut store = ToolOutputStore::with_capacity(2);
        store.insert("a", "1");
        store.insert("b", "2");
        store.insert("c", "3");
        assert_eq!(store.len(), 2);
        assert!(store.get("a").is_none(), "oldest should be evicted");
        assert!(store.get("b").is_some());
        assert!(store.get("c").is_some());
    }

    #[test]
    fn store_refresh_moves_to_most_recent() {
        let mut store = ToolOutputStore::with_capacity(2);
        store.insert("a", "1");
        store.insert("b", "2");
        // Refresh "a" so it becomes most recent; then "b" becomes oldest.
        store.insert("a", "1-updated");
        store.insert("c", "3");
        assert!(store.get("b").is_none(), "b should be evicted");
        assert!(store.get("a").is_some());
        assert!(store.get("c").is_some());
    }

    #[test]
    fn store_clear() {
        let mut store = ToolOutputStore::new();
        store.insert("a", "1");
        store.clear();
        assert!(store.is_empty());
    }
}
