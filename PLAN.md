# Git Branch in TUI Footer — Implementation Plan

## Objetivo

Mostrar la rama actual de git en el centro del footer de la TUI, entre el working directory (izquierda) y el nombre del modelo (derecha).

## Arquitectura

Se añade un campo `git_branch: Option<String>` a `App`, inicializado en `App::new()` ejecutando `git rev-parse --abbrev-ref HEAD` vía `std::process::Command`. La función `render_working_dir` se modifica para renderizar el texto de la rama (`⎇ main`) centrado entre el dir y el modelo. Si no hay repo o hay error, no se muestra nada.

## Tareas

### Tarea 1: Añadir campo `git_branch` a `App`

**Archivos:**
- Modificar: `src/tui/app.rs:107`

- [ ] **Paso 1:** Añadir el campo `pub git_branch: Option<String>` después de `working_dir` (línea 107).

  ```rust
  /// Current working directory for display.
  pub working_dir: String,
  /// Current git branch name (None if not a git repo or on detached HEAD).
  pub git_branch: Option<String>,
  ```

### Tarea 2: Inicializar `git_branch` en `App::new()`

**Archivos:**
- Modificar: `src/tui/app.rs:217-219`

- [ ] **Paso 1:** Justo después de obtener `working_dir` (tras la línea 219), añadir la detección de rama git.

  ```rust
  let git_branch = std::process::Command::new("git")
      .args(["rev-parse", "--abbrev-ref", "HEAD"])
      .output()
      .ok()
      .and_then(|o| {
          if o.status.success() {
              let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
              if s.is_empty() || s == "HEAD" {
                  None // detached HEAD or no commits
              } else {
                  Some(s)
              }
          } else {
              None
          }
      });
  ```

  Notas:
  - `Command::new("git")` usa el PATH del sistema, no requiere rutas absolutas.
  - `String::from_utf8_lossy` maneja salida no UTF-8 sin panic (poco común pero seguro).
  - `s == "HEAD"` cubre el caso detached HEAD (git devuelve "HEAD" literal).
  - `o.status.success()` es false si `git` no está instalado o si el directorio no es un repo.

- [ ] **Paso 2:** Añadir `git_branch` al struct literal `Self { ... }` (después de `working_dir` en línea 271).

  ```rust
  working_dir,
  git_branch,
  ```

### Tarea 3: Modificar `render_working_dir` para mostrar la rama

**Archivos:**
- Modificar: `src/tui/render.rs:792-830`

- [ ] **Paso 1:** Reemplazar la función `render_working_dir` completa con una versión que calcule el texto de la rama y lo intercale entre dir y modelo.

  ```rust
  /// Render the current working directory (left), git branch (center),
  /// and active model (right). The branch is only shown when inside a git
  /// repo with a valid named branch.
  fn render_working_dir(f: &mut Frame, area: Rect, app: &App) {
      let dir_text = format!(" 📁 {}", app.working_dir);
      let model_text = format!("🤖 {}", app.current_model);

      let branch_span = app.git_branch.as_ref().map(|b| {
          Span::styled(
              format!(" ⎇ {} ", b),
              Style::default().fg(Color::Rgb(188, 143, 254)), // lavender
          )
      });

      let width = area.width as usize;

      // Calculate display widths
      let dir_width = dir_text.width();
      let model_width = model_text.width();
      let branch_width = branch_span.as_ref().map(|s| s.width()).unwrap_or(0);

      // Remaining space after reserving dir + branch + model (with 1-space gaps)
      let total_fixed = dir_width + branch_width + model_width;
      let gap_count = if branch_span.is_some() { 2 } else { 1 };
      let padding = width.saturating_sub(total_fixed + gap_count);

      let line = if let Some(branch) = branch_span {
          // Distribute padding: put branch roughly centered.
          // 1/3 of padding before branch, 2/3 after (between branch and model).
          let left_pad = padding / 3;
          let right_pad = padding - left_pad;
          Line::from(vec![
              Span::styled(dir_text, Style::default().fg(Color::DarkGray)),
              Span::raw(" ".repeat(left_pad)),
              branch,
              Span::raw(" ".repeat(right_pad)),
              Span::styled(model_text, Style::default().fg(Color::Cyan)),
          ])
      } else {
          // Original layout: dir left, model right (no branch)
          let max_dir_width = width.saturating_sub(model_width + 1);
          let truncated_dir = if dir_width > max_dir_width && max_dir_width > 3 {
              let keep = max_dir_width.saturating_sub(1);
              let mut s = String::new();
              let mut w = 0;
              for ch in dir_text.chars() {
                  let cw = ch.to_string().width();
                  if w + cw > keep {
                      break;
                  }
                  s.push(ch);
                  w += cw;
              }
              s.push('…');
              s
          } else {
              dir_text
          };
          let p = width.saturating_sub(truncated_dir.width() + model_width);
          Line::from(vec![
              Span::styled(truncated_dir, Style::default().fg(Color::DarkGray)),
              Span::raw(" ".repeat(p)),
              Span::styled(model_text, Style::default().fg(Color::Cyan)),
          ])
      };

      f.render_widget(Paragraph::new(line), area);
  }
  ```

## Verificación

1. `cargo build` debe compilar sin errores ni warnings.
2. `cargo clippy` sin nuevos warnings.
3. Ejecutar el TUI en un repo git: el footer debe mostrar `📁 /path ⎇ main 🤖 modelo`.
4. Ejecutar el TUI fuera de un repo git: el footer debe verse exactamente como antes (sin rama).
5. Ejecutar el TUI en un repo con detached HEAD (`git checkout --detach`): no mustra rama.