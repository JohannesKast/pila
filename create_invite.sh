#!/usr/bin/env bash

echo "Pila - Invite Generator"
echo "------------------------"
read -p "Enter the name of the player: " USER_NAME

TOKEN=$(cat /proc/sys/kernel/random/uuid)
USER_ID=$(cat /proc/sys/kernel/random/uuid)

docker exec -i pila_db psql -U pila -d pila_db -c "INSERT INTO users (id, name, token, is_admin) VALUES ('$USER_ID', '$USER_NAME', '$TOKEN', false);"

echo ""
echo "Player '$USER_NAME' created successfully!"
BASE_URL="${PILA_BASE_URL:-http://localhost:8000}"
echo "Send them this link to play: $BASE_URL/play/me/$TOKEN"
echo "They can simply bookmark this link. No password required."
