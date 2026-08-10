# Migrar Input a ratatui-textarea

> **Para workers agentic:** Este plan se ejecuta task por task. Usar checkboxes (`- [ ]`) para tracking.

**Goal:** Reemplazar la gestión manual del buffer de texto, cursor, wrapping y renderizado del Input de Anacleto por el widget `TextArea` de `ratatui-textarea`, eliminando ~280 líneas de código manual.

**Architecture:** `ratatui-textarea` 0.9.2 es un widget especializado que maneja internamente el buffer de texto (`Vec<String>`), cursor, selección, scroll, y renderizado. Se integra con el sistema de keymaps existente mediante `input_without_shortcuts()` y métodos individuales (`insert_char`, `move_cursor`, `delete_char`, etc.).

**Tech Stack:** Rust 1.86+, ratatui 0.30.2 (meta-crate que ya depende de `ratatui-core` 0.1.x y `ratatui-widgets` 0.3.x), ratatui-textarea 0.9.2.

## Hallazgos de la investigación

1. **Compatibilidad:** `ratatui` 0.30.2 ya es un meta-crate que depende internamente de `ratatui-core` 0.1.2 y `ratatui-widgets` 0.3.2. `ratatui-textarea` 0.9.2 depende de `ratatui-core` ^0.1.1 y `ratatui-widgets` ^0.3.1. Son compatibles — cargo resolverá las versiones compartidas.

2. **MSRV:** `ratatui-textarea` 0.9.2 requiere Rust 1.86.0. El proyecto tiene `rust-version = "1.85"`. Hay que bumpearlo a `"1.86"`. El rustc instalado es 1.97.0, no hay problema.

3. **API clave de TextArea:**
   - `TextArea::default()` / `TextArea::new(lines: Vec<String>)` / `TextArea::from([&str])`
   - `textarea.input(key_event)` — manejo automático con shortcuts por defecto
   - `textarea.input_without_shortcuts(key_event)` — solo insert/delete básico
   - `textarea.insert_char(c)`, `textarea.insert_newline()`
   - `textarea.delete_char()`, `textarea.delete_next_char()`
   - `textarea.delete_word()`, `textarea.delete_next_word()`
   - `textarea.delete_line_by_head()`, `textarea.delete_line_by_end()`
   - `textarea.move_cursor(CursorMove::...)` — `Forward`, `Back`, `WordForward`, `WordBack`, `Head`, `End`, `Top`, `Bottom`
   - `textarea.lines()` → `&[String]` (para obtener el texto)
   - `textarea.set_placeholder(placeholder)` — para el prompt "❯"
   - `textarea.set_block(block)` — para bordes y título
   - `textarea.set_cursor_style(style)` — estilo del cursor
   - `f.render_widget(&textarea, area)` — renderizado directo
   - `textarea.scroll(Scrolling::...)` — control de scroll

4. **Touch points en el código:**
   - `Cargo.toml` — añadir dependencia, subir MSRV
   - `src/tui/app.rs` — reemplazar `input: String` + `input_cursor: usize` por `textarea: TextArea<'static>`
   - `src/tui/input.rs` — reescribir `handle_input_key` para usar métodos de TextArea
   - `src/tui/render.rs` — reemplazar `render_input` (Paragraph + wrapping manual + cursor manual) por `f.render_widget(&textarea, area)`
   - `src/tui/commands.rs` — actualizar referencias a `app.input` → `app.textarea.lines()`
   - `src/tui/navigation.rs` — actualizar referencias a `app.input`
   - `src/tui/keys.rs` — actualizar referencias a `app.input`
   - Tests en `app.rs`, `input.rs`, `navigation.rs`, `events.rs` — actualizar

## Global Constraints

- Rust edition 2024, MSRV 1.86
- Usar `input_without_shortcuts()` para mantener el sistema de keymaps existente
- No perder funcionalidad: Tab completion, input history, paletas, command palette
- `cargo fmt --check && cargo clippy && cargo test` debe pasar antes de commitear

---

### Task 1: Añadir dependencia y subir MSRV

**Files:**
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: nada
- Produce: dependencia `ratatui-textarea` disponible, MSRV actualizada

- [ ] **Step 1: Editar Cargo.toml**

```toml
# En [package], cambiar:
rust-version = "1.85"
# a:
rust-version = "1.86"

# En [dependencies], añadir tras la línea de ratatui:
ratatui-textarea = "0.9.2"
```

- [ ] **Step 2: Verificar que resuelve**

