# Glossary — Builtin Core Tools

## Términos generales

| Término | Definición |
|---|---|
| **Builtin tool** | Herramienta nativa del sistema Anacleto, definida en `builtin_tool_definitions()`. Se diferencia de los skills (cargados dinámicamente) y de los plugins (registrados externamente). |
| **Tool definition** | Especificación JSON Schema de un tool, incluyendo nombre, descripción y schema de entrada. Se envía al LLM como parte de la lista de herramientas disponibles. |
| **Tool dispatch** | Proceso en el bucle de ejecución del agente (`src/agent/lifecycle.rs`) que recibe un `ToolCall` del LLM y lo rutea al ejecutor correspondiente. |
| **Permission check** | Verificación de que el agente tiene permiso para ejecutar una operación. Se realiza antes del dispatch, via `check_tool_permission()`. |
| **Plan mode** | Modo de solo lectura del agente. Bloquea cualquier tool que modifique el filesystem. |

## Builtins propuestos

### Familia `insert_lines` / `replace_lines` / `delete_lines`

| Término | Definición |
|---|---|
| **Line-based editing** | Edición de archivos usando números de línea (1-based) en lugar de coincidencia de texto. Elimina la fragilidad de `apply_patch`. |
| **after_line** | Parámetro que indica la línea después de la cual insertar contenido. `0` significa "al principio del archivo". |
| **start_line / end_line** | Rango inclusivo de líneas a reemplazar o eliminar. `start_line` debe ser ≤ `end_line`. |
| **Line number drift** | Fenómeno por el cual los números de línea cambian entre una lectura y una edición debido a modificaciones concurrentes. |

### `format_document`

| Término | Definición |
|---|---|
| **LSP formatting** | Uso del protocolo `textDocument/formatting` del LSP para formatear código según la configuración del proyecto. |
| **Server detection** | Mapeo de extensión de archivo a comando del LSP server (ej: `.rs` → `rust-analyzer`). Definido en `default_server_for_extension()`. |
| **TextEdit[]** | Array de ediciones devuelto por el LSP. Cada edición especifica un rango y un texto de reemplazo. |

### `search_symbol`

| Término | Definición |
|---|---|
| **CodeGraph** | Grafo de conocimiento de código fuente basado en tree-sitter. Indexa todos los símbolos, sus definiciones y relaciones. |
| **Symbol kind** | Tipo de símbolo: `function`, `method`, `struct`, `enum`, `trait`, `type`, `variable`, `interface`, `component`, `route`. |
| **Semantic search** | Búsqueda basada en el significado del código (símbolos definidos) vs búsqueda textual (grep). |

## Conceptos arquitectónicos

| Término | Definición |
|---|---|
| **Workspace** | Directorio raíz del proyecto. Todos los paths de archivos se resuelven relativos a este directorio. |
| **Path traversal** | Intento de acceder a un archivo fuera del workspace usando `..` o paths absolutos. Bloqueado por defecto. |
| **fs.write permission** | Permiso requerido para modificar archivos en el filesystem. |
| **command.run permission** | Permiso requerido para ejecutar comandos (incluyendo LSP servers). |
| **mcp.use permission** | Permiso requerido para usar herramientas MCP (incluyendo CodeGraph). |
| **Tool display template** | Plantilla de visualización configurable en `config.yaml` que personaliza cómo se muestra un tool en la TUI. Ej: `"📝 insertando en {path}"`. |