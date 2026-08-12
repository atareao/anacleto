# Collapsible Sections Implementation Plan

> **For agentic workers:** Each task produces independently testable, commit-able work. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Allow users to collapse/expand sections delimited by `[thinking]`, `[tool]`, `[command]`, `[user]` markers in the chat panel.

**Architecture:** Section boundaries are tracked during rendering with stable IDs (`thinking_1`, `tool_2`, etc.). A `HashSet<String>` in `App` stores which sections are collapsed. The renderer filters out collapsed lines and replaces the section header with a summary line. Click on the `▐` border (column 0) toggles collapse state.

**Tech Stack:** Rust, ratatui, crossterm (mouse events)

## Global Constraints

- TUI only (no web/batch mode)
- Collapse state is ephemeral — not persisted to DB, reset on app restart
- Follow existing patterns in `render.rs`, `app.rs`, `events.rs`
- Pre-wrap is already implemented — each visual line is a `Line` object (1:1)
- All existing tests must continue to pass

---

### Task 1: Section tracking data structures

**Files:**
- Modify: `src/tui/types.rs` (new struct)
- Modify: `src/tui/app.rs` (new fields on `App`)

**Interfaces:**
- Consumes: Existing `App` struct, `Focus` enum
- Produces: `CollapsedSection` struct, new fields on `App`, `generate_section_id()` helper

- [ ] **Step 1: Write the failing test for `generate_section_id`**

Añadir en `src/tui/render.rs` (nuevo `#[cfg(test)] mod` o ampliar el existente):

```rust
#[test]
fn generate_section_id_increments_per_type() {
    let mut counters: HashMap<String, u32> = HashMap::new();
    assert_eq!(generate_section_id("thinking", &mut counters), "thinking_1");
    assert_eq!(generate_section_id("thinking", &mut counters), "thinking_2");
    assert_eq!(generate_section_id("tool", &mut counters), "tool_1");
    assert_eq!(generate_section_id("thinking", &mut counters), "thinking_3");
}
```

