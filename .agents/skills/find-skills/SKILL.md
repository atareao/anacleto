---
name: find-skills
description: |
  Busca y descubre skills instaladas localmente, en skills.sh (mattpocock/skills)
  y en la web mediante SearXNG.
  Úsala cuando el usuario pregunte "cómo hago X", "busca una skill para X",
  "hay una skill que pueda...", o quiera extender las capacidades del agente.
metadata:
  version: "1.2"
  category: system
  risk: low
---

# Find Skills (Anacleto)

Esta skill te ayuda a **descubrir skills ya instaladas** en el ecosistema Anacleto
y a **encontrar nuevas skills** disponibles en registros públicos como
[skills.sh](https://www.skills.sh/mattpocock/skills).

A diferencia del ecosistema `vercel-labs/skills` (que usa `npx skills`), Anacleto
gestiona las skills como ficheros Markdown con frontmatter YAML en directorios
locales. Esta skill adapta el concepto de `find-skills` al modelo de Anacleto.

---

## ¿Dónde se buscan las skills?

Anacleto busca skills en estas rutas, por orden de prioridad:

| Ruta / Fuente | Ámbito | Descripción |
|---|---|---|
| `.agents/skills/<name>/SKILL.md` | Proyecto | Skills del proyecto actual |
| `~/.config/anacleto/skills/<name>/SKILL.md` | Global (usuario) | Skills globales del usuario |
| `~/.config/anacleto/agents/<name>.md` | Global (agentes) | Definiciones de agentes (también tienen frontmatter) |
| `https://github.com/mattpocock/skills` | Remoto (skills.sh) | Registro público de Matt Pocock (~51 skills) |
| `https://searxng.one.belcar.corp` | Web (SearXNG) | Búsqueda web en GitHub, blogs, foros, documentación |

> [!NOTE]
> skills.sh NO tiene API JSON pública. Para acceder a sus skills usamos
> directamente la GitHub API del repo [mattpocock/skills](https://github.com/mattpocock/skills).

---

## Cómo buscar skills instaladas (local)

### 1. Listar todas las skills disponibles

```bash
# Skills del proyecto
fd SKILL.md .agents/skills/ --full-path

# Skills globales
fd SKILL.md ~/.config/anacleto/skills/ --full-path

# Agentes disponibles
fd .md ~/.config/anacleto/agents/ --full-path
```

### 2. Buscar por palabra clave (nombre, descripción, dominio)

```bash
rg -l "nombre-del-skill|palabra-clave|dominio" .agents/skills/ ~/.config/anacleto/skills/
```

### 3. Inspeccionar el frontmatter de una skill

```bash
# Ver solo el frontmatter YAML de una skill
head -20 .agents/skills/<nombre>/SKILL.md
```

O mejor, parsear el frontmatter con herramientas estructuradas:

```bash
# Extraer nombre y descripción de todas las skills del proyecto
for f in .agents/skills/*/SKILL.md; do
  name=$(head -1 "$f" | rg -o '(?<=name: ).*')
  desc=$(head -2 "$f" | rg -o '(?<=description: ).*')
  echo "$name: $desc"
done
```

---

## Cómo buscar skills en skills.sh (registro público de mattpocock)

El repo [mattpocock/skills](https://github.com/mattpocock/skills) contiene ~51 skills
organizadas en 5 categorías:

| Categoría | Contenido |
|---|---|
| `engineering` | Skills técnicas: code-review, tdd, implement, research, grill-with-docs, etc. |
| `in-progress` | Skills en desarrollo: handoff, loop-me, writing-beats, etc. |
| `misc` | Skills varios: git-guardrails, migrate-to-shoehorn, scaffold-exercises, setup-pre-commit |
| `productivity` | Skills de productividad: grill-me, grilling, teach, wait-what, writing-for-agents |
| `deprecated` | Skills obsoletas (solo README) |

### 1. Listar todas las skills disponibles (por categoría)

Usa la GitHub API (sin auth para repos públicos):

```bash
# Listar skills de engineering
curl -sL "https://api.github.com/repos/mattpocock/skills/contents/skills/engineering" \
  | jq -r '.[] | select(.type == "dir") | .name'

# Listar skills de todas las categorías a la vez
for cat in engineering in-progress misc productivity; do
  echo "=== $cat ==="
  curl -sL "https://api.github.com/repos/mattpocock/skills/contents/skills/$cat" \
    | jq -r '.[] | select(.type == "dir") | .name'
done
```

### 2. Buscar por palabra clave en los SKILL.md remotos

```bash
# Buscar una palabra clave en los nombres y descripciones de todas las skills remotas
for cat in engineering in-progress misc productivity; do
  for skill in $(curl -sL "https://api.github.com/repos/mattpocock/skills/contents/skills/$cat" \
    | jq -r '.[] | select(.type == "dir") | .name'); do
    
    frontmatter=$(curl -sL "https://raw.githubusercontent.com/mattpocock/skills/main/skills/$cat/$skill/SKILL.md" \
      | head -5)
    
    if echo "$frontmatter" | rg -qi "palabra-clave"; then
      echo "→ $skill ($cat): $(echo "$frontmatter" | rg -o '(?<=description: ).*')"
    fi
  done
done
```

> [!TIP]
> Para búsquedas rápidas, usa el comando resumen que lista nombre + descripción de todas las skills
> de mattpocock en un solo paso (ver sección "Resumen rápido de skills.sh").

### 3. Ver el detalle de una skill específica

```bash
# Ver el SKILL.md completo de una skill remota
curl -sL "https://raw.githubusercontent.com/mattpocock/skills/main/skills/<categoria>/<skill>/SKILL.md"
```

Ejemplo para `grill-with-docs`:

```bash
curl -sL "https://raw.githubusercontent.com/mattpocock/skills/main/skills/engineering/grill-with-docs/SKILL.md"
```

### 4. Ver los agentes asociados a una skill (si tiene)

Algunas skills en el repo de mattpocock incluyen una carpeta `agents/` con
configuraciones de agente de ejemplo:

```bash
# Ver si una skill tiene agentes asociados
curl -sL "https://api.github.com/repos/mattpocock/skills/contents/skills/<categoria>/<skill>/agents" \
  | jq -r '.[].name'
```

---

## Resumen rápido de skills.sh

Para obtener una vista rápida de todas las skills de mattpocock con su nombre
y descripción, ejecuta:

```bash
for cat in engineering in-progress misc productivity; do
  for skill in $(curl -sL "https://api.github.com/repos/mattpocock/skills/contents/skills/$cat" \
    | jq -r '.[] | select(.type == "dir") | .name'); do
    desc=$(curl -sL "https://raw.githubusercontent.com/mattpocock/skills/main/skills/$cat/$skill/SKILL.md" \
      | head -5 | rg -o '(?<=description: ).*')
    echo "📦 $skill  ($cat): $desc"
  done
done
```

---

## Cómo instalar/adaptar una skill de skills.sh a Anacleto

Las skills del repo de mattpocock usan el mismo formato que Anacleto
(SKILL.md con frontmatter YAML), por lo que la adaptación es directa:

### Opción A: Copiar la skill al proyecto

```bash
# 1. Crear el directorio para la skill
mkdir -p .agents/skills/<nombre>

# 2. Descargar el SKILL.md remoto
curl -sL "https://raw.githubusercontent.com/mattpocock/skills/main/skills/<categoria>/<nombre>/SKILL.md" \
  > .agents/skills/<nombre>/SKILL.md

# 3. (Opcional) Si tiene agentes asociados, descargarlos también
curl -sL "https://api.github.com/repos/mattpocock/skills/contents/skills/<categoria>/<nombre>/agents" \
  | jq -r '.[].name' | while read agent_file; do
    curl -sL "https://raw.githubusercontent.com/mattpocock/skills/main/skills/<categoria>/<nombre>/agents/$agent_file" \
      > .agents/skills/<nombre>/$agent_file
  done
```

### Opción B: Copiar la skill a global (~/.agents/skills/)

```bash
mkdir -p ~/.agents/skills/<nombre>
curl -sL "https://raw.githubusercontent.com/mattpocock/skills/main/skills/<categoria>/<nombre>/SKILL.md" \
  > ~/.agents/skills/<nombre>/SKILL.md
```

> [!IMPORTANT]
> Algunas skills de mattpocock usan `disable-model-invocation: true` en su frontmatter.
> Esto significa que la skill no invoca el modelo directamente, sino que ejecuta
> otra skill (por ejemplo, `grill-with-docs` ejecuta `/grilling` y `/domain-modeling`).
> En Anacleto esto se traduce como: la skill delega a otras skills, asegúrate de
> que las skills referenciadas también estén instaladas.

---

## Flujo recomendado (ampliado)

### Paso 1: Entender qué necesita el usuario

Identifica:

1. **El dominio** (desarrollo web, testing, devops, documentación, etc.)
2. **La tarea específica** (escribir tests, crear documentación, revisar PRs)
3. **Si es una tarea común** para la que probablemente exista una skill

### Paso 2: Buscar en las skills del proyecto

```bash
# Buscar por dominio en descripciones y contenido
rg -il "testing|test" .agents/skills/*/SKILL.md
```

### Paso 3: Buscar en las skills globales

```bash
# Si existe el directorio global
rg -il "testing|test" ~/.config/anacleto/skills/*/SKILL.md 2>/dev/null

# También buscar en ~/.agents/skills/ (ruta alternativa donde tengas skills instaladas)
rg -il "testing|test" ~/.agents/skills/*/SKILL.md 2>/dev/null
```

### Paso 4: Buscar en skills.sh (registro público)

```bash
# Búsqueda rápida por palabra clave en todas las categorías a la vez
for cat in engineering in-progress misc productivity; do
  for skill in $(curl -sL "https://api.github.com/repos/mattpocock/skills/contents/skills/$cat" \
    | jq -r '.[] | select(.type == "dir") | .name' 2>/dev/null); do
    desc=$(curl -sL "https://raw.githubusercontent.com/mattpocock/skills/main/skills/$cat/$skill/SKILL.md" \
      | head -5 | rg -o '(?<=description: ).*' 2>/dev/null)
    if echo "$skill $desc" | rg -qi "testing|test|palabra-clave"; then
      echo "📦 $skill ($cat): $desc"
    fi
  done
done
```

### Paso 4b: Buscar en la web con SearXNG

Si las skills locales y skills.sh no tienen lo que buscas, **usa `searxng-search` para buscar skills en la web**:

| Escenario | Búsqueda recomendada | Categoría |
|---|---|---|
| Skill técnica (testing, devops, deploy) | `anthropics/skills [dominio]` | `general,it,repos` |
| Skill de productividad | `skills.sh [acción]` | `general` |
| Alternativas a skills conocidas | `[tarea] AI agent skill` | `general,news` |
| Inspiración para crear skill nueva | `[dominio] best practices guide` | `general` |
| Skills de terceros en GitHub | `site:github.com skill [dominio]` | `general,it` |

#### Proceso

1. **Busca** con `searxng-search` usando los términos según el dominio.
2. **Revisa** los resultados: repos de GitHub, blogs técnicos, documentación.
3. Si encuentras una skill publicada, **fetchea** el contenido con `web-research`.
4. **Evalúa** si es compatible con formato Anacleto (Markdown + YAML frontmatter).
5. Si no existe una skill específica, **informa al usuario** y sugiere crear una con `skill-creator`.

> 💡 **Ejemplo real:** Para crear `seo-optimizer`, no encontramos skills existentes. Investigamos con SearXNG las mejores prácticas (Search Engine Land, Ahrefs, Moz) y creamos la skill desde cero con datos reales.

### Paso 5: Mostrar resultados al usuario

Para cada skill encontrada (local o remota), muestra:

```
📦 <nombre>
   📝 <descripción>
   📍 <ruta o fuente>
   🏷️  <categoría> (del metadata)
```

Si la skill es remota y el usuario quiere instalarla, ofrece hacerlo con
el comando de instalación.

---

## Notas importantes

1. **Límites de GitHub API**: Para repos públicos sin autenticación, el límite es
   60 requests/hora. Para búsquedas intensivas, considera añadir `?client_id=...`
   o usar un token personal.

2. **skills.sh vs GitHub**: El sitio skills.sh es una interfaz Next.js sobre el
   repo de GitHub. Siempre prefiere GitHub raw content para los SKILL.md, es
   más rápido y no tiene restricciones de CORS.

3. **Formato compatible**: Las skills de mattpocock usan el mismo formato que
   Anacleto (Markdown + frontmatter YAML), por lo que son directamente
   compatibles. Solo necesitas copiarlas a la ruta local.

4. **Dependencias entre skills**: Algunas skills de mattpocock referencian a otras
   (ej: `grill-with-docs` ejecuta `grilling` y `domain-modeling`). Asegúrate de
   instalar también las skills dependientes.
