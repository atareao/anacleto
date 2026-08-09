---
name: weather
description: |
  Consulta meteorológica para cualquier localidad del mundo. Proporciona temperatura actual, sensación térmica,
  humedad, viento, probabilidad de lluvia, estado del cielo, amanecer/anochecer y previsión por horas/días.
  Úsala cuando el usuario pregunte "qué tiempo hace", "previsión meteorológica", "lloverá mañana",
  "hace frío/calor en X", o cualquier consulta sobre el clima.
metadata:
  version: "2.0"
  category: research
  risk: low
  source: wttr.in (WorldWeatherOnline)
  capabilities: "pronóstico_multi_día, auto_detección_ip, alertas_implícitas (chanceofthunder, chanceofwindy, chanceoffog, chanceofrain)"
  scripts: "weather.sh"
  triggers: "qué tiempo hace; previsión meteorológica; va a llover; temperatura en; clima en; hace frío; hace calor; tiempo hoy; tiempo esta semana"
---

# Weather Skill

Skill para consultar el tiempo meteorológico actual y la previsión desde cualquier ubicación del mundo.

## Fuente de datos

Usa la API pública de **wttr.in** (https://wttr.in) que no requiere clave API y devuelve datos estructurados en JSON. Los datos provienen de WorldWeatherOnline.

### ENDPOINT

```
https://wttr.in/{localidad},{pais}?lang=es&format=j1
```

| Parámetro | Descripción |
|---|---|
| `{localidad}` | Nombre de la ciudad/pueblo o coordenadas (lat,lon). Vacío = auto-detección por IP |
| `{pais}` | Nombre del país (opcional, mejora la precisión) |
| `lang=es` | Respuestas en español |
| `format=j1` | Formato JSON estructurado (hasta 3 días de previsión) |

## Script auxiliar

La skill incluye un script shell en `.agents/skills/weather/weather.sh`
que encapsula la llamada a la API de wttr.in.

### Características del script

- **No requiere API key**
- **Auto-detección por IP**: si no se pasa ubicación, detecta automáticamente dónde estás
- **Pronóstico multi-día**: flag `-d N` para obtener N días de previsión (1-3)
- **Maneja codificación de URLs** automáticamente
- **Usa `curl` o `wget`** (el que esté disponible)
- **Timeout de 10 segundos** para no bloquearse
- **Ayuda integrada** con `-h` o `--help`

### Uso desde terminal

```bash
# Auto-detección (hoy)
.agents/skills/weather/weather.sh

# Auto-detección, 3 días
.agents/skills/weather/weather.sh -d 3

# Ubicación específica, hoy
.agents/skills/weather/weather.sh "Catarroja,Valencia"

# Ubicación específica, 3 días
.agents/skills/weather/weather.sh -d 3 "Madrid"

# Por coordenadas
.agents/skills/weather/weather.sh "41.3874,2.1686"

# Ayuda
.agents/skills/weather/weather.sh -h
```

## Comportamiento

Cuando recibas una petición de tiempo meteorológico, sigue estos pasos:

### 1. Identificar la ubicación

Extrae la ciudad/región de la consulta del usuario. Si no se menciona ninguna ubicación, **usa auto-detección por IP** (no preguntes, actívala automáticamente). Si es una ciudad pequeña o hay ambigüedad, incluye el país para mejorar la precisión. También acepta coordenadas en formato `lat,lon`.

### 2. Determinar el alcance de la previsión

- Si el usuario pregunta por **hoy** o **ahora** → `-d 1`
- Si pregunta por **mañana** o **esta semana** → `-d 3`
- Por defecto → `-d 1`
- Si pregunta explícitamente "próximos X días" (y X <= 3) → `-d X`

### 3. Obtener los datos (prioridad)

#### Opción A (recomendada) — Usar el script shell

Ejecuta el script con `shell` según el caso:

```bash
# Sin ubicación (auto-detección)
.agents/skills/weather/weather.sh

# Sin ubicación, 3 días
.agents/skills/weather/weather.sh -d 3

# Con ubicación, hoy
.agents/skills/weather/weather.sh "LOCALIDAD,PAIS"

# Con ubicación, varios días
.agents/skills/weather/weather.sh -d 3 "LOCALIDAD,PAIS"
```

#### Opción B (fallback) — Usar webfetch

Si el script no está disponible o falla, usa `webfetch` directamente:

```
https://wttr.in/LOCALIDAD?lang=es&format=j1
```

Ejemplos:
- `https://wttr.in/Silla,Valencia?lang=es&format=j1`
- `https://wttr.in/Madrid?lang=es&format=j1`
- `https://wttr.in/41.3874,2.1686?lang=es&format=j1`

### 4. Interpretar la respuesta

La API devuelve JSON con esta estructura relevante:

```json
{
  "current_condition": [{
    "temp_C": "28",
    "FeelsLikeC": "30",
    "humidity": "45",
    "weatherDesc": [{"value": "Soleado"}],
    "winddir16Point": "ENE",
    "windspeedKmph": "15",
    "precipMM": "0.0",
    "visibility": "10",
    "uvIndex": "7"
  }],
  "weather": [{
    "date": "2025-08-09",
    "astronomy": [{"sunrise": "07:08", "sunset": "21:12"}],
    "maxtempC": "32",
    "mintempC": "22",
    "hourly": [
      {
        "time": "100",
        "tempC": "23",
        "FeelsLikeC": "23",
        "chanceofrain": "10"
      }
    ]
  }]
}
```

### 5. Campos clave a extraer y formatear

| Campo | Ubicación | Descripción |
|---|---|---|
| temp_C | current_condition[0] | Temperatura actual |
| FeelsLikeC | current_condition[0] | Sensación térmica |
| humidity | current_condition[0] | Humedad relativa % |
| weatherDesc[].value | current_condition[0] | Descripción del cielo |
| winddir16Point | current_condition[0] | Dirección del viento |
| windspeedKmph | current_condition[0] | Velocidad del viento km/h |
| precipMM | current_condition[0] | Precipitación mm |
| visibility | current_condition[0] | Visibilidad km |
| uvIndex | current_condition[0] | Índice UV |
| sunrise | weather[].astronomy[0] | Amanecer |
| sunset | weather[].astronomy[0] | Anochecer |
| maxtempC | weather[0] | Máxima del día |
| mintempC | weather[0] | Mínima del día |
| hourly[].time | weather[].hourly[] | Hora (formato 24h, 100 = 01:00) |
| hourly[].tempC | weather[].hourly[] | Temperatura por hora |
| hourly[].chanceofrain | weather[].hourly[] | Probabilidad lluvia % |
| hourly[].FeelsLikeC | weather[].hourly[] | Sensación térmica por hora |

### 6. Alertas implícitas

Revisa los siguientes campos en los datos horarios (`weather[].hourly[]`):

- **`chanceofthunder`** ≥ 30 → Posibilidad de tormentas
- **`chanceofwindy`** ≥ 50 → Rachas de viento
- **`chanceoffog`** ≥ 40 → Niebla/visibilidad reducida
- **`chanceofrain`** ≥ 60 → Alta probabilidad de lluvia

Si se detecta alguna, menciónala al usuario.

### 7. Formatear la respuesta

Presenta la información de forma amigable y bien estructurada:

```
🌤  Tiempo en [Ubicación]

Ahora: 28°C (sensación 30°C)
☁️  Estado: Soleado
💧  Humedad: 45%
🌬  Viento: ENE 15 km/h
🌧  Lluvia: 0.0 mm
👁  Visibilidad: 10 km
☀️  Índice UV: 7

📅  Pronóstico:
  • Sábado 9: 22°C / 32°C  ☀️  Amanecer 07:08 · Atardecer 21:12
```

## Ejemplos de uso

### Ejemplo 1: Consulta sin ubicación (auto-detección)

> Usuario: "Qué tiempo hace?"

Ejecutas: `.agents/skills/weather/weather.sh`

Respuesta:
```
🌤  Tiempo en Catarroja, Valencia

Ahora: 30°C (sensación 31°C)
☁️  Estado: Parcialmente nublado
💧  Humedad: 40%
🌬  Viento: Este 12 km/h
🌧  Lluvia: 0.0 mm

📅  Pronóstico:
  • Sábado 9: 23°C / 32°C  ☀️  Amanecer 07:10 · Atardecer 21:08
```

### Ejemplo 2: Consulta con ubicación y varios días

> Usuario: "Lloverá mañana en Madrid?"

Ejecutas: `.agents/skills/weather/weather.sh -d 3 "Madrid"`

Respuesta:
```
🌤  Tiempo en Madrid

Ahora: 26°C (sensación 24°C)
☁️  Estado: Soleado
💧  Humedad: 30%
🌬  Viento: Sur 8 km/h

📅  Pronóstico:
  • Sábado 9: 20°C / 33°C  ☀️  Amanecer 07:15 · Atardecer 21:30
  • Domingo 10: 21°C / 30°C  🌧  Prob. lluvia: 60%  ⚠️ Posibles tormentas
  • Lunes 11: 18°C / 22°C  🌧  Prob. lluvia: 80%

⚠️  Alertas:
  • Domingo: posibilidad de tormentas
```

### Ejemplo 3: Coordenadas y alertas

> Usuario: "Tiempo en 41.3874,2.1686"

Ejecutas: `.agents/skills/weather/weather.sh "41.3874,2.1686"`

## Notas técnicas

- **Límite de peticiones**: wttr.in permite ~1000 peticiones diarias por IP
- **Cache**: la API cachea resultados ~15 minutos
- **Sin API key**: no requiere registro
- **Tiempo de espera**: timeout del script a 10 segundos
- **Idioma**: las respuestas llegan en español (parámetro `lang=es`)
- **Coordenadas**: acepta formato `lat,lon` sin espacios
- **Sin ubicación**: no preguntes al usuario, activa auto-detección automáticamente
