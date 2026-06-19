#!/usr/bin/env bash
# Apply (or re-apply) the Supabase schema.
# Safe to re-run: uses IF NOT EXISTS / DROP POLICY IF EXISTS throughout.
#
# Requires a direct Postgres connection — the anon key cannot run DDL.
# Set SUPABASE_DB_URL to the connection string from:
#   Supabase dashboard → Settings → Database → Connection string → URI
# e.g. postgresql://postgres:[password]@db.[ref].supabase.co:5432/postgres
#
# Alternatively, if the Supabase CLI is installed and linked to this project,
# the script will use `supabase db execute` as a fallback.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$SCRIPT_DIR/.."
ENV_FILE="$ROOT_DIR/.env"
SCHEMA="$ROOT_DIR/supabase_schema.sql"

# Load .env if SUPABASE_DB_URL is not already set.
if [[ -z "${SUPABASE_DB_URL:-}" && -f "$ENV_FILE" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
fi

echo "Applying Supabase schema: $SCHEMA"

if [[ -n "${SUPABASE_DB_URL:-}" ]]; then
  psql "$SUPABASE_DB_URL" -f "$SCHEMA"
elif command -v supabase &>/dev/null; then
  supabase db execute --file "$SCHEMA"
else
  echo "Error: set SUPABASE_DB_URL in .env (Settings → Database → Connection string → URI)" >&2
  echo "       or install the Supabase CLI and link this project." >&2
  exit 1
fi

echo "Done."
