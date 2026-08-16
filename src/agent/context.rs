use crate::agent::retry::{error_message_is_retriable, retry_with_backoff_filtered};
use crate::agent::tool_store::ToolOutputStore;
use crate::config::types::RetryConfig;
use crate::llm::provider::LlmProvider;
use crate::llm::types::{LlmMessage, LlmRequest, MessageRole};

/// Roughly estimate the number of tokens in a text string.
///
/// Conservative heuristic: ~3 characters per token (instead of the classic
/// 4 chars/token for prose) because agent conversations are heavily code-based
/// (tool outputs, diffs, apply_patch payloads) and code tokenizes denser than
/// prose. Counting characters instead of bytes keeps the estimate stable for
/// UTF-8 input (accented Spanish chars are 2 bytes but 1 char), and the lower
/// divisor deliberately overestimates so compaction fires *before* the real
/// context limit is hit, not after.
pub(crate) fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.chars().count().div_ceil(3)
    }
}

/// Estimate the total tokens for a single message, including tool_calls and tool_call_id.
pub(crate) fn estimate_message_tokens(m: &LlmMessage) -> usize {
    let mut total = estimate_tokens(&m.content);

    // Count tool_calls (e.g. assistant messages with tool invocations)
    if let Some(tool_calls) = &m.tool_calls {
        for tc in tool_calls {
            // Serialize id + type + function name + arguments as one blob
            let serialized = format!(
                "{} {} {} {}",
                tc.id, tc.call_type, tc.function.name, tc.function.arguments
            );
            total += estimate_tokens(&serialized);
        }
    }

    // Count tool_call_id (e.g. tool result messages)
    if let Some(tool_call_id) = &m.tool_call_id {
        total += estimate_tokens(tool_call_id);
    }

    total
}

