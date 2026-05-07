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
TIMESTAMP=$(date +"%Y%m%d_%H%M%S")
BACKUP_FILE="$BACKUP_DIR/pila_db_$TIMESTAMP.sql.gz"

mkdir -p "$BACKUP_DIR"

echo "Creating backup: $BACKUP_FILE"
docker exec pila_db pg_dump -U pila pila_db | gzip > "$BACKUP_FILE"

echo "Done: $BACKUP_FILE ($(du -h "$BACKUP_FILE" | cut -f1))"

ls -t "$BACKUP_DIR"/pila_db_*.sql.gz 2>/dev/null | tail -n +11 | xargs -r rm --