Run: `cargo check 2>&1 | head -20`
Expected: éxito (puede que falle por código aún no actualizado, pero la resolución de dependencias debe funcionar)

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build: add ratatui-textarea 0.9.2 dependency, bump MSRV to 1.86"
```

---

### Task 2: Reemplazar `input`/`input_cursor` por `TextArea` en `App`

**Files:**
- Modify: `src/tui/app.rs`

**Interfaces:**
- Consumes: `ratatui_textarea::TextArea`
- Produces: `app.textarea: TextArea<'static>` (reemplaza `app.input` y `app.input_cursor`)

- [ ] **Step 1: Añadir import y reemplazar campos**

En `src/tui/app.rs`, añadir al inicio:
```rust
use ratatui_textarea::TextArea;
```

Reemplazar los campos:
```rust
// ANTES:
/// Current user input buffer.
pub input: String,
/// Character index of the cursor within `input` (for shell-style editing).
pub(crate) input_cursor: usize,

// DESPUÉS:
/// TextArea widget state (buffer, cursor, selection).
pub(crate) textarea: TextArea<'static>,
```

- [ ] **Step 2: Actualizar el constructor `App::new()`**

Reemplazar:
```rust
input: String::new(),
input_cursor: 0,
```
por:
```rust
textarea: {
    let mut ta = TextArea::default();
    ta.set_placeholder(" ❯ ");
    ta.set_cursor_style(Style::default().add_modifier(Modifier::REVERSED));
    ta
},
```

Añadir el import necesario para `Style` y `Modifier` si no está ya en `app.rs`:
```rust
use ratatui::style::{Modifier, Style};
```

- [ ] **Step 3: Commit**

```bash
git add src/tui/app.rs
git commit -m "refactor(input): replace input/input_cursor with TextArea widget"
```

---

### Task 3: Reescribir `handle_input_key` para usar TextArea

**Files:**
- Modify: `src/tui/input.rs`

**Interfaces:**
- Consumes: `app.textarea: TextArea<'static>` (con métodos `insert_char`, `move_cursor`, `delete_char`, etc.)
- Produce: `handle_input_key` delegando en TextArea

- [ ] **Step 1: Reescribir `handle_input_key`**

Reemplazar TODO el contenido de `src/tui/input.rs`:

