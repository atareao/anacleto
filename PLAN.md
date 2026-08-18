# Plan: ToolOutputStore con resúmenes inteligentes

## Problema

El agente `code-analyzer` entraba en bucle cuando se le pedía analizar múltiples archivos. Causa raíz:

- `TOOL_RESULT_MAX_CHARS = 16000` — cada resultado de tool fill the conversation con ~16K caracteres
- Tras múltiples tools, la conversación llegaba a 79 mensajes, saturando el contexto
- El modelo perdía su razonamiento previo al tener que procesar todo el contenido de vuelta

## Solución (Opción 4)

Reemplazar el truncado plano con **resúmenes inteligentes con referencia al ToolOutputStore**:

1. **`summarize_tool_result()`** — en lugar de `truncate_output()` que corta ciegamente a 16K chars, produce un resumen estructurado:
   - Primeros 2000 caracteres del resultado
   - Últimos 500 caracteres del resultado
   - Marcador con tamaño total y referencia al store
   - Resultados ≤ 2700 chars pasan completos

2. **Nueva herramienta `get_tool_result(tool_call_id)`** — el LLM puede recuperar el contenido completo de cualquier resultado previo directamente desde el `ToolOutputStore`

3. **System prompt actualizado** — se explica el mecanismo al modelo para que sepa usar `get_tool_result()` cuando necesite el contenido completo

## Archivos modificados

### `src/agent/tool_store.rs`
- Nuevas constantes: `SUMMARY_FRONT_CHARS`, `SUMMARY_BACK_CHARS`, `SUMMARY_PASSTHROUGH_THRESHOLD`
- Nueva función pública: `summarize_tool_result(content, tool_call_id) -> String`
- `truncate_output()` se mantiene (usado en otras partes del sistema)
- 5 tests nuevos para `summarize_tool_result`

### `src/agent/tools.rs`
- Nueva función pública: `get_tool_result_tool_definition() -> ToolDefinition`
- Añadida a `builtin_tool_definitions()`

### `src/agent/session.rs`
- Importa `summarize_tool_result` (elimina import de `truncate_output`)
- Elimina constante `TOOL_RESULT_MAX_CHARS` (16000) — ya no se usa
- En `process()`: reemplaza `truncate_output(&result, TOOL_RESULT_MAX_CHARS)` por `summarize_tool_result(&result, &tc.id)`
- En `process()`: añade manejo especial para `get_tool_result` — lee directamente del `ToolOutputStore`
- En `execute_builtin_tool()`: añade redirect para `get_tool_result` (safety net)
- En `render_system_prompt()`: añade sección "Tool Output Store" explicando el mecanismo

## Verificación

- `cargo check`: 0 errors, 0 warnings
- `cargo test`: 475 tests, 0 failures
- `cargo clippy`: sin warnings nuevos
- `cargo fmt --check`: clean