use rand::Rng;
use rand::rngs::OsRng;
use tokio::time::{Duration, sleep};

use crate::config::types::RetryConfig;

/// Check if an error message corresponds to a retriable (transient) failure.
///
/// Retriable errors include:
/// - Network/timeout errors (connection refused, reset, DNS, TLS)
/// - Server errors (5xx)
/// - Rate limiting (429)
///
/// Non-retriable errors include:
/// - Client errors (4xx except 429): auth (401), forbidden (403), not found (404),
///   bad request (400), invalid request (422), etc.
/// - Parse/validation errors from the response
pub fn error_message_is_retriable(msg: &str) -> bool {
    let lower = msg.to_lowercase();

    // Network and transport errors — always retriable
    if lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("connection closed")
        || lower.contains("broken pipe")
        || lower.contains("dns")
        || lower.contains("tls")
        || lower.contains("certificate")
        || lower.contains("eof")
        || lower.contains("no route to host")
        || lower.contains("temporarily unavailable")
    {
        return true;
    }

    // Rate limiting — retriable (with backoff)
    if lower.contains("429") || lower.contains("rate limit") || lower.contains("too many requests")
    {
        return true;
    }

    // Server errors — retriable
    if lower.contains("50")
        && (lower.contains("server error")
            || lower.contains("internal server")
            || lower.contains("bad gateway")
            || lower.contains("service unavailable")
            || lower.contains("gateway timeout"))
    {
        return true;
    }
    if lower.contains("502") || lower.contains("503") || lower.contains("504") {
        return true;
    }

    // 5xx generic status code mention
    if lower.contains("5xx") || lower.contains("status 5") {
        return true;
    }

    // Service overload / contention
    if lower.contains("overload")
        || lower.contains("capacity")
        || lower.contains("throttl")
        || lower.contains("busy")
    {
        return true;
    }

    false
}

/// Retry an async operation with exponential backoff + jitter.
///
/// By default, retries on ANY error. If `should_retry` is provided, only
/// errors for which the predicate returns `true` will be retried; all other
/// errors are returned immediately.
///
/// The operation closure receives the current attempt number (0-indexed) and
/// should return `Result<T, E>`. On retriable error, it waits and retries.
pub async fn retry_with_backoff<Fut, T, E>(
    operation: impl FnMut(u32) -> Fut,
    config: &RetryConfig,
    operation_name: &str,
) -> Result<T, E>
where
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    retry_with_backoff_impl(operation, config, operation_name, None::<fn(&E) -> bool>).await
}

/// Like [`retry_with_backoff`], but with an optional predicate controlling
/// which errors are retriable.
///
/// When `should_retry` is `Some`, only errors for which the closure returns
/// `true` will trigger a retry. All other errors are returned immediately.
pub async fn retry_with_backoff_filtered<Fut, T, E>(
    operation: impl FnMut(u32) -> Fut,
    config: &RetryConfig,
    operation_name: &str,
    should_retry: impl Fn(&E) -> bool,
) -> Result<T, E>
where
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    retry_with_backoff_impl(operation, config, operation_name, Some(should_retry)).await
}