```rust
//! Input-box key handling using ratatui-textarea's TextArea widget.
//!
//! Contains the `App` methods that handle keys while the Input window has
//! focus, delegating text editing to `TextArea` and handling custom actions
//! (Tab completion, history, palettes) on top.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui_textarea::CursorMove;

use super::app::App;
use super::render::shift_char;
use crate::tui::keymap::Action;

impl App {
    /// Handle a key while the Input window (1) has focus.
    pub(crate) fn handle_input_key(
        &mut self,
        key: KeyCode,
        modifiers: KeyModifiers,
        key_event: KeyEvent,
    ) {
        if self.keymap.matches(key_event, Action::TabComplete) {
            // Reset matches if the input has changed since last Tab
            if !self.textarea.lines().first().map_or(true, |l| !l.starts_with('/')) {
                return;
            }
            let current_text = self.textarea.lines().join("\n");
            let prefix = current_text.to_lowercase();
            if self.tab_index == 0 || self.tab_matches.is_empty() {
                self.tab_matches = self
                    .commands
                    .iter()
                    .filter(|(c, _)| c.starts_with(&prefix))
                    .map(|(c, _)| c.clone())
                    .collect();
            }
            if self.tab_matches.is_empty() {
                return;
            }
            let idx = self.tab_index % self.tab_matches.len();
            let completed = self.tab_matches[idx].clone();
            // Replace textarea content with the completed command
            self.textarea = ratatui_textarea::TextArea::from([completed.as_str()]);
            self.tab_index += 1;
        } else if self.keymap.matches(key_event, Action::InsertNewline) {
            self.reset_tab_state();
            self.textarea.insert_newline();
        } else if self.keymap.matches(key_event, Action::ClearInput) {
            self.reset_tab_state();
            self.textarea.delete_line_by_head();
            while self.textarea.move_cursor(CursorMove::Back) {}
            self.textarea.delete_line_by_end();
        } else if self.keymap.matches(key_event, Action::DeleteToStart) {
            self.reset_tab_state();
            self.textarea.delete_line_by_head();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::DeleteWordBefore) {
            self.reset_tab_state();
            self.textarea.delete_word();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::DeleteToEnd) {
            self.reset_tab_state();
            self.textarea.delete_line_by_end();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::CursorHome) {
            self.reset_tab_state();
            self.textarea.move_cursor(CursorMove::Head);
        } else if self.keymap.matches(key_event, Action::CursorEnd) {
            self.reset_tab_state();
            self.textarea.move_cursor(CursorMove::End);
        } else if self.keymap.matches(key_event, Action::CursorWordLeft) {
            self.reset_tab_state();
            self.textarea.move_cursor(CursorMove::WordBack);
        } else if self.keymap.matches(key_event, Action::CursorWordRight) {
            self.reset_tab_state();
            self.textarea.move_cursor(CursorMove::WordForward);
        } else if self.keymap.matches(key_event, Action::CursorLeft) {
            self.textarea.move_cursor(CursorMove::Back);
        } else if self.keymap.matches(key_event, Action::CursorRight) {
            self.textarea.move_cursor(CursorMove::Forward);
        } else if self.keymap.matches(key_event, Action::DeleteChar) {
            self.textarea.delete_next_char();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::DeleteCharBefore) {
            self.tab_matches.clear();
            self.tab_index = 0;
            self.textarea.delete_char();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::HistoryUp) {
            if self.show_model_palette && !self.model_matches.is_empty() {
                self.model_index = self
                    .model_index
                    .saturating_sub(1)
                    .min(self.model_matches.len() - 1);
            } else if self.show_agent_palette && !self.agent_matches.is_empty() {
                self.agent_index = self
                    .agent_index
                    .saturating_sub(1)
                    .min(self.agent_matches.len() - 1);
            } else if self.show_command_palette && !self.palette_matches.is_empty() {
                self.palette_index = self
                    .palette_index
                    .saturating_sub(1)
                    .min(self.palette_matches.len() - 1);
            } else if !self.input_history.is_empty() {
                // Navigate backwards through input history.
                let next = match self.history_index {
                    Some(i) if i > 0 => i - 1,
                    Some(_) => 0,
                    None => self.input_history.len() - 1,
                };
                self.history_index = Some(next);
                self.textarea = TextArea::from([self.input_history[next].as_str()]);
                self.tab_matches.clear();
                self.tab_index = 0;
            }
        } else if self.keymap.matches(key_event, Action::HistoryDown) {
            if self.show_model_palette && !self.model_matches.is_empty() {
                self.model_index = (self.model_index + 1) % self.model_matches.len();
            } else if self.show_agent_palette && !self.agent_matches.is_empty() {
                self.agent_index = (self.agent_index + 1) % self.agent_matches.len();
            } else if self.show_command_palette && !self.palette_matches.is_empty() {
                self.palette_index = (self.palette_index + 1) % self.palette_matches.len();
            } else if self.history_index.is_some() {
                // Navigate forwards through input history; past the newest returns to empty.
                match self.history_index {
                    Some(i) if i + 1 < self.input_history.len() => {
                        self.history_index = Some(i + 1);
                        self.textarea = TextArea::from([self.input_history[i + 1].as_str()]);
                    }
                    _ => {
                        self.history_index = None;
                        self.textarea = TextArea::default();
                    }
                }
                self.tab_matches.clear();
                self.tab_index = 0;
            }
        } else if self.keymap.matches(key_event, Action::Send) {
            self.tab_matches.clear();
            self.tab_index = 0;
            if self.show_model_palette && !self.model_matches.is_empty() {
                // Execute `/models <selected>` from the model combo.
                let name = self.model_matches[self.model_index].clone();
                self.show_model_palette = false;
                self.model_matches.clear();
                self.model_index = 0;
                self.textarea = TextArea::default();
                self.handle_command(format!("/models {}", name));
            } else if self.show_agent_palette && !self.agent_matches.is_empty() {
                // Execute `/agent <selected>` from the agent combo.
                let name = self.agent_matches[self.agent_index].clone();
                self.show_agent_palette = false;
                self.agent_matches.clear();
                self.agent_index = 0;
                self.textarea = TextArea::default();
                self.handle_command(format!("/agent {}", name));
            } else if self.show_command_palette && !self.palette_matches.is_empty() {
                // Execute the highlighted command from the palette.
                let idx = self.palette_matches[self.palette_index];
                let cmd = self.commands[idx].0.clone();
                self.show_command_palette = false;
                self.palette_matches.clear();
                self.palette_index = 0;
                self.textarea = TextArea::default();
                self.handle_command(cmd);
            } else {
                let input = self.textarea.lines().join("\n");
                self.textarea = TextArea::default();
                if !input.is_empty() {
                    // Record in input history (dedupe consecutive repeats).
                    if self.input_history.last() != Some(&input) {
                        self.input_history.push(input.clone());
                    }
                    self.history_index = None;
                    // Auto-scroll al final al enviar
                    self.chat_scroll = 0;
                    self.process_input(input);
                }
            }
        } else if self.keymap.matches(key_event, Action::CancelInput) {
            // Any non-Tab key resets autocomplete state
            self.tab_matches.clear();
            self.tab_index = 0;
            // Close the command palette first, then other overlays
            if self.show_model_palette {
                self.show_model_palette = false;
                self.model_matches.clear();
                self.model_index = 0;
            } else if self.show_agent_palette {
                self.show_agent_palette = false;
                self.agent_matches.clear();
                self.agent_index = 0;
            } else if self.show_command_palette {
                self.show_command_palette = false;
                self.palette_matches.clear();
                self.palette_index = 0;
            } else if self.show_session_list {
                self.show_session_list = false;
            } else if self.show_agents {
                self.show_agents = false;
            } else if self.show_subagents {
                self.show_subagents = false;
            } else {
                // No overlay open — clear input
                self.textarea = TextArea::default();
            }
        } else if let KeyCode::Char(c) = key {
            // Any non-Tab key resets autocomplete state
            self.tab_matches.clear();
            self.tab_index = 0;
            if self.kb_supported && modifiers.contains(KeyModifiers::SHIFT) {
                // Kitty protocol: shift is reported as a modifier;
                // apply keyboard-appropriate shift mapping
                self.textarea.insert_char(shift_char(c, &self.lang));
            } else {
                self.textarea.insert_char(c);
            }
            self.update_command_palette();
        }
    }

    /// Reset the Tab-completion autocomplete state.
    pub(crate) fn reset_tab_state(&mut self) {
        self.tab_matches.clear();
        self.tab_index = 0;
    }
}
```

