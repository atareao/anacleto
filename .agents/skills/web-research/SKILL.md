---
name: web-research
description: Investiga cualquier tema combinando búsqueda web con SearXNG y fetch de URLs — encuentra fuentes, las analiza y sintetiza un informe estructurado
metadata:
  version: "2.0"
  category: research
  risk: low
---

# Web Research skill

Skill híbrida de investigación que combina **búsqueda web** (SearXNG) y **fetch de URLs**
para producir informes estructurados sobre cualquier tema.

## Flujo de trabajo

```
1. BÚSQUEDA (si no hay URL concreta)
   → searxng-search para encontrar fuentes relevantes
   
2. FETCH (de las fuentes más relevantes)
   → webfetch para leer el contenido de cada URL
   
3. SÍNTESIS
   → Informe estructurado en Markdown
```

## Cuándo usar esta skill

- **Tengo una URL** → Fetch directo y resumen.
- **Tengo un tema** → Búsqueda SearXNG + fetch de los mejores resultados + síntesis.
- **Investigar un tema nuevo** → Búsqueda multi-categoría (general + news + science) + fetch + síntesis.

---

## Comportamiento detallado

### Si el usuario proporciona URL(s) concretas

1. Usa `webfetch` para obtener el contenido de cada URL.
2. Sintetiza la información en un resumen Markdown.
3. Si la URL es documentación técnica, incluye API signatures, ejemplos de código y enlaces relacionados.

### Si el usuario describe un tema (sin URL)

1. **Buscar** con `searxng-search`:
   - Categoría `general` para resultados web amplios.
   - Si el tema es técnico: añadir `it,repos` para GitHub/Docker Hub.
   - Si el tema es de actualidad: añadir `news` con `time_range=week`.
   - Si el tema es científico: añadir `science`.
   - Idioma acorde al tema (`es` para español, `en` para inglés).
   - Si los primeros resultados no son suficientes, refinar la búsqueda con términos más específicos.

2. **Seleccionar** las 2-3 URLs más prometedoras (por relevancia, autoridad de la fuente).

3. **Fetchear** cada URL con `webfetch`.

4. **Sintetizar** en un informe Markdown estructurado.

### Para investigaciones complejas (multi-tema)

1. Descomponer el tema en subtemas.
2. Buscar cada subtema por separado (puede requerir varias iteraciones).
3. Fetchear las fuentes más relevantes de cada subtema.
4. Sintetizar todo en un informe unificado.

---

## Output

Siempre en Markdown, con esta estructura cuando sea aplicable:

```markdown
# Informe: [Tema investigado]

## Resumen ejecutivo
[2-3 frases con lo más importante]

## Hallazgos principales
- **Punto 1**: explicación con fuentes
- **Punto 2**: explicación con fuentes

## Detalle por fuente

### [Título de la fuente 1]
[URL]
[Resumen del contenido relevante]

### [Título de la fuente 2]
[URL]
[Resumen del contenido relevante]

## Conclusiones
[Qué se puede concluir de la investigación]

## Enlaces adicionales
- [Fuente 1](url)
- [Fuente 2](url)
```

## Ejemplos

```yaml
task: "Investiga las novedades de Rust edition 2024"
```
→ Busca con SearXNG → fetchea los resultados top → sintetiza informe

```yaml
task: "Fetchea la documentación de tokio mpsc: https://docs.rs/tokio/latest/tokio/sync/mpsc/index.html"
```
→ Fetch directo + resumen con API signatures y ejemplos

```yaml
task: |
  Investiga las mejores prácticas de SEO on-page en 2025:
  - Title tags
  - Meta descriptions
  - Estructura de encabezados
```
→ Descompone en subtemas → busca cada uno → fetchea → sintetiza informe

---

## Buenas prácticas

- Para URLs concretas de documentación, prioriza fuentes oficiales.
- Para temas técnicos, busca en `general,it` y prioriza docs oficiales, GitHub y tutoriales contrastados.
- Para verificar datos concretos (versiones, fechas), busca con las keywords exactas.
- Si la primera ronda de búsqueda no da resultados de calidad, refina los términos.
- No te limites a la primera página de resultados — si el tema es complejo, profundiza.
- Cuando fetchees documentación técnica, extrae API signatures, firmas de funciones y ejemplos de código.

## Limitaciones

- No ejecuta comandos ni modifica archivos.
- Depende de la disponibilidad de la instancia SearXNG y de los motores que tenga configurados.
- Algunos motores pueden fallar (CAPTCHA, rate limiting) — en ese caso, intenta con otras categorías o términos.