async fn retry_with_backoff_impl<Fut, T, E, F>(
    mut operation: impl FnMut(u32) -> Fut,
    config: &RetryConfig,
    operation_name: &str,
    should_retry: Option<F>,
) -> Result<T, E>
where
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
    F: Fn(&E) -> bool,
{
    let mut last_error: Option<E> = None;

    for attempt in 0..=config.max_retries {
        if attempt > 0 {
            // Calculate delay: base * 2^(attempt-1) with jitter
            let base = config.base_delay_ms as f64;
            let exponent = attempt - 1;
            let delay = base * (2u64.pow(exponent) as f64);

            // Cap at max_delay
            let delay = delay.min(config.max_delay_ms as f64);

            // Add 25% random jitter (0.75x to 1.25x)
            let mut rng = OsRng;
            let jitter = rng.gen_range(0.75..1.25);
            let delay_ms = (delay * jitter) as u64;

            tracing::warn!(
                "{} attempt {}/{}, retrying in {}ms",
                operation_name,
                attempt,
                config.max_retries,
                delay_ms,
            );

            sleep(Duration::from_millis(delay_ms)).await;
        }

        match operation(attempt).await {
            Ok(value) => return Ok(value),
            Err(e) => {
                // If we have a predicate and the error is NOT retriable, bail immediately
                if let Some(ref pred) = should_retry {
                    if !pred(&e) {
                        return Err(e);
                    }
                }
                last_error = Some(e);
            }
        }
    }

    Err(last_error.expect("retry_with_backoff called with max_retries=0 but no result"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    // -----------------------------------------------------------------------
    // error_message_is_retriable
    // -----------------------------------------------------------------------

    #[test]
    fn test_retriable_timeout() {
        assert!(error_message_is_retriable("request timed out after 30s"));
        assert!(error_message_is_retriable("timeout: connection to api.openai.com"));
    }

    #[test]
    fn test_retriable_connection() {
        assert!(error_message_is_retriable("connection refused: 127.0.0.1:11434"));
        assert!(error_message_is_retriable("connection reset by peer"));
        assert!(error_message_is_retriable("broken pipe"));
        assert!(error_message_is_retriable("no route to host"));
        assert!(error_message_is_retriable("temporarily unavailable"));
    }

    #[test]
    fn test_retriable_rate_limit() {
        assert!(error_message_is_retriable("HTTP 429: Too Many Requests"));
        assert!(error_message_is_retriable("rate limit exceeded, please slow down"));
        assert!(error_message_is_retriable("too many requests, retry after 60s"));
    }

    #[test]
    fn test_retriable_server_error() {
        assert!(error_message_is_retriable("502 Bad Gateway"));
        assert!(error_message_is_retriable("503 Service Unavailable"));
        assert!(error_message_is_retriable("504 Gateway Timeout"));
        assert!(error_message_is_retriable("internal server error (500)"));
        assert!(error_message_is_retriable("server error: 5xx"));
    }

    #[test]
    fn test_non_retriable_auth() {
        assert!(!error_message_is_retriable("401 Unauthorized: invalid API key"));
        assert!(!error_message_is_retriable("403 Forbidden: insufficient permissions"));
    }

    #[test]
    fn test_non_retriable_bad_request() {
        assert!(!error_message_is_retriable("400 Bad Request: invalid model"));
        assert!(!error_message_is_retriable("404 Not Found: model not available"));
        assert!(!error_message_is_retriable("422 Unprocessable Entity: invalid schema"));
    }

    #[test]
    fn test_non_retriable_parse_error() {
        assert!(!error_message_is_retriable("JSON parse error: expected value at line 1"));
        assert!(!error_message_is_retriable("failed to deserialize response"));
    }

    // -----------------------------------------------------------------------
    // retry_with_backoff
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_retry_succeeds_first_try() {
        let config = RetryConfig::default();
        let result =
            retry_with_backoff(|_attempt| async { Ok::<_, String>(42) }, &config, "test").await;
        assert_eq!(result, Ok(42));
    }

    #[tokio::test]
    async fn test_retry_succeeds_after_failures() {
        let config = RetryConfig {
            max_retries: 3,
            base_delay_ms: 10, // small for tests
            max_delay_ms: 100,
        };
        let attempts = AtomicU32::new(0);

        let result = retry_with_backoff(
            |_attempt| {
                let prev = attempts.fetch_add(1, Ordering::SeqCst);
                async move {
                    if prev < 2 {
                        Err::<i32, String>("not yet".into())
                    } else {
                        Ok::<i32, String>(99)
                    }
                }
            },
            &config,
            "test",
        )
        .await;
        assert_eq!(result, Ok(99));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_exhausts_and_fails() {
        let config = RetryConfig {
            max_retries: 2,
            base_delay_ms: 10,
            max_delay_ms: 100,
        };

        let result = retry_with_backoff(
            |_attempt| async { Err::<i32, String>("always fail".into()) },
            &config,
            "test",
        )
        .await;
        assert_eq!(result, Err("always fail".into()));
    }

    #[tokio::test]
    async fn test_filtered_retry_skips_non_retriable() {
        // A non-retriable error should NOT be retried at all.
        let config = RetryConfig {
            max_retries: 3,
            base_delay_ms: 10,
            max_delay_ms: 100,
        };
        let attempts = AtomicU32::new(0);

        let result = retry_with_backoff_filtered(
            |_attempt| {
                let prev = attempts.fetch_add(1, Ordering::SeqCst);
                async move {
                    if prev < 1 {
                        Err::<i32, String>("401 Unauthorized: invalid API key".into())
                    } else {
                        Ok::<i32, String>(42)
                    }
                }
            },
            &config,
            "test_filtered",
            |e: &String| error_message_is_retriable(e),
        )
        .await;
        // Should fail immediately on the first error (non-retriable)
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "401 Unauthorized: invalid API key");
        // Only 1 attempt (the first one failed, predicate returned false, no retry)
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_filtered_retry_retries_retriable() {
        // A retriable error SHOULD be retried.
        let config = RetryConfig {
            max_retries: 3,
            base_delay_ms: 10,
            max_delay_ms: 100,
        };
        let attempts = AtomicU32::new(0);

        let result = retry_with_backoff_filtered(
            |_attempt| {
                let prev = attempts.fetch_add(1, Ordering::SeqCst);
                async move {
                    if prev < 2 {
                        Err::<i32, String>("503 Service Unavailable".into())
                    } else {
                        Ok::<i32, String>(42)
                    }
                }
            },
            &config,
            "test_filtered_retry",
            |e: &String| error_message_is_retriable(e),
        )
        .await;
        assert_eq!(result, Ok(42));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_retry_config_defaults() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.base_delay_ms, 1000);
        assert_eq!(config.max_delay_ms, 30000);
    }
}