- [ ] **Step 2: Verificar que compila**

Run: `cargo check 2>&1 | head -30`
Expected: puede que fallen otros archivos que referencian `app.input` / `app.input_cursor`, pero `input.rs` debe compilar

- [ ] **Step 3: Commit**

```bash
git add src/tui/input.rs
git commit -m "refactor(input): rewrite handle_input_key to use TextArea methods"
```

---

### Task 4: Reescribir `render_input` para usar TextArea

**Files:**
- Modify: `src/tui/render.rs`

**Interfaces:**
- Consumes: `app.textarea: TextArea<'static>` (renderizable directamente)
- Produce: `render_input` simplificado, eliminando wrapping manual y cursor

- [ ] **Step 1: Añadir import**

En `src/tui/render.rs`, añadir entre los imports:
```rust
use ratatui_textarea::TextArea;
```

- [ ] **Step 2: Reemplazar la función `render_input`**

Reemplazar TODO el cuerpo de `render_input` (desde `fn render_input` hasta justo antes de `fn render_approval_dialog`):

```rust
/// Render the input box at the bottom of the screen.
fn render_input(f: &mut Frame, area: Rect, app: &App) {
    let title = if let Some(flow) = &app.init_flow {
        format!(" Init — {} ", flow.prompt())
    } else {
        " [1] Input ".to_string()
    };
    let input_border = if app.focus == Focus::Input {
        app.theme.accent()
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(input_border))
        .title(title);

    // Clone the textarea to set the block (we need to mutate it).
    let mut textarea = app.textarea.clone();
    textarea.set_block(block);
    f.render_widget(&textarea, area);
}
```

- [ ] **Step 3: Eliminar imports no usados**

Si después del cambio `Paragraph`, `Wrap`, `UnicodeWidthStr` ya no se usan en el archivo, eliminarlos de los imports. Verificar con `cargo check`.

- [ ] **Step 4: Verificar que compila**

Run: `cargo check 2>&1 | head -30`
Expected: éxito en render.rs (pueden quedar errores en otros archivos)

- [ ] **Step 5: Commit**

```bash
git add src/tui/render.rs
git commit -m "refactor(input): replace manual Paragraph rendering with TextArea widget"
```

---

### Task 5: Actualizar referencias a `app.input` en commands y navigation

**Files:**
- Modify: `src/tui/commands.rs`
- Modify: `src/tui/navigation.rs`
- Modify: `src/tui/keys.rs`

**Interfaces:**
- Consumes: `app.textarea.lines()` → `&[String]`, `app.textarea = TextArea::from(...)` para reemplazar contenido
- Produce: todas las referencias a `app.input` y `app.input_cursor` actualizadas

