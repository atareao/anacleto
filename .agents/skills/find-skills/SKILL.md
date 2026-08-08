---
name: find-skills
description: |
  Busca y descubre skills instaladas localmente en el ecosistema Anacleto.
  Úsala cuando el usuario pregunte "cómo hago X", "busca una skill para X",
  "hay una skill que pueda...", o quiera extender las capacidades del agente.
metadata:
  version: "1.0"
  category: system
  risk: low
---

# Find Skills (Anacleto)

Esta skill te ayuda a **descubrir skills ya instaladas** en el ecosistema Anacleto
y a **encontrar nuevas skills** disponibles en registros públicos.

A diferencia del ecosistema `vercel-labs/skills` (que usa `npx skills`), Anacleto
gestiona las skills como ficheros Markdown con frontmatter YAML en directorios
locales. Esta skill adapta el concepto de `find-skills` al modelo de Anacleto.

---

## ¿Dónde se buscan las skills?

Anacleto busca skills en estas rutas, por orden de prioridad:

| Ruta | Ámbito | Descripción |
|---|---|---|
| `.agents/skills/<name>/SKILL.md` | Proyecto | Skills del proyecto actual |
| `~/.config/anacleto/skills/<name>/SKILL.md` | Global (usuario) | Skills globales del usuario |
| `~/.config/anacleto/agents/<name>.md` | Global (agentes) | Definiciones de agentes (también tienen frontmatter) |

---

## Cómo buscar skills instaladas

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

## Flujo recomendado

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
```

### Paso 4: Mostrar resultados al usuario

Para cada skill encontrada, muestra:

```
📦 <nombre>
   📝 <descripción>
   📍 <ruta>
   🏷️  <categoría> (del metadata)
```

---

## Instalar una nueva skill

Anacleto no tiene un CLI tipo `npx skills add`. Para instalar una skill nueva:

### Opción A: Desde vercel-labs/skills (fuente externa)

1. Encuentra la skill en https://github.com/vercel-labs/skills/tree/main/skills
2. Examina su `SKILL.md` (o `SKILL.md`) para entender qué hace
3. Crea el directorio local y copia/adapta el contenido:

```bash
# Crear directorio para la skill en el proyecto
mkdir -p .agents/skills/<nombre>

# Crear el fichero SKILL.md con el frontmatter adaptado
cat > .agents/skills/<nombre>/SKILL.md << 'EOF'
---
name: <nombre>
description: <descripción adaptada>
metadata:
  version: "1.0"
  category: <categoría>
  risk: <bajo|medio|alto>
---

# <Nombre>

Contenido adaptado de la skill original...
EOF
```

4. Añadir la skill al agente correspondiente en `.agents/agents/<nombre>.md`:

```yaml
skills:
  - .agents/skills/<nombre>/
```

### Opción B: Crear una skill desde cero

Usa la plantilla estándar de Anacleto:

```markdown
---
name: mi-skill
description: Describe brevemente qué hace
metadata:
  version: "1.0"
  category: development  # system | development | research | productivity
  risk: medium           # low | medium | high
---

# Mi Skill

Instrucciones detalladas...

## Uso

Describe cómo debe usarse esta skill...

## Ejemplos

Incluye ejemplos prácticos...
```

---

## Categorías comunes de búsqueda

| Categoría | Palabras clave sugeridas |
|---|---|
| **Desarrollo web** | react, nextjs, typescript, css, tailwind |
| **Testing** | testing, test, jest, playwright, e2e |
| **DevOps** | deploy, docker, kubernetes, ci-cd |
| **Documentación** | docs, readme, changelog, api-docs |
| **Calidad de código** | review, lint, refactor, best-practices |
| **Diseño** | ui, ux, design-system, accesibilidad |
| **Productividad** | workflow, automation, git |
| **Sistema** | shell, filesystem, permissions, config |

---

## Cuándo no se encuentra una skill

Si no existe una skill para lo que el usuario necesita:

1. **Reconoce** que no se encontró ninguna skill existente
2. **Ofrece** ayudar directamente con las capacidades generales del agente
3. **Sugiere** crear una skill nueva siguiendo la plantilla de Anacleto

```
No encontré ninguna skill para "X" en las skills instaladas.
Puedo ayudarte directamente con esta tarea. Si es algo que haces a menudo,
podemos crear una skill personalizada para Anacleto.
```

---

## Consejos para búsquedas efectivas

1. **Usa palabras clave específicas**: "react testing" es mejor que solo "testing"
2. **Prueba términos alternativos**: si "desplegar" no funciona, prueba "deploy" o "ci-cd"
3. **Revisa el frontmatter**: la descripción y metadata suelen tener las pistas clave
4. **Busca también en agentes**: los agentes (`agents/*.md`) también tienen frontmatter y pueden contener skills embebidas
