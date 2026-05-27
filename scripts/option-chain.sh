#!/usr/bin/env bash
# Fetch the full option chain for a given underlying symbol from Alpaca.
# Usage: ./option-chain.sh AAPL
#
# Credentials are read from environment variables ALPACA_API_KEY and ALPACA_SECRET,
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
  echo "Usage: $0 <symbol>  e.g. $0 AAPL" >&2
  exit 1
fi

SYMBOL="${SYMBOL^^}"  # uppercase

URL="https://data.alpaca.markets/v1beta1/options/snapshots/${SYMBOL}?feed=indicative&limit=1000"

echo "Fetching option chain for $SYMBOL..." >&2

curl -s \
  -H "APCA-API-KEY-ID: ${ALPACA_API_KEY}" \
  -H "APCA-API-SECRET-KEY: ${ALPACA_API_SECRET}" \
  "$URL" \
  | jq '.' 2>/dev/null || cat
