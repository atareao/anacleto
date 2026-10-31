# Git Flow

Este proyecto sigue **Git Flow** estricto con versionado semántico. Ningún cambio entra directamente en `main` ni en `development`: todo pasa por PR.

## Ramas

| Rama | Propósito | Base |
|---|---|---|
| `main` | Producción. Cada merge aquí dispara una release. | — |
| `development` | Integración de features en curso. Rama activa de trabajo. | `main` |
| `feature/<kebab-case>` | Nuevas funcionalidades. | `development` |
| `hotfix/<kebab-case>` | Correcciones urgentes a producción. | `main` |

## Flujo diario

### Features

```bash
git checkout development && git pull origin development
git checkout -b feature/mi-feature
# ... trabajo ...
git commit -m "✨ feat: añade modo oscuro"
git push -u origin feature/mi-feature
gh pr create --base development --head feature/mi-feature
```

### Hotfixes

```bash
git checkout main && git pull origin main
git checkout -b hotfix/arreglo-critico
git commit -m "🚑️ hotfix: crash con entrada vacía"
git push -u origin hotfix/arreglo-critico
gh pr create --base main --head hotfix/arreglo-critico
# tras el merge, sincronizar la integración:
git checkout development && git merge main && git push origin development
```

### Releases

Un merge a `main` **es** una release. No se etiqueta a mano: `release-prepare.yml` decide el bump desde los conventional commits, genera el changelog, commitea `chore: release vX.Y.Z` y empuja el tag, que es lo que dispara `publish.yml` (publicación a crates.io).

## Conventional Commits con Gitmoji

El formato `<emoji> <tipo>(<scope>): <mensaje>` determina el bump automático:

| Mensaje | Bump |
|---|---|
| `✨ feat: ...` | minor (0.Y.0) |
| `🐛 fix: ...` | patch (0.0.Z) |
| `💥 feat!: ...` o `BREAKING CHANGE` | major (X.0.0) |
| cualquier otro tipo (`refactor`, `docs`, `chore`, `test`…) | patch (0.0.Z) |

| Tipo | Emoji | Tipo | Emoji |
|---|---|---|---|
| `feat` | ✨ | `style` | 💄 |
| `fix` | 🐛 | `test` | ✅ |
| `hotfix` | 🚑️ | `chore` | 🔧 |
| `docs` | 📝 | `ci` | 👷 |
| `refactor` | ♻️ | `revert` | ⏪️ |
| `perf` | ⚡ | `deps` | ⬆️ |
| `breaking` | 💥 | `release` | 🔖 |

## Secretos de GitHub necesarios

| Secreto | Para qué |
|---|---|
| `GH_PAT` | PAT con `contents: write`. Lo necesita `release-prepare.yml` en el checkout: un tag empujado con el `GITHUB_TOKEN` por defecto **no** dispara `publish.yml`. También se usa para sincronizar `development` con `main`. |
| `CARGO_REGISTRY_TOKEN` | Token de API de crates.io para el job de publicación de `publish.yml`. |

## CI Workflows

| Workflow | Se dispara con | Hace |
|---|---|---|
| `ci.yml` | PR y push a `main`/`development` | `fmt --check`, `clippy -D warnings`, `cargo test` |
| `release-prepare.yml` | push a `main` | bump con vampus, CHANGELOG con git-cliff, tag `vX.Y.Z`, sincroniza `development` |
| `publish.yml` | tag `v*` | publica el crate en crates.io |
