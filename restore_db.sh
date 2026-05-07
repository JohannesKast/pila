#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "$SCRIPT_DIR/.env" ]]; then
  while IFS= read -r line; do
    [[ "$line" != *"="* ]] && continue
    key="${line%%=*}"
    value="${line#*=}"
    if [[ "$key" =~ ^[A-Z_][A-Z0-9_]*$ ]]; then
      if [[ "$value" =~ ^\"(.*)\"$ ]] || [[ "$value" =~ ^\'(.*)\'$ ]]; then
        value="${BASH_REMATCH[1]}"
      fi
      export "$key=$value"
    fi
  done < <(grep -v '^\s*#' "$SCRIPT_DIR/.env" | grep -v '^\s*$')
fi

BACKUP_DIR="${BACKUP_DIR:-./backups}"
DB_NAME="${2:-pila_db}"

usage() {
  echo "Usage: $0 <backup_file> [db_name]"
  echo ""
  echo "  backup_file  Path to .sql.gz backup file (required)"
  echo "  db_name      Target database name (default: pila_db)"
  echo ""
  echo "Example: $0 backups/pila_db_20260601_120000.sql.gz staging_db"
  exit 1
}

if [[ $# -lt 1 ]]; then
  echo "Available backups in $BACKUP_DIR:"
  ls -t "$BACKUP_DIR"/pila_db_*.sql.gz 2>/dev/null | nl -w2 -s') ' || echo "  (none found)"
  echo ""
  usage
fi

BACKUP_FILE="$1"

if [[ ! -f "$BACKUP_FILE" ]]; then
  echo "Error: File not found: $BACKUP_FILE"
  exit 1
fi

echo "Restoring $BACKUP_FILE into database '$DB_NAME'..."
read -p "Are you sure? This will overwrite existing data. [y/N] " CONFIRM
[[ "$CONFIRM" =~ ^[Yy]$ ]] || { echo "Aborted."; exit 0; }

docker exec pila_db psql -U pila -d postgres -tc "SELECT 1 FROM pg_database WHERE datname = '$DB_NAME'" \
  | grep -q 1 || docker exec pila_db psql -U pila -d postgres -c "CREATE DATABASE \"$DB_NAME\";"

gunzip -c "$BACKUP_FILE" | docker exec -i pila_db psql -U pila "$DB_NAME"

echo "Done."
