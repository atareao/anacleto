---
name: python-best-practices
description: Escribir, compilar, probar y mantener código Python idiomático, moderno y profesional
metadata:
  version: "1.0"
  category: development
  risk: medium
---

# Python Best Practices skill

Especializado en escribir y mantener código Python idiomático, moderno y correcto.
Usa este skill cuando necesites implementar, refactorizar, probar o depurar
código Python dentro del workspace.

## Estándares de Python moderno

- **Python 3.10+ como mínimo.** Prefiere Python 3.12+ si no hay restricciones del proyecto.
- Usa **type hints** (`typing` module, `|` para uniones desde 3.10, `list[str]` desde 3.9).
- Fomenta el uso de **dataclasses** (`@dataclass`) sobre clases manuales con `__init__`.
- Usa **pathlib** (`Path`) en lugar de `os.path` para manejo de rutas.
- Prefiere **f-strings** sobre `%` formatting o `.format()`.
- Usa **context managers** (`with` statement) para recursos (archivos, conexiones, locks).

## Flujo de trabajo estándar

1. **Explora** el proyecto: lee `pyproject.toml`, `setup.py`, `setup.cfg`, `requirements.txt`,
   `Pipfile` o `pyproject.toml` para entender dependencias y configuración.
2. **Comprueba el estado base** antes de escribir código:

   ```sh
   python --version
   pip list 2>/dev/null || poetry show 2>/dev/null
   ```

3. **Implementa** el cambio siguiendo las convenciones del proyecto.
4. **Verifica la orden obligatoria** antes de dar por terminado. **Ruff es imprescindible** — debe estar instalado y ejecutarse siempre:

   ```sh
   ruff check .          # Linter (NO usar flake8 ni isort)
   ruff format --check . # Formateo (NO usar black)
   mypy .                # Type checking estático
   pytest                # Tests
   ```

   > ⚠️ **IMPORTANTE**: No se aceptan alternativas como flake8, black o isort. **Solo ruff** para linting y formateo. Si no está instalado, instálalo con `pip install ruff` o `uv add ruff`.

5. **Documenta** cualquier decisión con docstrings estilo Google, NumPy o Sphinx
   según lo que use el proyecto.

## Herramientas recomendadas

| Herramienta | Propósito | Comando |
|---|---|---|
| **ruff** | Linter + formateo (todo-en-uno) | `ruff check . && ruff format .` |
| **mypy** | Type checking estático | `mypy .` |
| **pytest** | Testing | `pytest -v` |
| **pytest-cov** | Cobertura de tests | `pytest --cov=.` |
| **pip-tools / uv** | Gestión de dependencias | `pip-compile` / `uv pip compile` |
| **pre-commit** | Hooks de git | `pre-commit run --all-files` |
| **nox / tox** | Tests multi-entorno | `nox` / `tox` |

## Reglas y anti-patterns

### Estilo y estructura

- **PEP 8** es la guía de estilo base. Ruff la aplica automáticamente.
- **Nombres**: `snake_case` para variables, funciones y métodos; `UPPER_CASE` para constantes;
  `CamelCase` para clases; `_` prefijo para privado (por convención, no enforcement).
- **Límite de línea**: 88 caracteres (estándar de Ruff/Black).
- **Imports**: primero stdlib, luego terceros, luego locales. Ruff los ordena automáticamente.

### Type hints

- **Siempre anota funciones públicas** con tipos de parámetros y retorno.
- Usa `Optional[X]` o `X | None` (desde 3.10) para valores opcionales.
- Para tipos complejos, crea `TypeAlias` (desde 3.10) o `type` alias.
- Prefiere `collections.abc.Sequence` sobre `list` si solo necesitas iteración.

### Errores y manejo de excepciones

- **No uses excepto genéricos `except:`** sin especificar el tipo de excepción.
- Captura excepciones específicas: `except ValueError:`.
- Usa `contextlib.suppress` para ignorar excepciones esperadas de forma explícita.
- No uses `assert` para validación de datos de entrada (se desactivan con `-O`).

### Testing

- Los tests van en el directorio `tests/` con estructura reflejando `src/`.
- Nombra los archivos `test_*.py` y las funciones `test_*`.
- Usa `pytest` como framework. Prefiere `pytest.fixture` sobre `setUp`/`tearDown`.
- Usa `pytest.mark.parametrize` para múltiples casos.
- Marca tests lentos o de red con `@pytest.mark.slow` o `@pytest.mark.network`.
- Tests de skill no deben acceder a la red salvo que estén marcados explícitamente.

### Rendimiento

- Prefiere comprensiones (`list`, `dict`, `set`) sobre `map`/`filter` con `lambda`.
- Usa `yield` y generadores para secuencias grandes.
- Evita repetición de cálculos; usa `functools.lru_cache` o `functools.cache`.
- Prefiere operaciones con sets (`&`, `|`, `-`, `^`) sobre bucles anidados para búsquedas.

### Seguridad

- No hardcodees secrets, API keys o contraseñas. Usa variables de entorno o `.env`.
- Usa `secrets` module (no `random`) para valores criptográficos.
- Valida y sanitiza toda entrada externa.
- Prefiere `subprocess.run` con `check=True` sobre `os.system`.

## Estructura de proyecto recomendada

```
proyecto/
├── pyproject.toml          # Configuración moderna (PEP 621)
├── README.md
├── src/
│   └── proyecto/           # Código fuente en namespace package
│       ├── __init__.py
│       ├── mod1.py
│       └── mod2.py
├── tests/
│   ├── __init__.py
│   ├── test_mod1.py
│   └── conftest.py         # Fixtures compartidos
├── docs/                   # Documentación (opcional)
├── .pre-commit-config.yaml # Hooks de pre-commit (opcional)
└── .env.example            # Variables de entorno de ejemplo
```

## Modo de uso

Pasa el `task` describiendo qué construir o arreglar, rutas relevantes y el
criterio de aceptación. El skill ejecutará los comandos y devolverá stdout+stderr.

### Ejemplo

```yaml
task: |
  Implementa un módulo `crc32` en src/crc32.py con cobertura de tests,
  luego valida con:
    ruff check . && ruff format --check . && mypy . && pytest -v
```

Otro ejemplo:

```yaml
task: |
  Refactoriza el archivo src/utils.py:
  - Convierte a type hints modernos (Python 3.12)
  - Cambia os.path por pathlib
  - Añade docstrings estilo Google
  - Añade tests en tests/test_utils.py
  Usa ruff, mypy y pytest para validar.
```
