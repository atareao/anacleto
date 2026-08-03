use rand::Rng;
/// Retry helper with exponential backoff + jitter.
///
/// Generic over any async operation returning `Result<T, E>` where `E: std::fmt::Display`.
/// Retries on any error, up to `config.max_retries` times, with exponential
/// backoff (base_delay_ms × 2^attempt) + 25% random jitter, capped at max_delay_ms.
use rand::rngs::OsRng;
use tokio::time::{Duration, sleep};

use crate::config::types::RetryConfig;

/// Retry an async operation with exponential backoff + jitter.
///
/// The operation closure receives the current attempt number (0-indexed) and
/// should return `Result<T, E>`. On error, it waits and retries.
pub async fn retry_with_backoff<Fut, T, E>(
    mut operation: impl FnMut(u32) -> Fut,
    config: &RetryConfig,
    operation_name: &str,
) -> Result<T, E>
where
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
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

    #[test]
    fn test_retry_config_defaults() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.base_delay_ms, 1000);
        assert_eq!(config.max_delay_ms, 30000);
    }
}
