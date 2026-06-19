#!/usr/bin/env bash
# Apply (or re-apply) the Cloudflare D1 schema.
# Safe to re-run: all statements use IF NOT EXISTS.
#
# Usage:
#   ./scripts/d1-schema.sh           # against the remote production D1 database
#   ./scripts/d1-schema.sh --local   # against the local wrangler dev database

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKER_DIR="$SCRIPT_DIR/../worker"
SCHEMA="$WORKER_DIR/schema.sql"

LOCAL_FLAG="${1:-}"

echo "Applying D1 schema: $SCHEMA"

cd "$WORKER_DIR"
npx wrangler d1 execute markets-sync --file schema.sql $LOCAL_FLAG

echo "Done."