- [ ] **Step 1: Actualizar `src/tui/commands.rs`**

Buscar y reemplazar todos los patrones:

| Patrón antiguo | Reemplazo |
|---|---|
| `self.input.clone()` | `self.textarea.lines().join("\n")` |
| `self.input.clear()` | `self.textarea = TextArea::default()` |
| `self.input = String::from(...)` | `self.textarea = TextArea::from([...])` |
| `self.input = contents...` | `self.textarea = TextArea::from([contents.as_str()])` |
| `self.input.trim().is_empty()` | `self.textarea.lines().join("\n").trim().is_empty()` |
| `self.input_cursor = ...` | (eliminar, TextArea gestiona el cursor) |
| `std::mem::take(&mut self.input)` | `self.textarea.lines().join("\n")` seguido de `self.textarea = TextArea::default()` |

Casos específicos en commands.rs:
- Línea 441: `self.input = saved;` → `self.textarea = TextArea::from([saved.as_str()]);`
- Línea 442: `self.input_cursor = self.input.chars().count();` → eliminar
- Línea 459: `if self.input.trim().is_empty()` → `if self.textarea.lines().join("\n").trim().is_empty()`
- Línea 462: `self.stash_stack.push(self.input.clone());` → `self.stash_stack.push(self.textarea.lines().join("\n"));`
- Línea 463: `self.input.clear();` → `self.textarea = TextArea::default();`
- Línea 464: `self.input_cursor = 0;` → eliminar
- Línea 529: `if std::fs::write(&tmp, &self.input).is_err()` → `if std::fs::write(&tmp, self.textarea.lines().join("\n")).is_err()`
- Línea 535: `self.input = contents.trim_end_matches('\n').to_string();` → `self.textarea = TextArea::from([contents.trim_end_matches('\n')]);`
- Línea 536: `self.input_cursor = self.input.chars().count();` → eliminar
- Línea 588: `let answer = std::mem::take(&mut self.input);` → `let answer = self.textarea.lines().join("\n"); self.textarea = TextArea::default();`

- [ ] **Step 2: Actualizar `src/tui/navigation.rs`**

Buscar y reemplazar:
- Línea 138: `self.input = prompt.clone();` → `self.textarea = TextArea::from([prompt.as_str()]);`
- Línea 139: `self.input_cursor = self.input.chars().count();` → eliminar

- [ ] **Step 3: Actualizar `src/tui/keys.rs`**

Buscar referencias a `self.input` y `self.input_cursor` en keys.rs y actualizar con el mismo patrón.

- [ ] **Step 4: Verificar que compila**

Run: `cargo check 2>&1`
Expected: sin errores (si quedan, buscarlos con `cargo check 2>&1 | grep "error"`)

- [ ] **Step 5: Commit**

```bash
git add src/tui/commands.rs src/tui/navigation.rs src/tui/keys.rs
git commit -m "refactor(input): update all app.input references to use textarea"
```

---

### Task 6: Actualizar tests

**Files:**
- Modify: `src/tui/app.rs` (tests)
- Modify: `src/tui/navigation.rs` (tests)
- Modify: `src/tui/events.rs` (tests)
- Modify: `src/tui/input.rs` (tests, si los hay)

**Interfaces:**
- Consumes: tests que usan `app.input` y `app.input_cursor`
- Produce: tests actualizados que usan `app.textarea.lines()` y `app.textarea.cursor()` (o similar)

- [ ] **Step 1: Actualizar tests en `app.rs`**

Buscar el bloque `#[cfg(test)] mod tests { ... }` en `app.rs` y reemplazar:

```rust
// ANTES:
app.input = String::from("hola");
app.input_cursor = 2;
app.input_insert_char('X');
assert_eq!(app.input, "hoXla");
assert_eq!(app.input_cursor, 3);

// DESPUÉS:
app.textarea = TextArea::from(["hola"]);
// Mover cursor a posición 2
for _ in 0..2 { app.textarea.move_cursor(CursorMove::Forward); }
app.textarea.insert_char('X');
assert_eq!(app.textarea.lines().join("\n"), "hoXla");
```

Para tests de cursor, usar `app.textarea.cursor()` si está disponible, o verificar el contenido.

Nota: `TextArea::cursor()` devuelve `(row, col)`. Si no está disponible en la API pública, podemos verificar solo el contenido.

