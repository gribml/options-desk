#!/usr/bin/env bash
# Fetch 1-minute bars for a given symbol from Alpaca, paginating automatically.
# Usage: ./bars-1min.sh <symbol> [start-date] [end-date] [output-file]
#   start-date and end-date are YYYY-MM-DD (default: 30 days ago to today)
#   output-file: path to save JSON array of all bars (default: print to stdout)
#
# Credentials are read from environment variables ALPACA_API_KEY and ALPACA_API_SECRET,
# or from a .env file in the project root (one directory up from this script).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENV_FILE="$SCRIPT_DIR/../.env"

# Load .env if env vars are not already set
if [[ -z "${ALPACA_API_KEY:-}" || -z "${ALPACA_API_SECRET:-}" ]]; then
  if [[ -f "$ENV_FILE" ]]; then
    set -a
    # shellcheck disable=SC1090
    source "$ENV_FILE"
    set +a
  fi
fi

if [[ -z "${ALPACA_API_KEY:-}" || -z "${ALPACA_API_SECRET:-}" ]]; then
  echo "Error: ALPACA_API_KEY and ALPACA_API_SECRET must be set in environment or .env file" >&2
  exit 1
fi

SYMBOL="${1:-}"
if [[ -z "$SYMBOL" ]]; then
  echo "Usage: $0 <symbol> [start-date] [end-date] [output-file]  e.g. $0 AAPL 2024-01-01 2024-01-02 bars.json" >&2
  exit 1
fi

SYMBOL="${SYMBOL^^}"  # uppercase

# Default date range: 30 days ago to today
if [[ "$(uname)" == "Darwin" ]]; then
  DEFAULT_START=$(date -v-30d +%Y-%m-%d)
else
  DEFAULT_START=$(date -d "30 days ago" +%Y-%m-%d)
fi
DEFAULT_END=$(date +%Y-%m-%d)

START="${2:-$DEFAULT_START}"
END="${3:-$DEFAULT_END}"
OUTPUT="${4:-}"

BASE_URL="https://data.alpaca.markets/v2/stocks/${SYMBOL}/bars"
HEADERS=(-H "APCA-API-KEY-ID: ${ALPACA_API_KEY}" -H "APCA-API-SECRET-KEY: ${ALPACA_API_SECRET}")

echo "Fetching 1Min bars for $SYMBOL from $START to $END..." >&2

# Accumulate all bars across pages into a temp file as a JSON array
TMPFILE=$(mktemp)
trap 'rm -f "$TMPFILE"' EXIT

echo "[]" > "$TMPFILE"

URL="${BASE_URL}?timeframe=1Min&start=${START}&end=${END}&feed=sip&sort=asc&limit=1000"
PAGE=1

while [[ -n "$URL" ]]; do
  echo "  Page $PAGE..." >&2

  RESPONSE=$(curl -s "${HEADERS[@]}" "$URL")

  # Append this page's bars to accumulated array
  TMPFILE2=$(mktemp)
  jq -s '.[0] + .[1].bars' "$TMPFILE" <(echo "$RESPONSE") > "$TMPFILE2"
  mv "$TMPFILE2" "$TMPFILE"

  # Get next_page_token; if null or empty, we're done
  NEXT_TOKEN=$(echo "$RESPONSE" | jq -r '.next_page_token // empty')

  if [[ -n "$NEXT_TOKEN" ]]; then
    URL="${BASE_URL}?timeframe=1Min&start=${START}&end=${END}&feed=sip&sort=asc&limit=1000&page_token=${NEXT_TOKEN}"
    PAGE=$((PAGE + 1))
  else
    URL=""
  fi
done

TOTAL=$(jq 'length' "$TMPFILE")
echo "Done — $TOTAL bars fetched." >&2

if [[ -n "$OUTPUT" ]]; then
  jq '.' "$TMPFILE" > "$OUTPUT"
  echo "Saved to $OUTPUT" >&2
else
  jq '.' "$TMPFILE"
fi
