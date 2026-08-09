# Tests para la Weather Skill (v2.0)

## Test 1 — Auto-detección por IP

```yaml
task: "¿Qué tiempo hace?"
```

**Resultado esperado:**
- NO debe preguntar "¿en qué ciudad?"
- Debe activar auto-detección automáticamente
- Mostrar datos de la ubicación detectada

---

## Test 2 — Consulta básica con ubicación

```yaml
task: "¿Qué tiempo hace en Silla, Valencia?"
```

**Resultado esperado:**
- Usa el script `weather.sh` (Opción A)
- ☀️ Estado soleado con datos actuales
- Predicción por horas del día
- Máximas ~36 °C, sin lluvia
- Emojis meteorológicos correctos

---

## Test 3 — Pronóstico a varios días

```yaml
task: "Previsión para Barcelona para los próximos 3 días"
```

**Resultado esperado:**
- Usa `weather.sh -d 3 "Barcelona"`
- Datos de hoy + 2 días adicionales
- Tabla con Máx/Mín/Lluvia por día
- Sección "Pronóstico de los próximos días"

---

## Test 4 — Coordenadas geográficas

```yaml
task: "Tiempo en 41.3874, 2.1686"
```

**Resultado esperado:**
- Usa el script `weather.sh` (Opción A)
- Debe resolver las coordenadas a Barcelona
- Mostrar tiempo actual y previsión

---

## Test 5 — Consulta internacional

```yaml
task: "Qué tiempo hace en Londres"
```

**Resultado esperado:**
- Usa el script `weather.sh` (Opción A)
- Datos de Londres, Reino Unido
- Predicción en °C (no °F)
- Descripciones en español

---

## Test 6 — Alertas implícitas (tormenta, viento, niebla)

```yaml
task: "Va a llover en Valencia esta tarde?"
```

**Resultado esperado:**
- Consulta `weather.sh "Valencia"`
- Si `chanceofrain > 60%` o `chanceofthunder > 30%` → muestra ⚠️ Alertas
- Si no superan umbral, no muestra sección de alertas

---

## Test 7 — Script no disponible (fallback a webfetch)

```yaml
task: "Qué tiempo hace en Bilbao"
```

**Resultado esperado:**
- Si `weather.sh` no se puede ejecutar, debe caer en Opción B (webfetch)
- Datos correctos de Bilbao igualmente

---

## Test 8 — Ubicación desconocida (edge case)

```yaml
task: "Tiempo en AquíNoExiste"
```

**Resultado esperado:**
- La API devolverá la localidad más cercana
- Debe indicar al usuario que no se encontró exactamente

---

## Test 9 — Consulta ambigua sin ubicación

```yaml
task: "Hará buen tiempo mañana?"
```

**Resultado esperado:**
- Debe activar auto-detección, NO preguntar
- Mostrar previsión para la ubicación detectada, con `-d 3`

---

## Test 10 — Ayuda del script

```yaml
task: "Ejecuta el script de weather con -h para ver la ayuda"
```

**Resultado esperado:**
- Muestra el mensaje de ayuda del script
- Explica flags -d, -h
