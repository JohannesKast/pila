#!/usr/bin/env bash

echo "Pila - User löschen"
echo "---------------------"
echo ""

echo "Vorhandene User:"
echo ""
docker exec -i pila_db psql -U pila -d pila_db -t -c "SELECT id, name FROM users ORDER BY name;"
echo ""

read -p "Name des Users zum Löschen: " USER_NAME

USER_ID=$(docker exec -i pila_db psql -U pila -d pila_db -t -c "SELECT id FROM users WHERE name = '$USER_NAME';" | tr -d ' \n')

if [ -z "$USER_ID" ]; then
    echo "Kein User mit dem Namen '$USER_NAME' gefunden."
    exit 1
fi

echo ""
echo "User '$USER_NAME' (ID: $USER_ID) wird mit allen Predictions gelöscht!"
read -p "Bist du sicher? (ja/nein): " CONFIRM

if [ "$CONFIRM" != "ja" ]; then
    echo "Abgebrochen."
    exit 0
fi

docker exec -i pila_db psql -U pila -d pila_db -c "
    DELETE FROM predictions WHERE user_id = '$USER_ID';
    DELETE FROM special_predictions WHERE user_id = '$USER_ID';
    DELETE FROM users WHERE id = '$USER_ID';
"

echo ""
echo "User '$USER_NAME' und alle zugehörigen Daten gelöscht."
