# Anacleto — Guía de Usuario

**Versión:** 0.17.1  
**Repositorio:** https://github.com/atareao/anacleto  
**Licencia:** MIT  

Anacleto es un motor de orquestación de agentes construido en Rust. Gestiona un árbol de agentes y subagentes con separación limpia de skills, servidores MCP y permisos. La única interfaz es una TUI (Terminal User Interface) construida con [ratatui](https://github.com/ratatui-org/ratatui) + [crossterm](https://github.com/crossterm-rs/crossterm).

---

## Índice

1. [Primeros pasos](#1-primeros-pasos)
2. [Conceptos clave](#2-conceptos-clave)
3. [Configuración](#3-configuración)
4. [Agentes y subagentes](#4-agentes-y-subagentes)
    - [Tools (herramientas built-in)](#tools-herramientas-built-in)
5. [Skills](#5-skills)
6. [MCPs (Model Context Protocol)](#6-mcps-model-context-protocol)
7. [Permisos](#7-permisos)
8. [Sesiones](#8-sesiones)
9. [Comandos TUI](#9-comandos-tui)
10. [Atajos de teclado y navegación](#10-atajos-de-teclado-y-navegación)
11. [Modo debug](#11-modo-debug)
12. [Modo headless](#12-modo-headless)
13. [Hooks y automatización](#13-hooks-y-automatización)
14. [Plugins](#14-plugins)
15. [Solución de problemas](#15-solución-de-problemas)

---

## 1. Primeros pasos

### Instalación

```sh
# Clonar el repositorio
$ git clone https://github.com/atareao/anacleto.git
$ cd anacleto

# Compilar
$ cargo build --release

# El binario estará en ./target/release/anacleto
```

### Configuración mínima

Crea `~/.config/anacleto/config.yaml`:

```yaml
models:
  anthropic:
    api_key: "${ANTHROPIC_API_KEY}"
    model: "claude-sonnet-4-20250514"
    context_window: 200000

  openrouter:
    api_key: "${OPENROUTER_API_KEY}"
    model: "openai/gpt-4o"
    context_window: 128000
    base_url: "https://openrouter.ai/api/v1"

session:
  database_path: "~/.local/share/anacleto/sessions.db"
```

Asegúrate de tener las variables de entorno con tus API keys:

```sh
export ANTHROPIC_API_KEY="sk-ant-..."
export OPENROUTER_API_KEY="sk-or-..."
```

### Ejecutar

```sh
# Desde el directorio del proyecto (busca .agents/ automáticamente)
$ anacleto

# Con un config específico
$ anacleto -c ruta/a/mi-config.yaml

# Modo verbose
$ anacleto -v

# Modo debug (muestra payloads LLM)
$ anacleto --debug

# Modo headless (sin TUI, útil para scripting)
$ anacleto --headless --task "Analiza este proyecto"
```

---

## 2. Conceptos clave

### ¿Qué es un agente?

Un **agente** es una entidad configurable que puede ser invocada directamente por el usuario. Cada agente tiene:

- **Un system prompt** (personalidad, instrucciones)
- **Un modelo LLM** (qué cerebro usa)
- **Skills** (capacidades especializadas)
- **MCPs** (herramientas externas)
- **Permisos** (qué puede y no puede hacer)
- **Subagentes** (a quién puede delegar trabajo)

### ¿Qué es un subagente?

Un **subagente** es un agente especializado que NO puede ser invocado directamente por el usuario. Solo su agente padre puede crearlo. Los subagentes:

- Son **desechables** (se crean para una tarea y se destruyen al completarla)
- Son **independientes** (no heredan skills, MCPs ni permisos del padre)
- No pueden tener subagentes propios
- Se comunican con el padre solo por mensajes de texto

### ¿Qué es una skill?

Una **skill** es una capacidad especializada que se le da a un agente. Se define como un archivo Markdown con instrucciones para el LLM. Ejemplos: `shell` (ejecutar comandos), `code-review` (revisar código), `weather` (consultar el tiempo).

### ¿Qué es un MCP?

Un **MCP (Model Context Protocol)** es un servidor externo que expone herramientas y recursos al agente. Se comunica via JSON-RPC 2.0 por stdio o TCP. Ejemplos: servidor de base de datos, analizador de código, etc.

### ¿Qué es una sesión?

Una **sesión** es una conversación completa con Anacleto, persistida en SQLite. Las sesiones son reanudables: puedes cerrar Anacleto y al volver retomar exactamente donde lo dejaste.

---

## 3. Configuración

### Sistema de dos capas

| Nivel | Ruta | Propósito |
|---|---|---|
| **Global** | `~/.config/anacleto/config.yaml` | API keys, MCPs compartidos, defaults |
| **Proyecto** | `.agents/config.yaml` | Overrides por proyecto |

La configuración de proyecto se fusiona SOBRE la global. Los valores del proyecto sobrescriben los globales.

### Variables de entorno

Las API keys y otros secretos se referencian con `${VAR_NAME}`:

```yaml
models:
  anthropic:
    api_key: "${ANTHROPIC_API_KEY}"
```

### Schema completo

```yaml
# ── Proveedores LLM ──────────────────────────────────────────────
models:
  anthropic:        # Claude
    api_key: "${ANTHROPIC_API_KEY}"
    model: "claude-sonnet-4-20250514"
    context_window: 200000

  openai:           # GPT
    api_key: "${OPENAI_API_KEY}"
    model: "gpt-4o"
    context_window: 128000

  openrouter:       # OpenAI-compatible (DeepSeek, etc.)
    api_key: "${OPENROUTER_API_KEY}"
    model: "openai/gpt-4o"
    context_window: 128000
    base_url: "https://openrouter.ai/api/v1"

  ollama:           # Local
    base_url: "http://localhost:11434"
    model: "llama3.2"
    context_window: 8192

  azure:            # Azure OpenAI
    api_key: "${AZURE_API_KEY}"
    model: "gpt-4o"
    base_url: "https://<resource>.openai.azure.com"

  bedrock:          # AWS Bedrock
    api_key: "${AWS_ACCESS_KEY_ID}"
    model: "anthropic.claude-sonnet-4"
    base_url: "https://bedrock-runtime.<region>.amazonaws.com"

  google:           # Google Gemini
    api_key: "${GOOGLE_API_KEY}"
    model: "gemini-2.0-flash"
    base_url: "https://generativelanguage.googleapis.com/v1beta"

  cache:            # Política de cacheo de prompts
    mode: auto      # auto | off

# ── Servidores MCP ───────────────────────────────────────────────
mcps:
  codegraph:
    transport: stdio
    command: "/usr/local/bin/codegraph"
    args: ["mcp"]

# ── Sesión ────────────────────────────────────────────────────────
session:
  history_limit_percent: 50    # % del context window para historial
  database_path: "~/.local/share/anacleto/sessions.db"
  max_steps: 100               # pasos máximos por defecto para agentes
  retry:
    max_retries: 3
    base_delay_ms: 1000
    max_delay_ms: 30000
  debug: false                 # debug mode por defecto

# ── Directorios de trabajo conocidos ─────────────────────────────
workspaces:
  - ~/projects
  - ~/work

# ── Overrides de herramientas shell ───────────────────────────────
shell:
  tools:
    - name: bat
      classic: cat
      description: "view files with syntax highlighting"

# ── Atajos de teclado personalizados ──────────────────────────────
keymap:
  keys:
    submit: "enter"
    command_palette: "ctrl+p"
    search: "ctrl+f"

# ── Editor externo ───────────────────────────────────────────────
editor: "code -w"

# ── Comandos slash personalizados ────────────────────────────────
commands:
  - name: deploy
    description: Desplegar el proyecto
    command: "ansible-playbook deploy.yml"
    timeout: 120

# ── Hooks ─────────────────────────────────────────────────────────
hooks:
  after_apply:
    - type: shell
      command: "codegraph sync"
      timeout_secs: 60
```

### Resolución de proveedor por modelo

El modelo configurado en el agente determina qué proveedor lo sirve:

| Patrón | Proveedor |
|---|---|
| Empieza con `claude` | Anthropic |
| Empieza con `gpt`/`o1`/`o3` | OpenAI |
| Contiene `/` | OpenRouter |
| Cualquier otro | Ollama |
| Config explícita | Azure, Bedrock, Google |

### Reglas de fusión de config

1. Cargar config global (`~/.config/anacleto/config.yaml`)
2. Cargar config de proyecto (`.agents/config.yaml`) si existe
3. Fusionar: proyecto sobrescribe global
4. Resolver `${VAR}` de variables de entorno
5. Flags CLI (`--config`, `--database`) sobrescriben todo

---

## 4. Agentes y subagentes

### Cómo se definen los agentes

Los agentes NO se definen en YAML. Se definen como archivos **Markdown con frontmatter YAML** en:

| Ámbito | Ruta |
|---|---|
| Global | `~/.config/anacleto/agents/<nombre>.md` |
| Proyecto | `.agents/agents/<nombre>.md` |

Formato:

```markdown
---
name: root
role: root
description: Senior engineering agent
model: deepseek/deepseek-v4-flash
max_steps: 90
skills:
  - .agents/skills/shell/
  - .agents/skills/filesystem/
mcps: [codegraph]
permissions:
  deny:
    - command.run.sudo
    - net.http.delete
subagents:
  - reviewer
  - writer
  - rust-dev
subagent_depth: 3
---

You are **Anacleto**, a senior engineering agent...
```

### Campos del frontmatter

| Campo | Obligatorio | Descripción |
|---|---|---|
| `name` | ✅ | Nombre único del agente |
| `description` | ✅ | Descripción (para el LLM y el listado) |
| `role` | ❌ | `root` o `subagent` (default: `subagent`) |
| `model` | ❌ | Modelo LLM (default: `claude-sonnet-4-20250514`) |
| `max_steps` | ❌ | Pasos máximos por tarea (default: 100) |
| `subagent_depth` | ❌ | Profundidad de delegación dinámica |
| `skills` | ❌ | Lista de rutas a skills |
| `mcps` | ❌ | Lista de nombres de MCP |
| `permissions` | ❌ | Permisos (allow/deny) |
| `subagents` | ❌ | Lista de subagentes (solo root) |

### Agentes raíz disponibles en el proyecto

| Agente | Propósito | Skills | Subagentes |
|---|---|---|---|
| **root** | Ingeniería senior | 12 skills (shell, filesystem, web-research, code-review, rust-dev, find-skills, skill-creator, agent-creator, planning, version-control, tool-discovery, weather) | reviewer, writer, rust-dev, tech-writer, python-dev |
| **chat** | Conversación y tiempo | weather, shell | — |

### Cómo cambiar de agente

Usa `/agent <nombre>` en la TUI o `/agents` para listar disponibles.

### Cómo crear un nuevo agente

1. Crea `~/.config/anacleto/agents/<nombre>.md` (global) o `.agents/agents/<nombre>.md` (proyecto)
2. Añade frontmatter YAML con los campos necesarios
3. Escribe el system prompt en el cuerpo Markdown
4. Ejecuta `/reload` en la TUI para recargar

### Subagentes

Los subagentes se definen IGUAL que los agentes, con `role: subagent`. La diferencia es que:

- No son invocables directamente por el usuario
- Los crea su agente padre cuando necesita ayuda especializada
- Se destruyen al completar la tarea
- No pueden tener subagentes

Ejemplo de subagente (`.agents/agents/reviewer.md`):

```markdown
---
name: reviewer
role: subagent
description: Code review specialist
model: deepseek/deepseek-v4-flash
skills:
  - .agents/skills/code-review/
mcps: []
permissions:
  deny:
    - command.run
    - net.http
subagents: []
---

You are a code review specialist...
```

Para ver subagentes activos: `/subagents` o `/sa`.

### Tools (herramientas built-in)

Cada agente declara explícitamente qué herramientas built-in necesita en su frontmatter `tools:`. Si una herramienta no está en la lista, el agente no tiene acceso a ella.

**Principios:**

- **Sin herramientas core** — Ni `task`, `question`, ni `todo` son obligatorias.
- **Sin herencia** — Los subagentes no heredan herramientas del padre.
- **Declaración explícita** — Cada agente lista sus herramientas.

**Ejemplo:**

```yaml
tools:
  read:
    color: cyan
  grep:
    color: blue
  bash:
    color: green
    display: "$ {command}"
  question:
    color: yellow
```

Las herramientas disponibles son: `read`, `grep`, `glob`, `bash`, `webfetch`, `websearch`, `todo`, `question`, `delegate`, `compress`, `skill`, `apply_patch`, `mcp_list_resources`, `mcp_read_resource`, `mcp_list_resource_templates`, `lsp_query`.

Los valores por defecto (description, show, display, color) se pueden configurar globalmente en `~/.config/anacleto/config.yaml` bajo la clave `tools:`.

---

## 5. Skills

### ¿Qué es una skill?

Una skill es un archivo Markdown con instrucciones especializadas para el LLM. El formato sigue el estándar Anthropic:

```markdown
---
name: code-review
description: Code review specialist — reviews code for quality
metadata:
  category: development
hooks:
  after_apply:
    - type: shell
      command: "echo review done"
      timeout_secs: 30
---

When reviewing code, evaluate against:
1. Correctness
2. Safety & robustness
3. ...
```

### Ubicaciones

| Ámbito | Ruta |
|---|---|
| Proyecto | `.agents/skills/<nombre>/SKILL.md` |
| Global | `$HOME/.agents/skills/<nombre>/SKILL.md` |
| Absoluta | Cualquier ruta absoluta en el sistema |

### Skills instaladas en el proyecto

| Skill | Descripción |
|---|---|
| `shell` | Ejecutar comandos shell en el workspace |
| `filesystem` | Operaciones atómicas (read/write/edit/list/delete) |
| `web-research` | Buscar en la web y obtener documentación |
| `code-review` | Revisar código (calidad, corrección, estándares) |
| `rust-dev` | Escribir, compilar, testear y depurar Rust |
| `find-skills` | Buscar skills instaladas localmente y en skills.sh |
| `skill-creator` | Crear, modificar y optimizar skills |
| `agent-creator` | Crear y gestionar agentes, subagentes, skills y MCPs |
| `planning` | Planificación estructurada (WBS, Milestones, Agile) |
| `version-control` | Git, Conventional Commits, GitHub workflows |
| `tool-discovery` | Audita y recomienda skills/MCPs/subagentes para una tarea |
| `weather` | Consulta meteorológica para cualquier localidad |

### Cómo listar skills de un agente

`/skills` o en el panel lateral Skills.

### Cómo añadir una skill a un agente

1. Añade la ruta en el frontmatter del agente: `skills: [.agents/skills/mi-skill/]`
2. Ejecuta `/reload`

---

## 6. MCPs (Model Context Protocol)

### ¿Qué es un MCP?

MCP es un protocolo JSON-RPC 2.0 que permite a los agentes usar herramientas y recursos externos. Los servidores MCP pueden ejecutarse como:

- **Procesos hijo** (transporte stdio) — el método más común
- **Servicios TCP** (transporte tcp) — para servidores remotos

### Definición en config.yaml

```yaml
mcps:
  codegraph:
    transport: stdio
    command: "/usr/local/bin/codegraph"
    args: ["mcp"]

  postgres:
    transport: tcp
    host: "localhost"
    port: 5432
```

### Cómo se asignan a los agentes

En el frontmatter del agente:

```yaml
mcps: [codegraph]
```

### Cómo gestionar MCPs en caliente

| Comando | Descripción |
|---|---|
| `/mcps` | Listar servidores MCP y su estado |
| `/mcps codegraph on` | Activar servidor MCP |
| `/mcps codegraph off` | Desactivar servidor MCP |

---

## 7. Permisos

### Tipos de permiso

| Permiso | Descripción |
|---|---|
| `fs.read` | Leer archivos |
| `fs.write` | Escribir archivos |
| `fs.external` | Acceso fuera del workspace (opt-in explícito) |
| `net.http` | Hacer peticiones HTTP |
| `command.run` | Ejecutar comandos shell |
| `mcp.use` | Usar herramientas MCP |
| `env.read` | Leer variables de entorno |
| `skill.use` | Invocar skills |

### Modelo de permisos

| Regla | Comportamiento |
|---|---|
| **Allow by default** | Todo lo no denegado explícitamente está permitido |
| **Deny explícito** | `deny: ["command.run"]` bloquea ese permiso |
| **Allow explícito** | Si usas `allow`, esos permisos pasan a deny-by-default |
| **FsExternal es opt-in** | Debe estar explícitamente en `allow` |

### Ejemplos de configuración

```yaml
# Agente root — permisivo pero seguro
permissions:
  deny:
    - command.run.sudo
    - net.http.delete

# Subagente reviewer — solo lectura
permissions:
  deny:
    - command.run
    - net.http
    - fs.write

# Agente con acceso externo
permissions:
  allow:
    - fs.external
```

### Aprobación humana

Ciertas operaciones sensibles REQUIEREN confirmación explícita en la TUI antes de ejecutarse:

- Comandos con `sudo`
- `rm -rf`
- `chmod`
- Operaciones en `/boot/`

Aparecerá un diálogo de aprobación que debes aceptar o rechazar.

---

## 8. Sesiones

### ¿Qué es una sesión?

Una sesión captura toda la conversación con Anacleto: mensajes del usuario, respuestas del agente, resultados de tools, etc. Todo se persiste en SQLite.

### Gestión de sesiones

| Comando | Descripción |
|---|---|
| `/sessions` o `/s` | Listar todas las sesiones |
| `/new` | Crear nueva sesión |
| `/resume <id>` o `/r <id>` | Reanudar sesión existente |
| `/delete <id>` o `/d <id>` | Eliminar una sesión |
| `/rename <id> <nombre>` | Renombrar una sesión |
| `/undo` | Deshacer el último par de mensajes |
| `/redo` | Rehacer el último undo |
| `/fork` | Bifurcar (fork) la sesión activa en una nueva |
| `/parent` | Navegar a la sesión padre |
| `/children` | Listar sesiones hijas |
| `/export` | Exportar sesión a archivo (JSON/Markdown) |
| `/import` | Importar sesión desde archivo |
| `/share` | Marcar sesión como compartida |
| `/unshare` | Desmarcar sesión como compartida |

### Snapshot del sistema

Puedes capturar el estado completo de una sesión y revertir a él:

| Comando | Descripción |
|---|---|
| `/snapshot` | Crear un snapshot de la sesión |
| `/snapshots` | Listar snapshots disponibles |
| `/revert` | Revertir la sesión a un snapshot |
| `/stage` | Escenario (stage) la conversación como snapshot pendiente |
| `/clear` | Limpiar el staged snapshot |
| `/commit` | Confirmar el staged snapshot |

### Límite de contexto

Anacleto gestiona automáticamente el context window del LLM:

- Usa el 50% del context window para el historial de la conversación
- Cuando se supera el 80%, dispara compactación **automática**
- La compactación usa el propio LLM para generar un resumen estructurado
- También puedes compactar manualmente con `/compact` o `/c`

---

## 9. Comandos TUI

### Comandos slash

| Comando | Alias | Descripción |
|---|---|---|
| `/help` | `/h` | Mostrar ayuda |
| `/sessions` | `/s` | Listar sesiones |
| `/new` | — | Nueva sesión |
| `/resume <id>` | `/r` | Reanudar sesión |
| `/delete <id>` | `/d` | Eliminar sesión |
| `/rename <id> <nombre>` | — | Renombrar sesión |
| `/reload` | `/rl` | Recargar agente activo (config + skills) |
| `/agents` | `/a` | Listar agentes |
| `/agent <nombre>` | — | Cambiar agente activo |
| `/subagents` | `/sa` | Listar subagentes |
| `/copy` | — | Copiar chat al portapapeles |
| `/export-editor` | `/ee` | Exportar chat a editor externo |
| `/compact` | `/c` | Compactar conversación |
| `/debug` | — | Activar/desactivar modo debug |
| `/models` | — | Listar modelos disponibles |
| `/exit` | `/quit` | Salir |
| `/undo` | — | Deshacer último mensaje |
| `/redo` | — | Rehacer último undo |
| `/fork` | — | Bifurcar sesión |
| `/export` | — | Exportar sesión |
| `/import` | — | Importar sesión |
| `/share` | — | Compartir sesión |
| `/unshare` | — | Descompartir sesión |
| `/skills` | — | Listar skills del agente activo |
| `/mcps` | — | Listar y toggle MCPs |
| `/status` | — | Mostrar estado del engine |
| `/init` | — | Configuración guiada de AGENTS.md |
| `/review` | — | Revisar cambios git |
| `/warp` | — | Cambiar directorio de trabajo |
| `/workspaces` | — | Listar workspaces |
| `/move` | — | Mover sesión a otro workspace |
| `/worktree` | — | Gestionar git worktrees |
| `/timeline` | — | Mostrar timeline de la sesión |
| `/themes` | — | Cambiar tema de color |
| `/timestamps` | — | Activar/desactivar timestamps |
| `/thinking` | — | Mostrar/ocultar razonamiento LLM |
| `/stash` | — | Guardar prompt actual |
| `/editor` | — | Abrir editor externo |
| `/build` | — | Cambiar a modo construcción |
| `/jobs` | — | Listar trabajos background |
| `/parent` | — | Ir a sesión padre |
| `/children` | — | Listar sesiones hijas |
| `/snapshot` | — | Crear snapshot |
| `/snapshots` | — | Listar snapshots |
| `/revert` | — | Revertir a snapshot |
| `/stage` | — | Staged snapshot |
| `/clear` | — | Limpiar staged |
| `/commit` | — | Confirmar staged |

### Command palette

Pulsa `Ctrl+P` para abrir la paleta de comandos. Empieza a escribir para filtrar con búsqueda difusa (fuzzy matching).

### Modos de agente

| Modo | Descripción |
|---|---|
| **Normal** | Modo por defecto. El agente piensa y ejecuta herramientas |
| **Plan** | Modo planificación. Solo analiza y planifica, NO ejecuta |
| **Build** | Modo construcción. Ejecuta el plan generado |

Para cambiar: usa `/plan` (entra en modo plan) y `/build` (entra en modo build).

---

## 10. Atajos de teclado y navegación

### Navegación principal

| Tecla | Acción |
|---|---|
| `Enter` | Enviar mensaje |
| `Escape` | Cancelar / cerrar diálogo |
| `Ctrl+C` | Cancelar operación actual |
| `Ctrl+D` | Salir |
| `Up/Down` | Navegar historial de input |
| `Tab` | Autocompletar comando |

### Paneles

| Tecla | Acción |
|---|---|
| `Ctrl+P` | Abrir paleta de comandos |
| `Ctrl+F` | Buscar en la conversación |
| `Ctrl+[` | Foco al panel de Skills |
| `Ctrl+]` | Foco al panel de MCPs |
| `Ctrl+Shift+[` | Foco al panel de SubAgents |
| `Ctrl+Shift+]` | Foco al panel de Agents |

Los paneles laterales son navegables con las flechas del teclado cuando tienen el foco.

### Which-key popup

Pulsa `?` para abrir el popup de atajos de teclado (which-key). Muestra todos los atajos disponibles en el contexto actual.

### Keymap configurable

Puedes personalizar los atajos en `config.yaml`:

```yaml
keymap:
  keys:
    focus_input: "escape i"
    focus_chat: "escape c"
    toggle_sidebar: "escape s"
    command_palette: "ctrl+p"
    submit: "enter"
    search: "ctrl+f"
```

---

## 11. Modo debug

El modo debug muestra los payloads completos de las peticiones y respuestas LLM, útil para depurar problemas.

| Método | Descripción |
|---|---|
| `--debug` | Activar al arrancar |
| `/debug` | Activar/desactivar en caliente |
| `-v` | Modo verbose (logs más detallados) |

Los logs se guardan en:
- `~/.local/share/anacleto/logs/anacleto.log` (rotación diaria)
- También se muestran en stdout

---

## 12. Modo headless

El modo headless permite ejecutar Anacleto sin TUI, útil para scripting y automatización:

```sh
# Una consulta rápida
anacleto --headless --task "¿Qué versión de Rust tengo?"

# Con configuración específica
anacleto --headless --config .agents/config.yaml --task "Analiza este proyecto"

# Con output detallado
anacleto --headless --verbose --task "Haz una revisión de código"
```

---

## 13. Hooks y automatización

Los hooks son comandos shell que se ejecutan automáticamente en puntos específicos del ciclo de vida:

### Hook points disponibles

| Hook Point | Cuándo se dispara |
|---|---|
| `before_tool` | Antes de ejecutar cualquier tool |
| `after_tool` | Después de cualquier tool (éxito) |
| `before_apply` | Antes de apply_patch |
| `after_apply` | Después de apply_patch (éxito) |
| `before_shell` | Antes de comando shell |
| `after_shell` | Después de comando shell (éxito) |
| `before_fs_write` | Antes de write/edit/delete |
| `after_fs_write` | Después de write/edit/delete (éxito) |
| `on_startup` | Arranque del engine |
| `on_shutdown` | Parada del engine |

### Configuración

```yaml
hooks:
  after_apply:
    - type: shell
      command: "codegraph sync"
      timeout_secs: 60
  on_startup:
    - type: shell
      command: "echo 'Anacleto started'"
      timeout_secs: 10
```

### Auto-detección

Anacleto detecta automáticamente herramientas conocidas en PATH (como `codegraph`) y registra hooks apropiados.

---

## 14. Plugins

Los plugins extienden Anacleto con hooks y transforms. Un plugin puede:

- Observar/modificar el spawn de agentes
- Interceptar tool calls
- Manejar comandos slash personalizados
- Reaccionar a eventos del engine
- Registrar herramientas personalizadas

Los plugins se cargan desde `~/.config/anacleto/plugins/`. Cada plugin es un directorio con un `plugin.yaml` manifest.

---

## 15. Solución de problemas

### No arranca

```sh
# Verifica la sintaxis YAML
anacleto -v

# Comprueba que las API keys están configuradas
echo $ANTHROPIC_API_KEY
echo $OPENROUTER_API_KEY
```

### Errores de conexión LLM

- Verifica que las API keys son válidas
- Comprueba el `base_url` para OpenRouter/Ollama
- Prueba con `anacleto --debug` para ver los payloads

### No encuentra agentes

```sh
# Verifica que existe .agents/ directory
ls -la .agents/agents/

# Verifica el formato del frontmatter
head -5 .agents/agents/root.md

# Recarga
ejecuta /reload en la TUI
```

### La sesión no se guarda

```sh
# Verifica la ruta de la base de datos
ls -la ~/.local/share/anacleto/

# O usa una ruta explícita
anacleto -d /tmp/test.db
```

### Rendimiento

- Usa modelos locales (Ollama) para tareas simples
- Compacta manualmente con `/compact` si la conversación es muy larga
- Ajusta `session.history_limit_percent` para usar menos contexto

---

*Documentación de usuario actualizada a partir del estudio exhaustivo del código fuente. Agosto 2026.*