Ejecutar: `cargo test generate_section_id_increments_per_type -- --ignored`
Expected: COMPILATION ERROR (no test found, function doesn't exist yet)

- [ ] **Step 2: Add `CollapsedSection` struct and `generate_section_id`**

En `src/tui/types.rs`, añadir:

```rust
/// Represents a single collapsible section in the chat render.
#[derive(Debug, Clone)]
pub(crate) struct CollapsedSection {
    /// Unique identifier: "{type}_{counter}" e.g. "thinking_1"
    pub(crate) id: String,
    /// Section type: "thinking", "tool", "normal", "user", "command"
    pub(crate) section_type: String,
    /// Index of the first line (the ▐ border) in rendered_chat_lines
    pub(crate) start_line: usize,
    /// Number of content lines (excluding the header ▐)
    pub(crate) line_count: usize,
}
```

En `src/tui/app.rs`, añadir los campos al struct `App`:

```rust
/// Set of collapsed section IDs (ephemeral, per-session)
pub(crate) collapsed_sections: HashSet<String>,
/// Per-frame mapping: line index in rendered_chat_lines → section_id
pub(crate) section_line_map: Vec<Option<String>>,
/// Per-frame section info for collapse rendering
pub(crate) section_info: Vec<CollapsedSection>,
```

Inicializar en el constructor de `App`:

```rust
collapsed_sections: HashSet::new(),
section_line_map: Vec::new(),
section_info: Vec::new(),
```

Añadir el helper `generate_section_id` en `src/tui/render.rs` (fuera de cualquier módulo de test):

```rust
/// Generate a unique section ID from a type name and a per-type counter.
/// Returns strings like "thinking_1", "tool_2", etc.
pub(crate) fn generate_section_id(
    section_type: &str,
    counters: &mut HashMap<String, u32>,
) -> String {
    let entry = counters.entry(section_type.to_string()).or_insert(0);
    *entry += 1;
    format!("{}_{}", section_type, entry)
}
```

Asegurar que `HashMap` esté importado en `render.rs`:

```rust
use std::collections::HashMap;
```

- [ ] **Step 3: Run test to verify it passes**

Ejecutar: `cargo test generate_section_id_increments_per_type`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/tui/types.rs src/tui/app.rs src/tui/render.rs
git commit -m "feat: add section tracking data structures and ID generation"
```

---

### Task 2: Section ID generation during rendering

**Files:**
- Modify: `src/tui/render.rs`
- Modify: `src/tui/app.rs` (clear section data per frame)

**Interfaces:**
- Consumes: `generate_section_id()`, `CollapsedSection`, `App.collapsed_sections`, `App.section_line_map`, `App.section_info`
- Produces: `flush_section()` now tags lines with section IDs, `App.section_line_map` and `App.section_info` populated

- [ ] **Step 5: Write the failing test for section tagging**

En `src/tui/render.rs` (test module):

```rust
#[test]
fn flush_section_tags_lines_with_section_id() {
    let styles = test_styles();
    let mut out: Vec<Line> = Vec::new();
    let mut buf: Vec<Line> = vec![raw_line("hello")];
    let mut cv = 0usize;
    let mut last: Option<&'static str> = None;
    let mut section_line_map: Vec<Option<String>> = Vec::new();
    let mut section_info: Vec<CollapsedSection> = Vec::new();
    let mut counters: HashMap<String, u32> = HashMap::new();

    flush_section(
        &mut out, &mut buf, "tool", &styles, &mut cv, 80, &mut last,
        &mut section_line_map, &mut section_info, &mut counters,
    );

    // Should have 3 lines: ▐, hello, ▐
    assert_eq!(out.len(), 3);
    // First line should be tagged with "tool_1"
    assert_eq!(section_line_map[0], Some("tool_1".to_string()));
    assert_eq!(section_line_map[1], Some("tool_1".to_string()));
    assert_eq!(section_line_map[2], Some("tool_1".to_string()));
    // section_info should have one entry
    assert_eq!(section_info.len(), 1);
    assert_eq!(section_info[0].id, "tool_1");
    assert_eq!(section_info[0].line_count, 1);
}
```

Ejecutar: `cargo test flush_section_tags_lines_with_section_id -- --ignored`
Expected: FAIL (compilation error — `flush_section` signature doesn't match)

- [ ] **Step 6: Modify `flush_section` signature and implementation**

Signature actual (aproximadamente línea 1002):

```rust
fn flush_section(
    out: &mut Vec<Line<'static>>,
    buf: &mut Vec<Line<'static>>,
    section_type: &'static str,
    styles: &SectionStyles,
    cumulative_visual: &mut usize,
    content_width: usize,
    last_flushed: &mut Option<&'static str>,
) {
```

Nueva firma:

```rust
fn flush_section(
    out: &mut Vec<Line<'static>>,
    buf: &mut Vec<Line<'static>>,
    section_type: &'static str,
    styles: &SectionStyles,
    cumulative_visual: &mut usize,
    content_width: usize,
    last_flushed: &mut Option<&'static str>,
    section_line_map: &mut Vec<Option<String>>,
    section_info: &mut Vec<CollapsedSection>,
    counters: &mut HashMap<String, u32>,
) {
```

Al inicio de `flush_section`, si el buffer no está vacío, generar un section_id:

```rust
if buf.is_empty() {
    return;
}

let section_id = generate_section_id(section_type, counters);
let start_line = out.len();
let content_lines = buf.len();

// Record section info
section_info.push(CollapsedSection {
    id: section_id.clone(),
    section_type: section_type.to_string(),
    start_line,
    line_count: content_lines,
});
```

Después de cada `out.push(line)` en `flush_section`, añadir:

```rust
section_line_map.push(Some(section_id.clone()));
```

Al final de `flush_section`, añadir un marker para el final del bloque:

```rust
// The last line (▐) is also the end of this section
```

**Nota**: `flush_section` actualmente no itera push por push — hay que revisar su implementación exacta para añadir `section_line_map.push(Some(id.clone()))` después de cada `out.push()`. Mirar la implementación actual (líneas ~1002-1045) y adaptar.

- [ ] **Step 7: Update all callers of `flush_section`**

Hay 6 llamadas a `flush_section` en `render_sectioned_block` (líneas ~1589-1633) y 1 en `flush_ai_batch` (indirecta). Todas necesitan pasar los nuevos parámetros.

En `render_sectioned_block`, añadir parámetros al inicio:

```rust
pub(crate) fn render_sectioned_block(
    // ... existing params ...
    section_line_map: &mut Vec<Option<String>>,
    section_info: &mut Vec<CollapsedSection>,
    counters: &mut HashMap<String, u32>,
) -> Vec<Line<'static>> {
```

Y pasar `section_line_map`, `section_info`, `counters` a cada llamada a `flush_section`.

En `flush_ai_batch` y `render_chat`, crear los vectores y pasarlos.

- [ ] **Step 8: Clear section data per frame in `render_chat`**

Al inicio de `render_chat`, antes de construir líneas:

```rust
app.section_line_map.clear();
app.section_info.clear();
```

Y pasar `&mut app.section_line_map`, `&mut app.section_info`, y un `&mut HashMap::new()` local a `flush_ai_batch`.

- [ ] **Step 9: Run tests**

Ejecutar: `cargo test`
Expected: All 427+ tests pass

- [ ] **Step 10: Commit**

```bash
git add src/tui/render.rs src/tui/app.rs
git commit -m "feat: tag rendered lines with section IDs during flush_section"
```

---

### Task 3: Collapsed rendering

**Files:**
- Modify: `src/tui/render.rs`

**Interfaces:**
- Consumes: `App.collapsed_sections`, `App.section_info`, `App.section_line_map`
- Produces: Filtered `display_lines` with collapsed sections replaced by summary lines

- [ ] **Step 11: Write the failing test for collapsed rendering**

```rust
#[test]
fn collapsed_section_hides_content_and_shows_summary() {
    let mut collapsed = HashSet::new();
    collapsed.insert("tool_1".to_string());

    let mut lines: Vec<Line> = Vec::new();
    // Simulate a tool section with 3 content lines
    for i in 0..5 {
        lines.push(Line::from(Span::raw(format!("line {}", i))));
    }
    let section_info = vec![CollapsedSection {
        id: "tool_1".to_string(),
        section_type: "tool".to_string(),
        start_line: 0,
        line_count: 3,
    }];
    let section_line_map: Vec<Option<String>> = vec![
        Some("tool_1".to_string()),
        Some("tool_1".to_string()),
        Some("tool_1".to_string()),
        Some("tool_1".to_string()),
        Some("tool_1".to_string()),
    ];

    let result = apply_collapsed(lines, &collapsed, &section_info, &section_line_map, 80);

    // Should have 1 line (the summary)
    assert_eq!(result.len(), 1);
    let text: String = result[0].spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(text.contains("▾")); // collapsed indicator
    assert!(text.contains("tool")); // section type
    assert!(text.contains("3")); // line count
}
```

Ejecutar: `cargo test collapsed_section_hides_content_and_shows_summary -- --ignored`
Expected: COMPILATION ERROR (apply_collapsed doesn't exist yet)

- [ ] **Step 12: Implement `apply_collapsed` function**

En `src/tui/render.rs`, añadir:

```rust
/// Apply collapse state to a list of rendered lines.
/// Returns a new Vec with collapsed sections replaced by summary lines.
pub(crate) fn apply_collapsed(
    lines: Vec<Line<'static>>,
    collapsed: &HashSet<String>,
    section_info: &[CollapsedSection],
    section_line_map: &[Option<String>],
    content_width: usize,
) -> Vec<Line<'static>> {
    // Build a set of line indices to hide
    let mut hidden_lines: HashSet<usize> = HashSet::new();
    let mut summary_lines: HashMap<usize, String> = HashMap::new(); // line_idx → summary text

    for section in section_info {
        if !collapsed.contains(&section.id) {
            continue;
        }
        // First line becomes the summary
        let summary = format!(
            " \u{25be} [{}] ({} l\u{ed}neas ocultas)",
            section.section_type, section.line_count
        );
        summary_lines.insert(section.start_line, summary);

        // All other lines of this section are hidden
        for i in (section.start_line + 1)..(section.start_line + 1 + section.line_count + 1) {
            if i < lines.len() {
                hidden_lines.insert(i);
            }
        }
    }

    let mut result: Vec<Line<'static>> = Vec::with_capacity(lines.len());

    for (idx, line) in lines.into_iter().enumerate() {
        if hidden_lines.contains(&idx) {
            continue;
        }
        if let Some(summary) = summary_lines.remove(&idx) {
            // Replace the header ▐ line with the summary
            // Keep the first span (the ▐ border style)
            let prefix = line.spans.first().cloned().unwrap_or_else(|| {
                Span::styled("\u{2590} ", Style::default())
            });
            let mut spans = vec![prefix];
            spans.push(Span::styled(
                summary,
                Style::default().fg(Color::Rgb(150, 150, 180)).add_modifier(Modifier::DIM),
            ));
            result.push(Line::from(spans));
        } else {
            result.push(line);
        }
    }

    result
}
```

Añadir imports necesarios en `render.rs`:

```rust
use std::collections::HashSet;
```

- [ ] **Step 13: Integrate `apply_collapsed` in `render_chat`**

En `render_chat`, justo después del pre-wrap y antes de `select_visible_start`:

```rust
// Apply collapsed sections
lines = apply_collapsed(
    lines,
    &app.collapsed_sections,
    &app.section_info,
    &app.section_line_map,
    content_width,
);
```

- [ ] **Step 14: Run tests**

Ejecutar: `cargo test`
Expected: All tests pass

- [ ] **Step 15: Commit**

```bash
git add src/tui/render.rs
git commit -m "feat: collapsed sections show summary instead of content"
```

---

### Task 4: Click handler for toggle

**Files:**
- Modify: `src/tui/events.rs`

**Interfaces:**
- Consumes: `App.collapsed_sections`, `App.section_line_map`, `App.rendered_chat_lines`
- Produces: Toggle in `App.collapsed_sections`, re-render

- [ ] **Step 16: Write the failing test for click toggle**

En `src/tui/render.rs` (test module):

```rust
#[test]
fn toggle_collapsed_adds_and_removes_from_set() {
    let mut collapsed: HashSet<String> = HashSet::new();
    let section_line_map: Vec<Option<String>> = vec![
        Some("tool_1".to_string()),
        Some("tool_1".to_string()),
    ];

    // Toggle on line 0 → should add "tool_1"
    let result = toggle_section_at_line(0, &mut collapsed, &section_line_map);
    assert!(result);
    assert!(collapsed.contains("tool_1"));

    // Toggle again on line 0 → should remove "tool_1"
    let result = toggle_section_at_line(0, &mut collapsed, &section_line_map);
    assert!(result);
    assert!(!collapsed.contains("tool_1"));
}

#[test]
fn toggle_section_unknown_line_returns_false() {
    let mut collapsed: HashSet<String> = HashSet::new();
    let section_line_map: Vec<Option<String>> = vec![None, None];

    let result = toggle_section_at_line(0, &mut collapsed, &section_line_map);
    assert!(!result);
}
```

Ejecutar: `cargo test toggle_collapsed_adds_and_removes_from_set -- --ignored`
Expected: COMPILATION ERROR

- [ ] **Step 17: Implement `toggle_section_at_line`**

En `src/tui/render.rs`:

```rust
/// Toggle collapse state for the section at the given line index.
/// Returns true if a section was toggled, false if the line has no section.
pub(crate) fn toggle_section_at_line(
    line_idx: usize,
    collapsed: &mut HashSet<String>,
    section_line_map: &[Option<String>],
) -> bool {
    let Some(Some(section_id)) = section_line_map.get(line_idx) else {
        return false;
    };
    if collapsed.contains(section_id) {
        collapsed.remove(section_id);
    } else {
        collapsed.insert(section_id.clone());
    }
    true
}
```

- [ ] **Step 18: Integrate click handler in `events.rs`**

En `src/tui/events.rs`, localizar el handler de mouse click. Buscar donde se maneja `MouseEventKind::Down` o similar. Añadir:

```rust
// Collapse/expand toggle on ▐ border (column 0)
if col == 0 {
    // The click is on the chat panel's border area
    // Compute the line index from the y coordinate
    // (need to account for the Paragraph border offset)
    if let Some(line_idx) = compute_line_from_y(y, area, app) {
        if toggle_section_at_line(
            line_idx,
            &mut app.collapsed_sections,
            &app.section_line_map,
        ) {
            return;
        }
    }
}
```

Donde `compute_line_from_y` es una función helper que convierte la coordenada Y del ratón a índice de línea en `rendered_chat_lines`:

```rust
fn compute_line_from_y(y: u16, area: &Rect, app: &App) -> Option<usize> {
    let y_local = y.checked_sub(area.y)?;
    // Subtract border: Paragraph has 2 lines of border (top + title)
    let y_content = y_local.checked_sub(2)?;
    // Add scroll offset
    let line_idx = y_content as usize;
    if line_idx < app.rendered_chat_lines.len() {
        Some(line_idx)
    } else {
        None
    }
}
```

- [ ] **Step 19: Run tests**

Ejecutar: `cargo test`
Expected: All tests pass

- [ ] **Step 20: Commit**

```bash
git add src/tui/events.rs src/tui/render.rs
git commit -m "feat: click on ▐ border toggles section collapse"
```

---

### Task 5: Edge cases and polish

**Files:**
- Modify: `src/tui/render.rs`
- Modify: `src/tui/events.rs`

- [ ] **Step 21: Test consecutive same-type sections**

```rust
#[test]
fn consecutive_same_type_sections_collapse_independently() {
    let mut collapsed = HashSet::new();
    collapsed.insert("tool_1".to_string()); // only first section collapsed

    // Simulate two same-type sections: ▐, a, ▐, b, ▐
    let lines: Vec<Line> = (0..5).map(|i| Line::from(Span::raw(format!("line {}", i)))).collect();
    let section_info = vec![
        CollapsedSection {
            id: "tool_1".to_string(),
            section_type: "tool".to_string(),
            start_line: 0,
            line_count: 1, // content: "line 1"
        },
        CollapsedSection {
            id: "tool_2".to_string(),
            section_type: "tool".to_string(),
            start_line: 2,
            line_count: 1, // content: "line 3"
        },
    ];
    let section_line_map: Vec<Option<String>> = vec![
        Some("tool_1".to_string()), // line 0: ▐
        Some("tool_1".to_string()), // line 1: a
        Some("tool_2".to_string()), // line 2: ▐ (shared)
        Some("tool_2".to_string()), // line 3: b
        Some("tool_2".to_string()), // line 4: ▐
    ];

    let result = apply_collapsed(lines, &collapsed, &section_info, &section_line_map, 80);

    // Should have 4 lines: summary_tool1, ▐_tool2, b, ▐_tool2
    assert_eq!(result.len(), 4);
    let text: String = result.iter().flat_map(|l| {
        l.spans.iter().map(|s| s.content.as_ref()).collect::<Vec<_>>()
    }).collect();
    assert!(text.contains("▾")); // collapsed indicator present
    assert!(text.contains("tool_1")); // or just "tool" type
}
```

- [ ] **Step 22: Verify streaming behavior**

En `render_chat`, la línea de streaming se añade con `lines.extend(render_sectioned_block(...))`. Como `apply_collapsed` se ejecuta después del pre-wrap y después de añadir el streaming, las secciones colapsadas se ocultan incluso si están en el stream actual.

Verificar manualmente: iniciar una respuesta larga, colapsar `[thinking]` mientras el modelo sigue escribiendo. La línea de resumen debe permanecer visible y actualizarse con el contador de líneas.

- [ ] **Step 23: Run full test suite**

Ejecutar: `cargo test`
Expected: All 427+ tests pass

- [ ] **Step 24: Commit**

```bash
git add src/tui/render.rs src/tui/events.rs
git commit -m "test: add edge case tests for collapsed sections"
```

---

### Task 6: Manual QA and polish

**Files:**
- Modify: `src/tui/render.rs` (style tweaks)
- Modify: `src/tui/app.rs` (any state refinements)

- [ ] **Step 25: Build and run**

```bash
cargo build --release && cargo run
```

- [ ] **Step 26: Manual test scenarios**

1. Enviar un mensaje que genere thinking → tool → response
2. Click en el `▐` del `[thinking]` → debe colapsarse
3. Click en el mismo `▐` → debe expandirse
4. Colapsar `[tool]` → debe ocultar el output de la herramienta
5. Redimensionar la ventana → las secciones colapsadas deben mantener su estado
6. Scroll hacia arriba/abajo → el colapso debe funcionar con scroll
7. Hacer click en columna 0 pero fuera de una sección → no debe togglear nada

- [ ] **Step 27: Final commit with any polish**

```bash
git add -A
git commit -m "fix: polish collapsed section rendering and click handling"
```