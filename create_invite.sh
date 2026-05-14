#!/usr/bin/env bash
# Pila — fallback invite generator.
#
# Prefer the in-app admin UI (/admin/users + /admin/leagues) for new invites.
# This script remains as a no-frontend escape hatch and now requires the
# target league: pick from the printed list, or pass it via $PILA_LEAGUE_ID.

set -euo pipefail

echo "Pila - Invite Generator"
echo "------------------------"

# 1) Resolve the league. Use $PILA_LEAGUE_ID if set, otherwise list and prompt.
LEAGUE_ID="${PILA_LEAGUE_ID:-}"
if [[ -z "$LEAGUE_ID" ]]; then
    echo "Available leagues:"
    docker exec -i pila_db psql -U pila -d pila_db -At -c "SELECT id || ' — ' || name FROM leagues ORDER BY name;"
    echo ""
    read -p "Enter the target league UUID: " LEAGUE_ID
fi

if [[ -z "$LEAGUE_ID" ]]; then
    echo "ERROR: league id is required (set PILA_LEAGUE_ID or enter at the prompt)." >&2
    exit 1
fi

# 2) Player name.
read -p "Enter the name of the player: " USER_NAME

TOKEN=$(cat /proc/sys/kernel/random/uuid)
USER_ID=$(cat /proc/sys/kernel/random/uuid)

# 3) Insert with league_id (NOT NULL) — language defaults to 'de' at column level
#    or whatever the migration sets.
docker exec -i pila_db psql -U pila -d pila_db -c \
    "INSERT INTO users (id, name, token, is_admin, league_id, email) \
     VALUES ('$USER_ID', '$USER_NAME', '$TOKEN', false, '$LEAGUE_ID');"

echo ""
echo "Player '$USER_NAME' created successfully in league $LEAGUE_ID!"
BASE_URL="${PILA_BASE_URL:-http://localhost:8000}"
echo "Send them this link to play: $BASE_URL/play/me/$TOKEN"
echo "They can simply bookmark this link. No password required."
