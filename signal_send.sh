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

API_URL="${SIGNAL_API_URL:-}"
FROM="${SIGNAL_FROM_NUMBER:-}"
GROUP="${SIGNAL_GROUP_ID:-}"

if [[ -z "$API_URL" || -z "$FROM" || -z "$GROUP" ]]; then
  echo "Error: SIGNAL_API_URL, SIGNAL_FROM_NUMBER, SIGNAL_GROUP_ID must be set (env or .env)" >&2
  exit 1
fi

API_URL="${API_URL%/}"

case "$API_URL" in
  *signal-cli*) API_URL="http://127.0.0.1:8080" ;;
esac

echo "From:   $FROM"
echo "Group:  $GROUP"
echo "API:    $API_URL"
echo

if [[ $# -gt 0 ]]; then
  MESSAGE="$*"
else
  echo "Message (end with Ctrl-D):"
  MESSAGE="$(cat)"
fi

if [[ -z "${MESSAGE//[[:space:]]/}" ]]; then
  echo "Empty message — aborting." >&2
  exit 1
fi

PAYLOAD=$(jq -n \
  --arg msg "$MESSAGE" \
  --arg num "$FROM" \
  --arg grp "$GROUP" \
  '{message: $msg, number: $num, recipients: [$grp]}')

echo "Sending..."
HTTP_CODE=$(curl -sS -o /tmp/signal_send_resp.$$ -w "%{http_code}" \
  -X POST "$API_URL/v2/send" \
  -H "Content-Type: application/json" \
  -d "$PAYLOAD")

BODY=$(cat /tmp/signal_send_resp.$$)
rm -f /tmp/signal_send_resp.$$

echo "HTTP $HTTP_CODE"
echo "$BODY"

[[ "$HTTP_CODE" =~ ^2 ]] || exit 1
