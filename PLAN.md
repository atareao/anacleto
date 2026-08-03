# Corrección de expansión de tilde (~) en rutas — Implementation Plan

## Objetivo

Corregir el bug por el que las rutas que empiezan por `~` (p. ej. `~/.local/share/anacleto/sessions.db`) no se expanden a la ruta real del home, provocando la creación de un directorio literal llamado `~` en el directorio de trabajo actual.

## Contexto del bug

El config de proyecto `.anacleto/config.yaml` define `session.database_path: "~/.local/share/anacleto/sessions.db"`. El código **no** expande el `~` a la ruta del home:

- En `src/db/session.rs`, la función `Database::open` (líneas 20-24) llama a `create_dir_all(parent)` con el path literal. Como el path empieza por `~`, se crea un directorio literal llamado `~` en el directorio de trabajo actual. Como Anacleto se ejecuta desde distintos directorios, se crea un `~` en cada uno.
- El valor por defecto en `src/config/types.rs` (función `default_db_path`, líneas 249-254) usa `dirs::data_dir()`, que devuelve la ruta real del home, pero la config lo sobrescribe con la cadena literal `~`.

**Solución elegida (Opción B):** añadir una función de expansión de tilde y aplicarla a las rutas que vienen de la config.

## Arquitectura

Se añade una función pura y reutilizable `expand_tilde(path: &Path) -> PathBuf` en el módulo de resolución de rutas (`src/config/paths.rs`), que convierte un path que empiece por `~` o `~/` en la ruta del home usando `dirs::home_dir()`, y devuelve el path sin cambios en caso contrario. Esta función se aplica a `database_path` en el loader de config y a las rutas absolutas de skills en `resolve_skill_path`.

## Tareas

### Tarea 1: Añadir la función `expand_tilde` en `src/config/paths.rs`

**Archivos:**
- Modificar: `src/config/paths.rs`

- [ ] **Paso 1:** Añadir la función pública `expand_tilde(path: &Path) -> PathBuf`.
      - Si el path empieza por `~` o `~/`, reemplazar el prefijo `~` por `dirs::home_dir()`.
      - Si `dirs::home_dir()` devuelve `None`, devolver el path sin cambios.
      - En cualquier otro caso, devolver el path sin cambios.

      ```rust
      pub fn expand_tilde(path: &Path) -> PathBuf {
          let s = path.to_string_lossy();
          if s == "~" || s.starts_with("~/") {
              if let Some(home) = dirs::home_dir() {
                  if s == "~" {
                      return home;
                  }
                  return home.join(&s[2..]);
              }
          }
          path.to_path_buf()
      }
      ```

### Tarea 2: Aplicar `expand_tilde` a `database_path` en el loader de config

**Archivos:**
- Modificar: `src/config/loader.rs:11-42`

- [ ] **Paso 1:** Tras cargar/mergear la config, aplicar `expand_tilde` al campo `database_path` de la sesión antes de que se use.
      - Importar `expand_tilde` desde `crate::config::paths`.
      - Asignar el resultado de `expand_tilde(&config.session.database_path)` de vuelta a `config.session.database_path`.

      ```rust
      config.session.database_path = expand_tilde(&config.session.database_path);
      ```

### Tarea 3: Aplicar `expand_tilde` en `resolve_skill_path` de `src/agent/loader.rs`

**Archivos:**
- Modificar: `src/agent/loader.rs:167-173`

- [ ] **Paso 1:** En la función `resolve_skill_path`, para rutas absolutas que empiecen por `~`, aplicar `expand_tilde` antes de resolver la ruta.
      - Importar `expand_tilde` desde `crate::config::paths`.
      - Cuando la ruta sea absoluta y empiece por `~`, expandirla antes de continuar con la resolución.

      ```rust
      let path = expand_tilde(&path);
      ```

### Tarea 4: Añadir tests unitarios para `expand_tilde`

**Archivos:**
- Modificar: `src/config/paths.rs`

- [ ] **Paso 1:** Añadir un módulo `#[cfg(test)] mod tests` (o ampliar el existente) con tests para `expand_tilde`:
      - `~` → devuelve `dirs::home_dir()`.
      - `~/foo/bar` → devuelve `home_dir().join("foo/bar")`.
      - Path absoluto normal (p. ej. `/etc/hosts`) → se devuelve sin cambios.
      - Path relativo (p. ej. `./data/db.sqlite`) → se devuelve sin cambios.

      ```rust
      #[cfg(test)]
      mod tests {
          use super::*;
          use std::path::Path;

          #[test]
          fn expand_tilde_home_only() {
              let home = dirs::home_dir().unwrap();
              assert_eq!(expand_tilde(Path::new("~")), home);
          }

          #[test]
          fn expand_tilde_with_subpath() {
              let home = dirs::home_dir().unwrap();
              assert_eq!(expand_tilde(Path::new("~/foo/bar")), home.join("foo/bar"));
          }

          #[test]
          fn expand_tilde_absolute_unchanged() {
              let p = Path::new("/etc/hosts");
              assert_eq!(expand_tilde(p), p);
          }

          #[test]
          fn expand_tilde_relative_unchanged() {
              let p = Path::new("./data/db.sqlite");
              assert_eq!(expand_tilde(p), p);
          }
      }
      ```

## Criterios de aceptación / verificación

- [ ] `cargo fmt --check` pasa sin cambios pendientes.
- [ ] `cargo clippy` pasa sin warnings.
- [ ] `cargo test` pasa (incluidos los nuevos tests de `expand_tilde`).
- [ ] Al ejecutar Anacleto desde cualquier directorio, ya **no** se crea un directorio literal llamado `~` en el directorio de trabajo actual.
- [ ] `session.database_path` con valor `~/.local/share/anacleto/sessions.db` se resuelve a la ruta real del home.