Reemplazar sistemáticamente:
- `app.input = String::from(...)` → `app.textarea = TextArea::from([...])`
- `app.input_cursor = N` → bucles de `move_cursor(CursorMove::Forward)` o `move_cursor(CursorMove::Back)`
- `assert_eq!(app.input, ...)` → `assert_eq!(app.textarea.lines().join("\n"), ...)`
- `assert_eq!(app.input_cursor, N)` → verificar con `app.textarea.cursor()` si existe, o eliminar aserciones de cursor

- [ ] **Step 2: Actualizar tests en `navigation.rs`**

Reemplazar:
```rust
// ANTES:
assert_eq!(app.input, "second");
assert_eq!(app.input_cursor, 6);

// DESPUÉS:
assert_eq!(app.textarea.lines().join("\n"), "second");
```

- [ ] **Step 3: Verificar que los tests compilan y pasan**

Run: `cargo test 2>&1 | tail -30`
Expected: todos los tests pasan

- [ ] **Step 4: Commit**

```bash
git add src/tui/app.rs src/tui/navigation.rs src/tui/events.rs
git commit -m "test(input): update tests for TextArea migration"
```

---

### Task 7: Limpiar código muerto y verificación final

**Files:**
- Modify: `src/tui/input.rs` (eliminar métodos que ya no se usan)
- Modify: `src/tui/render.rs` (eliminar `VisRow`, `shift_char` si ya no se usa, etc.)

- [ ] **Step 1: Eliminar métodos manuales no usados**

De `src/tui/input.rs`, eliminar estos métodos que TextArea ahora gestiona:
- `input_char_to_byte`
- `input_insert_char`
- `input_delete_before`
- `input_delete_at`
- `input_move_word_left`
- `input_move_word_right`
- `input_delete_word_before`
- `input_delete_to_start`
- `input_delete_to_end`

- [ ] **Step 2: Eliminar struct `VisRow` y lógica de cursor manual**

De `src/tui/render.rs`, eliminar:
- El struct `VisRow`
- Todo el bloque de construcción de `vis_rows`
- Todo el bloque de posicionamiento de cursor (`f.set_cursor_position(...)`)
- La variable `rendered` y su lógica de construcción
- La variable `scroll_offset`

- [ ] **Step 3: Verificar que no quedan referencias a `input_cursor`**

Run: `grep -rn "input_cursor" src/`
Expected: 0 resultados

- [ ] **Step 4: Verificar que no quedan referencias a métodos antiguos**

Run: `grep -rn "input_insert_char\|input_delete_before\|input_delete_at\|input_move_word\|input_delete_word\|input_delete_to\|input_char_to_byte" src/`
Expected: 0 resultados

- [ ] **Step 5: Formatear y lint**

```bash
cargo fmt --check && cargo clippy 2>&1
```
Si hay errores de fmt: `cargo fmt`
Si hay warnings de clippy: corregirlos

- [ ] **Step 6: Tests finales**

```bash
cargo test 2>&1
```
Expected: all tests pass

- [ ] **Step 7: Commit final**

```bash
git add -A
git commit -m "refactor(input): remove dead code from manual input handling"
```

---

## Resumen de cambios

| Archivo | Líneas eliminadas | Líneas añadidas | Neto |
|---|---|---|---|
| `Cargo.toml` | 0 | 2 | +2 |
| `src/tui/app.rs` | 4 | ~15 | +11 |
| `src/tui/input.rs` | ~337 | ~200 | -137 |
| `src/tui/render.rs` | ~150 | ~20 | -130 |
| `src/tui/commands.rs` | ~10 | ~15 | +5 |
| `src/tui/navigation.rs` | ~4 | ~4 | 0 |
| `src/tui/keys.rs` | ~2 | ~2 | 0 |
| **Total** | **~507** | **~258** | **~-249** |

## Verificación post-migración

1. ✅ Escribir texto en el Input (caracteres normales, Unicode, emojis)
2. ✅ Navegación: Left/Right, Ctrl+Left/Right (word), Home/End
3. ✅ Borrado: Backspace, Delete, Ctrl+W (word), Ctrl+U (to start), Ctrl+K (to end)
4. ✅ Multi-línea: Alt+Enter / Ctrl+Enter / Shift+Enter
5. ✅ History: Up/Down arrow
6. ✅ Tab completion para comandos `/`
7. ✅ Paletas: `/` command palette, `/agent`, `/models`
8. ✅ Envío con Enter
9. ✅ Cancelar con Esc
10. ✅ Inline code (`/code`) y external editor (`/edit`)
11. ✅ Stash (`/stash`)