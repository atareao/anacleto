# Plan: Política de reintentos inteligente para LLM providers

## Objetivo
Implementar una política de reintentos configurable desde YAML para errores de conectividad con proveedores LLM (timeouts, 5xx, rate limits), distinguiendo errores retriables de no retriables (auth 401, bad request 400).

## Estado actual
- `RetryConfig` ya existe en `config/types.rs` y se usa desde `session.retry`
- `retry_with_backoff` ya existe en `agent/retry.rs` pero reintenta **todos** los errores
- `lifecycle.rs` y `tools.rs` ya envuelven `complete_stream` con retry
- `summarize_conversation` en `context.rs` llama a `prov.complete()` **sin** retry

## Cambios

### 1. Clasificación de errores retriables (`src/agent/retry.rs`)
- Añadir `fn error_message_is_retriable(msg: &str) -> bool`
- Retriable: timeout, connection refused/reset, 5xx, 429, DNS/TLS errors
- No retriable: 4xx (except 429), auth, invalid request, schema parse errors
- Modificar `retry_with_backoff` para aceptar `should_retry: Option<impl Fn(&E) -> bool>`

### 2. RetryConfig opcional por provider (`src/config/types.rs`)
- Añadir `retry: Option<RetryConfig>` a `ProviderConfig` y `OllamaConfig`

### 3. RetryConfig en LlmProviderConfig (`src/llm/types.rs`)
- Añadir `retry: RetryConfig` a `LlmProviderConfig`

### 4. Propagar en conversiones (`src/engine/orchestrator.rs`)
- `provider_config_to_llm` recibe `session_retry: &RetryConfig`
- `ollama_config_to_llm` recibe `session_retry: &RetryConfig`

### 5. Retry en summarize_conversation (`src/agent/context.rs`)
- Añadir parámetro `retry_config: &RetryConfig`
- Envolver `prov.complete()` con `retry_with_backoff` + `error_message_is_retriable`

### 6. Pasar retry_config en llamadas a summarize
- `lifecycle.rs`: 3 llamadas (líneas 332, 378, 1072)
- `tools.rs`: 1 llamada (línea 1644)

## Archivos a modificar
1. `src/agent/retry.rs`
2. `src/config/types.rs`
3. `src/llm/types.rs`
4. `src/engine/orchestrator.rs`
5. `src/agent/context.rs`
6. `src/agent/lifecycle.rs`
7. `src/agent/tools.rs`

## Verificación
- `cargo test` (todos los tests existentes + nuevos)
- `cargo clippy` (sin warnings nuevos)