/// Estimate the total number of tokens across a conversation buffer.
pub(crate) fn conversation_tokens(messages: &[LlmMessage]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

/// Fraction of the token budget at which compaction is triggered automatically.
///
/// When the conversation's estimated token count exceeds
/// `max_tokens * COMPACTION_THRESHOLD_RATIO`, compaction runs even without an
/// explicit `/compact` command.
pub const COMPACTION_THRESHOLD_RATIO: f64 = 0.8;

/// Whether the conversation should be compacted based on the token threshold.
///
/// Returns `true` when `total_tokens` exceeds `max_tokens * ratio`. A `ratio`
/// of `1.0` (or greater) effectively disables threshold-based compaction.
pub fn should_compact(total_tokens: usize, max_tokens: usize, ratio: f64) -> bool {
    if max_tokens == 0 {
        return false;
    }
    let threshold = (max_tokens as f64 * ratio) as usize;
    total_tokens > threshold
}

/// The anchored marker prepended to a structured summary injected as a System
/// message after compaction.
pub const SUMMARY_ANCHOR_MARKER: &str = "[Resumen anclado de conversación anterior]";

/// The structured summary template used to guide the LLM during compaction.
///
/// The LLM is asked to fill only these sections, in order, producing an
/// anchored, structured summary rather than a free-form blob of text.
pub const SUMMARY_TEMPLATE: &str = "\
Resumen estructurado de la conversación. Rellena SOLO estas secciones, en este orden, sin añadir nada más:
## Objetivo
## Decisiones tomadas
## Hechos y contexto clave
## Código/patrones relevantes
## Tareas pendientes
## Riesgos o bloqueos";

/// Build the summarization prompt for a set of messages to summarize.
///
/// The prompt instructs the LLM to produce a structured, anchored summary
/// following [`SUMMARY_TEMPLATE`], preserving key facts, decisions, code
/// patterns and context.
///
/// When `tool_store` is provided, Tool messages whose `tool_call_id` is present
/// in the store use the FULL tool output (retained by the [`ToolOutputStore`])
/// instead of the truncated version the LLM originally received, so the summary
/// is built from complete information rather than the shortened context.
fn build_summary_prompt(messages: &[LlmMessage], tool_store: Option<&ToolOutputStore>) -> String {
    let summary_text: String = messages
        .iter()
        .map(|m| {
            let role_label = match m.role {
                MessageRole::User => "User",
                MessageRole::Assistant => "Assistant",
                MessageRole::Tool => "Tool",
                MessageRole::System => "System",
            };
            // For Tool messages, prefer the full output retained in the store
            // over the truncated version the LLM originally received.
            let content = if m.role == MessageRole::Tool {
                if let (Some(id), Some(store)) = (&m.tool_call_id, tool_store) {
                    store.get(id).map(|s| s.as_str()).unwrap_or(&m.content)
                } else {
                    &m.content
                }
            } else {
                &m.content
            };
            format!("{}: {}", role_label, content)
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    format!("{SUMMARY_TEMPLATE}\n\nConversación a resumir:\n\n{summary_text}")
}

/// Trim conversation history to fit within a token budget.
///
/// Always preserves:
/// - System messages (role == System)
/// - The most recent non-system messages (at least the last exchange)
///
/// Removes oldest non-system messages first when over budget.
///
/// Returns `true` if any message was removed (the conversation changed).
fn trim_conversation(messages: &mut Vec<LlmMessage>, max_tokens: usize) -> bool {
    if messages.is_empty() || max_tokens == 0 {
        return false;
    }

    // Quick check: if already under budget, do nothing
    let total: usize = messages.iter().map(estimate_message_tokens).sum();
    if total <= max_tokens {
        return false;
    }

    // Separate system messages from the rest
    let mut system_msgs: Vec<LlmMessage> = Vec::new();
    let mut non_system_msgs: Vec<LlmMessage> = Vec::new();

    for msg in messages.drain(..) {
        if msg.role == MessageRole::System {
            system_msgs.push(msg);
        } else {
            non_system_msgs.push(msg);
        }
    }

    // Remove oldest non-system messages until under budget
    // Always keep at least the last 2 messages (one exchange)
    while non_system_msgs.len() > 2 {
        // Build a temporary reference list to check token count
        let test_total: usize = system_msgs
            .iter()
            .chain(non_system_msgs.iter())
            .map(estimate_message_tokens)
            .sum();

        if test_total <= max_tokens {
            break;
        }
        non_system_msgs.remove(0);
    }

    // Restore: system first, then remaining non-system
    *messages = system_msgs;
    messages.extend(non_system_msgs);
    true
}

/// Try to summarize old conversation messages using the LLM.
///
/// When the conversation exceeds the token budget, this function attempts to
/// summarize older messages (keeping the latest exchange intact) by calling
/// the LLM with a summarization prompt. The summary is injected as a System
/// message. Falls back to `trim_conversation` if the LLM call fails or if
/// there aren't enough messages to make summarization worthwhile.
///
/// When `tool_store` is provided, Tool messages being summarized use the full
/// output retained in the store (see [`build_summary_prompt`]).
///
/// When `force` is `true`, the summarization is attempted even if the
/// conversation is under the token budget (used by the `/compact` command).
/// The rest of the logic (needs a provider, needs at least 4 non-system
/// messages, preserves system + latest exchange) is still respected.
///
/// Returns `true` if the conversation was actually modified (a summary was
/// injected or messages were trimmed), so callers can report the event.
pub(crate) async fn summarize_conversation(
    conversation: &mut Vec<LlmMessage>,
    max_tokens: usize,
    provider: Option<&dyn LlmProvider>,
    model: &str,
    force: bool,
    tool_store: Option<&ToolOutputStore>,
    retry_config: Option<&RetryConfig>,
) -> bool {
    // Quick check: if already under the compaction threshold, do nothing
    // (unless forced). The threshold is `max_tokens * COMPACTION_THRESHOLD_RATIO`.
    let total: usize = conversation_tokens(conversation);
    if !force && !should_compact(total, max_tokens, COMPACTION_THRESHOLD_RATIO) {
        return false;
    }

    // Need a provider for summarization
    let Some(prov) = provider else {
        return trim_conversation(conversation, max_tokens);
    };

    // Separate system from non-system
    let mut system_msgs: Vec<LlmMessage> = Vec::new();
    let mut non_system_msgs: Vec<LlmMessage> = Vec::new();

    for msg in conversation.drain(..) {
        if msg.role == MessageRole::System {
            system_msgs.push(msg);
        } else {
            non_system_msgs.push(msg);
        }
    }

    // Need at least 4 non-system messages to make summarization worthwhile
    if non_system_msgs.len() < 4 {
        system_msgs.extend(non_system_msgs);
        *conversation = system_msgs;
        return trim_conversation(conversation, max_tokens);
    }

    // Keep the last 2 messages (latest exchange), summarize the rest
    let keep = 2;
    let split_at = non_system_msgs.len() - keep;
    let to_summarize: Vec<LlmMessage> = non_system_msgs.drain(..split_at).collect();
    let recent = non_system_msgs;

    // Build summarization prompt using the structured, anchored template.
    let summary_prompt = build_summary_prompt(&to_summarize, tool_store);

    // Call LLM for summarization
    let request = LlmRequest {
        model: model.to_string(),
        messages: vec![LlmMessage {
            role: MessageRole::User,
            content: summary_prompt,
            tool_calls: None,
            tool_call_id: None,
        }],
        tools: vec![],
        max_tokens: Some(1024),
        temperature: Some(0.3),
        stream: false,
        cache_control: None,
    };

    // Wrap the summarization call with retry for transient errors
    let default_retry = RetryConfig::default();
    let retry_cfg = retry_config.unwrap_or(&default_retry);
    let summarize_result = retry_with_backoff_filtered(
        |_attempt| {
            let req = request.clone();
            async { prov.complete(req).await }
        },
        retry_cfg,
        "summarize_conversation",
        |e: &crate::error::Error| error_message_is_retriable(&e.to_string()),
    )
    .await;

    match summarize_result {
        Ok(response) => {
            // Replace summarized messages with an anchored summary system message
            system_msgs.push(LlmMessage {
                role: MessageRole::System,
                content: format!("{SUMMARY_ANCHOR_MARKER}\n{}", response.content.trim()),
                tool_calls: None,
                tool_call_id: None,
            });
        }
        Err(_) => {
            // Fallback: restore and truncate
            system_msgs.extend(to_summarize);
            system_msgs.extend(recent);
            *conversation = system_msgs;
            return trim_conversation(conversation, max_tokens);
        }
    }

    // Rebuild conversation
    *conversation = system_msgs;
    conversation.extend(recent);

    // Final trim check (summary itself might still be over budget)
    trim_conversation(conversation, max_tokens);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Result;
    use crate::llm::types::{LlmResponse, LlmStreamChunk, LlmUsage, ToolCall, ToolFunction};

    #[test]
    fn test_build_summary_prompt_contains_anchored_sections() {
        let messages = vec![LlmMessage {
            role: MessageRole::User,
            content: "Implement auth".into(),
            tool_calls: None,
            tool_call_id: None,
        }];
        let prompt = build_summary_prompt(&messages, None);
        for section in [
            "## Objetivo",
            "## Decisiones tomadas",
            "## Hechos y contexto clave",
            "## Código/patrones relevantes",
            "## Tareas pendientes",
            "## Riesgos o bloqueos",
        ] {
            assert!(
                prompt.contains(section),
                "prompt should contain anchored section '{section}'"
            );
        }
        // The conversation content should also be included.
        assert!(prompt.contains("Implement auth"));
    }

    #[test]
    fn test_build_summary_prompt_uses_full_tool_output_from_store() {
        // A Tool message whose content was truncated for the LLM, but whose
        // full output is retained in the store.
        let messages = vec![LlmMessage {
            role: MessageRole::Tool,
            content: "truncated output...".into(),
            tool_calls: None,
            tool_call_id: Some("call_1".into()),
        }];

        // Without a store, the truncated content is used.
        let prompt_no_store = build_summary_prompt(&messages, None);
        assert!(prompt_no_store.contains("truncated output..."));
        assert!(!prompt_no_store.contains("FULL OUTPUT"));

        // With a store containing the full output, the full output is used.
        let mut store = ToolOutputStore::new();
        store.insert("call_1", "FULL OUTPUT of the tool call");
        let prompt_with_store = build_summary_prompt(&messages, Some(&store));
        assert!(prompt_with_store.contains("FULL OUTPUT of the tool call"));
        assert!(!prompt_with_store.contains("truncated output..."));
    }

    #[test]
    fn test_build_summary_prompt_falls_back_to_truncated_when_id_missing() {
        // A Tool message whose id is NOT in the store falls back to its own
        // (truncated) content.
        let messages = vec![LlmMessage {
            role: MessageRole::Tool,
            content: "truncated output...".into(),
            tool_calls: None,
            tool_call_id: Some("call_missing".into()),
        }];
        let store = ToolOutputStore::new();
        let prompt = build_summary_prompt(&messages, Some(&store));
        assert!(prompt.contains("truncated output..."));
    }

    #[test]
    fn test_should_compact_threshold_logic() {
        let max_tokens = 1000;
        // Below the 0.8 ratio -> no compaction.
        assert!(!should_compact(799, max_tokens, COMPACTION_THRESHOLD_RATIO));
        // Exactly at the threshold (800) -> still not over it.
        assert!(!should_compact(800, max_tokens, COMPACTION_THRESHOLD_RATIO));
        // Over the threshold -> compact.
        assert!(should_compact(801, max_tokens, COMPACTION_THRESHOLD_RATIO));
        // A ratio of 1.0 effectively disables threshold-based compaction.
        assert!(!should_compact(1000, max_tokens, 1.0));
        // Zero max_tokens never compacts.
        assert!(!should_compact(100, 0, COMPACTION_THRESHOLD_RATIO));
    }
    #[tokio::test]
    async fn test_summarize_under_budget_no_change() {
        let mut msgs = vec![
            LlmMessage {
                role: MessageRole::User,
                content: "hello".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "world".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let original_len = msgs.len();
        summarize_conversation(&mut msgs, 9999, None, "test", false, None, None).await;
        assert_eq!(msgs.len(), original_len);
        assert_eq!(msgs[0].content, "hello");
    }

    #[tokio::test]
    async fn test_summarize_force_true_under_budget_trims() {
        // With `force = true`, summarization is attempted even under budget.
        // A provider is available, so the old messages are replaced by a
        // summary System message (the conversation shrinks).
        let mut msgs = vec![
            LlmMessage {
                role: MessageRole::System,
                content: "System prompt".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "msg1".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "resp1".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "msg2".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "resp2".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let original_len = msgs.len();
        let provider = MockProvider;
        // Generous budget: without `force` nothing would change.
        summarize_conversation(&mut msgs, 9999, Some(&provider), "test", true, None, None).await;
        // System preserved, oldest non-system replaced by a summary, last exchange kept.
        assert_eq!(msgs[0].role, MessageRole::System);
        assert!(msgs.len() < original_len);
        assert!(msgs.iter().any(|m| m.content.contains("SUMMARY")));
        assert_eq!(msgs[msgs.len() - 2].content, "msg2");
        assert_eq!(msgs[msgs.len() - 1].content, "resp2");
    }

    #[tokio::test]
    async fn test_summarize_force_false_under_budget_no_change() {
        // With `force = false` and a generous budget, nothing is trimmed.
        let mut msgs = vec![
            LlmMessage {
                role: MessageRole::System,
                content: "System prompt".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "msg1".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "resp1".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "msg2".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "resp2".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let original_len = msgs.len();
        summarize_conversation(&mut msgs, 9999, None, "test", false, None, None).await;
        assert_eq!(msgs.len(), original_len);
    }

    #[tokio::test]
    async fn test_summarize_over_budget_no_provider_falls_back_to_trim() {
        let mut msgs = vec![
            LlmMessage {
                role: MessageRole::System,
                content: "You are a helpful assistant.".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "What is Rust?".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "Rust is a systems programming language.".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "Tell me more about ownership.".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "Ownership is Rust's core feature.".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        // Very tight budget: only system + last 2 messages fit
        summarize_conversation(&mut msgs, 30, None, "test", false, None, None).await;
        // System should be preserved, oldest non-system dropped
        assert_eq!(msgs[0].role, MessageRole::System);
        assert!(msgs.len() >= 2); // at least system + last exchange
    }

    #[tokio::test]
    async fn test_summarize_few_messages_falls_back_to_trim() {
        let mut msgs = vec![
            LlmMessage {
                role: MessageRole::User,
                content: "hello".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "hi".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "how are you?".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        // Only 3 non-system messages (< 4), should fall back to trim
        summarize_conversation(&mut msgs, 5, None, "test", false, None, None).await;
        // Should still have at least 2 messages (last exchange)
        assert!(msgs.len() >= 2);
    }

    #[tokio::test]
    async fn test_trim_conversation_preserves_system_and_recent() {
        let mut msgs = vec![
            LlmMessage {
                role: MessageRole::System,
                content: "System prompt".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "msg1".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "resp1".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "msg2".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "resp2".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        trim_conversation(&mut msgs, 10);
        assert_eq!(msgs[0].role, MessageRole::System);
        assert_eq!(msgs[0].content, "System prompt");
        // Last exchange preserved
        assert_eq!(msgs[msgs.len() - 2].content, "msg2");
        assert_eq!(msgs[msgs.len() - 1].content, "resp2");
    }
    #[test]
    fn test_estimate_message_tokens_with_tool_calls() {
        let msg = LlmMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            tool_calls: Some(vec![
                ToolCall {
                    id: "call_abc".into(),
                    call_type: "function".into(),
                    function: ToolFunction {
                        name: "read_file".into(),
                        arguments: r#"{"path": "/src/main.rs"}"#.into(),
                    },
                },
                ToolCall {
                    id: "call_def".into(),
                    call_type: "function".into(),
                    function: ToolFunction {
                        name: "grep_search".into(),
                        arguments: r#"{"pattern": "fn main", "include": "*.rs"}"#.into(),
                    },
                },
            ]),
            tool_call_id: None,
        };
        let tokens = estimate_message_tokens(&msg);
        // content is empty (0 tokens), but tool_calls should add tokens
        assert!(tokens > 0, "tool_calls should contribute tokens");
        // Should be substantially more than estimate_tokens(&"") which is 0
        assert!(
            tokens > estimate_tokens(&msg.content),
            "tool_calls should add more tokens than empty content"
        );
    }

    #[test]
    fn test_estimate_message_tokens_with_tool_call_id() {
        let msg = LlmMessage {
            role: MessageRole::Tool,
            content: "result data".into(),
            tool_calls: None,
            tool_call_id: Some("call_abc123".into()),
        };
        let tokens = estimate_message_tokens(&msg);
        let content_only = estimate_tokens(&msg.content);
        assert!(tokens > content_only, "tool_call_id should add tokens");
    }

    #[test]
    fn test_estimate_message_tokens_content_only() {
        let msg = LlmMessage {
            role: MessageRole::User,
            content: "hello world".into(),
            tool_calls: None,
            tool_call_id: None,
        };
        let tokens = estimate_message_tokens(&msg);
        let expected = estimate_tokens(&msg.content);
        assert_eq!(
            tokens, expected,
            "content-only message should match estimate_tokens"
        );
    }

    #[test]
    fn test_conversation_tokens_mixed() {
        let messages = vec![
            LlmMessage {
                role: MessageRole::User,
                content: "hello".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: String::new(),
                tool_calls: Some(vec![ToolCall {
                    id: "call_1".into(),
                    call_type: "function".into(),
                    function: ToolFunction {
                        name: "shell".into(),
                        arguments: r#"{"command": "ls"}"#.into(),
                    },
                }]),
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Tool,
                content: "file list".into(),
                tool_calls: None,
                tool_call_id: Some("call_1".into()),
            },
        ];
        let total = conversation_tokens(&messages);
        // Should be sum of all three messages with their tool fields
        let expected: usize = messages.iter().map(estimate_message_tokens).sum();
        assert_eq!(total, expected);
        // Should be greater than content-only estimate
        let content_only: usize = messages.iter().map(|m| estimate_tokens(&m.content)).sum();
        assert!(total >= content_only);
    }

    #[test]
    fn test_trim_conversation_respects_tool_calls() {
        let mut messages = vec![
            LlmMessage {
                role: MessageRole::System,
                content: "System prompt".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::User,
                content: "msg1".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: String::new(),
                tool_calls: Some(vec![ToolCall {
                    id: "call_big".into(),
                    call_type: "function".into(),
                    function: ToolFunction {
                        name: "large_tool".into(),
                        arguments: "x".repeat(300),
                    },
                }]),
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Tool,
                content: "result".into(),
                tool_calls: None,
                tool_call_id: Some("call_big".into()),
            },
            LlmMessage {
                role: MessageRole::User,
                content: "msg2".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            LlmMessage {
                role: MessageRole::Assistant,
                content: "resp2".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        // Budget that fits system + last exchange when counting tool_calls
        let total_with_tools: usize = messages.iter().map(estimate_message_tokens).sum();
        let budget = total_with_tools; // exactly fits
        let changed = trim_conversation(&mut messages, budget);
        // Should NOT trim anything since it fits exactly
        assert!(
            !changed,
            "should not trim when budget exactly fits with tool_calls"
        );
        assert_eq!(messages.len(), 6);
    }

    /// Minimal `LlmProvider` stub that returns a fixed summary, used to test
    /// `summarize_conversation` without hitting a real LLM API.
    struct MockProvider;

    #[async_trait::async_trait]
    impl crate::llm::provider::LlmProvider for MockProvider {
        async fn complete(&self, _request: LlmRequest) -> crate::error::Result<LlmResponse> {
            Ok(LlmResponse {
                content: "SUMMARY of earlier conversation".into(),
                tool_calls: vec![],
                finish_reason: "stop".into(),
                usage: None,
                thinking: None,
            })
        }

        async fn complete_stream(
            &self,
            _request: LlmRequest,
        ) -> crate::error::Result<tokio::sync::mpsc::Receiver<Result<LlmStreamChunk>>> {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let _ = tx
                .send(Ok(LlmStreamChunk::Done(LlmUsage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                    cost: None,
                })))
                .await;
            Ok(rx)
        }

        fn context_window(&self) -> usize {
            8192
        }

        async fn fetch_context_window(&self) -> crate::error::Result<usize> {
            Ok(8192)
        }

        fn set_context_window(&self, _value: usize) {}

        fn input_price_per_million(&self) -> f64 {
            0.0
        }

        fn output_price_per_million(&self) -> f64 {
            0.0
        }
    }
}
