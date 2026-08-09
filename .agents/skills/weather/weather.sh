#!/usr/bin/env bash
#
# weather.sh — Consulta meteorológica vía wttr.in
#
# Uso:
#   ./weather.sh                # Auto-detección por IP
#   ./weather.sh "Silla,Valencia"
#   ./weather.sh "41.3874,2.1686"
#   ./weather.sh -d 3           # Próximos 3 días (auto-detección)
#   ./weather.sh -d 3 "Madrid"
#
# Requisitos: curl o wget, jq (opcional, para formatear)

set -euo pipefail

# --- Configuración por defecto ---
LOCATION=""
DAYS=1

# --- Parsear argumentos ---
while [[ $# -gt 0 ]]; do
  case "$1" in
    -d|--days)
      DAYS="$2"
      shift 2
      ;;
    -h|--help)
      echo "Uso: $0 [-d DÍAS] [UBICACIÓN]"
      echo ""
      echo "Argumentos:"
      echo "  -d, --days DÍAS   Número de días de previsión (1-3, por defecto 1)"
      echo "  -h, --help        Muestra esta ayuda"
      echo ""
      echo "Ejemplos:"
      echo "  $0                        # Auto-detección por IP, hoy"
      echo "  $0 -d 3                   # Auto-detección, 3 días"
      echo "  $0 \"Silla,Valencia\"      # Ubicación específica"
      echo "  $0 -d 3 \"Madrid\"         # Madrid, 3 días"
      echo "  $0 \"41.3874,2.1686\"      # Por coordenadas"
      exit 0
      ;;
    *)
      LOCATION="$1"
      shift
      ;;
  esac
done

# --- Límite de días ---
if [[ "$DAYS" -lt 1 ]]; then DAYS=1; fi
if [[ "$DAYS" -gt 3 ]]; then DAYS=3; fi

# --- Construir URL ---
BASE_URL="https://wttr.in"

if [[ -n "$LOCATION" ]]; then
  # Codificar URL
  ENCODED=$(python3 -c "import urllib.parse; print(urllib.parse.quote('''$LOCATION'''))" 2>/dev/null \
    || echo "$LOCATION" | sed 's/ /%20/g; s/,/%2C/g')
  URL="${BASE_URL}/${ENCODED}?lang=es&format=j1"
else
  URL="${BASE_URL}/?lang=es&format=j1"
fi

# --- Descargar JSON ---
if command -v curl &>/dev/null; then
  DATA=$(curl -sS --max-time 10 "$URL")
else
  DATA=$(wget -q -O- --timeout=10 "$URL")
fi

# --- Extraer solo los días solicitados ---
# wttr.in devuelve hasta 3 días. Filtramos para quedarnos solo con DAYS días.
echo "$DATA" | python3 -c "
import json, sys

data = json.load(sys.stdin)

# Limitar los días de previsión solicitados
days = int($DAYS)
if 'weather' in data:
    data['weather'] = data['weather'][:days]

print(json.dumps(data, ensure_ascii=False))
"
