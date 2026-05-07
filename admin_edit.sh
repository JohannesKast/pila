#!/usr/bin/env bash
set -euo pipefail

PSQL=(docker exec -i pila_db psql -U pila -d pila_db -t -A -F $'\t')
PSQL_VERBOSE=(docker exec -i pila_db psql -U pila -d pila_db)

echo "Pila - Admin Edit"
echo "------------------"

# --- Pick user -----------------------------------------------------------
mapfile -t USERS < <("${PSQL[@]}" -c "SELECT id, name FROM users ORDER BY name;")
if [ ${#USERS[@]} -eq 0 ]; then
  echo "No users found." >&2
  exit 1
fi

echo ""
echo "Users:"
for i in "${!USERS[@]}"; do
  IFS=$'\t' read -r _UID UNAME <<< "${USERS[$i]}"
  printf "  [%2d] %s\n" "$((i+1))" "$UNAME"
done

read -p "Pick user number: " USER_IDX
if ! [[ "$USER_IDX" =~ ^[0-9]+$ ]] || [ "$USER_IDX" -lt 1 ] || [ "$USER_IDX" -gt ${#USERS[@]} ]; then
  echo "Invalid selection." >&2
  exit 1
fi
IFS=$'\t' read -r USER_ID USER_NAME <<< "${USERS[$((USER_IDX-1))]}"
echo "Selected: $USER_NAME ($USER_ID)"

# --- Helpers -------------------------------------------------------------
team_name_by_id() {
  "${PSQL[@]}" -c "SELECT name FROM teams WHERE id = $1;"
}

show_teams() {
  echo ""
  echo "Teams:"
  "${PSQL_VERBOSE[@]}" -c "SELECT id, COALESCE(group_letter, '-') AS grp, name FROM teams ORDER BY group_letter NULLS LAST, name;"
}

# --- Edit champion -------------------------------------------------------
edit_champion() {
  show_teams
  echo ""
  "${PSQL_VERBOSE[@]}" -c "
    SELECT t.name AS current_champion
    FROM special_predictions sp
    LEFT JOIN teams t ON t.id = sp.champion_id
    WHERE sp.user_id = '$USER_ID';"

  read -p "Champion (team id, blank to clear): " CHAMP_ID
  if [ -z "$CHAMP_ID" ]; then
    "${PSQL_VERBOSE[@]}" -c "
      INSERT INTO special_predictions (user_id, champion_id)
      VALUES ('$USER_ID', NULL)
      ON CONFLICT (user_id) DO UPDATE
        SET champion_id = NULL, updated_at = NOW();"
    echo "Champion cleared."
    return 0
  fi

  if ! [[ "$CHAMP_ID" =~ ^[0-9]+$ ]]; then
    echo "Team id must be numeric." >&2
    return 1
  fi

  CHAMP_NAME=$(team_name_by_id "$CHAMP_ID")
  if [ -z "$CHAMP_NAME" ]; then
    echo "Unknown team id." >&2
    return 1
  fi

  echo "Confirm for $USER_NAME: Weltmeister = $CHAMP_NAME"
  read -p "Write? [y/N] " YN
  [[ "$YN" == "y" || "$YN" == "Y" ]] || { echo "Cancelled."; return 0; }

  "${PSQL_VERBOSE[@]}" -c "
    INSERT INTO special_predictions (user_id, champion_id)
    VALUES ('$USER_ID', $CHAMP_ID)
    ON CONFLICT (user_id) DO UPDATE
      SET champion_id = EXCLUDED.champion_id,
          updated_at = NOW();"
  echo "Champion saved."
}

# --- Edit match prediction ----------------------------------------------
edit_match() {
  echo ""
  echo "Active matches (TBD excluded):"
  "${PSQL_VERBOSE[@]}" -c "
    SELECT m.id,
           m.stage,
           COALESCE(m.group_letter, '-') AS grp,
           th.name || ' - ' || ta.name AS matchup,
           COALESCE(p.predicted_home::text || ':' || p.predicted_away::text, '-') AS current_tip
    FROM matches m
    JOIN teams th ON th.id = m.team_home_id
    JOIN teams ta ON ta.id = m.team_away_id
    LEFT JOIN predictions p ON p.match_id = m.id AND p.user_id = '$USER_ID'
    WHERE m.team_home_id IS NOT NULL AND m.team_away_id IS NOT NULL
    ORDER BY m.kickoff_time NULLS LAST, m.id;"

  read -p "Match ID: " MID
  if ! [[ "$MID" =~ ^[0-9]+$ ]]; then
    echo "Invalid match ID." >&2
    return 1
  fi

  MATCH_INFO=$("${PSQL[@]}" -c "
    SELECT th.name, ta.name
    FROM matches m
    JOIN teams th ON th.id = m.team_home_id
    JOIN teams ta ON ta.id = m.team_away_id
    WHERE m.id = $MID;")
  if [ -z "$MATCH_INFO" ]; then
    echo "Match not found or TBD." >&2
    return 1
  fi
  IFS=$'\t' read -r HOME_NAME AWAY_NAME <<< "$MATCH_INFO"

  echo "Match: $HOME_NAME vs $AWAY_NAME"
  read -p "Prediction (format 'h a', e.g. '2 1'): " SCORE_H SCORE_A

  if ! [[ "$SCORE_H" =~ ^[0-9]+$ ]] || ! [[ "$SCORE_A" =~ ^[0-9]+$ ]]; then
    echo "Scores must be numbers." >&2
    return 1
  fi
  if [ "$SCORE_H" -gt 20 ] || [ "$SCORE_A" -gt 20 ]; then
    echo "Score out of range (0-20)." >&2
    return 1
  fi

  echo "Confirm for $USER_NAME: $HOME_NAME $SCORE_H : $SCORE_A $AWAY_NAME"
  read -p "Write? [y/N] " YN
  [[ "$YN" == "y" || "$YN" == "Y" ]] || { echo "Cancelled."; return 0; }

  "${PSQL_VERBOSE[@]}" -c "
    INSERT INTO predictions (user_id, match_id, predicted_home, predicted_away)
    VALUES ('$USER_ID', $MID, $SCORE_H, $SCORE_A)
    ON CONFLICT (user_id, match_id) DO UPDATE
      SET predicted_home = EXCLUDED.predicted_home,
          predicted_away = EXCLUDED.predicted_away,
          updated_at = NOW();"
  echo "Prediction saved."
}

# --- Set match result (admin override) ----------------------------------
edit_result() {
  read -p "Match ID: " MID
  if ! [[ "$MID" =~ ^[0-9]+$ ]]; then
    echo "Invalid match ID." >&2
    return 1
  fi
  read -p "Final score (format 'h a', e.g. '3 2'): " SCORE_H SCORE_A
  if ! [[ "$SCORE_H" =~ ^[0-9]+$ ]] || ! [[ "$SCORE_A" =~ ^[0-9]+$ ]]; then
    echo "Scores must be numbers." >&2
    return 1
  fi
  read -p "Mark as finished? [Y/n] " YN
  STATUS="finished"
  [[ "$YN" == "n" || "$YN" == "N" ]] && STATUS="live"

  "${PSQL_VERBOSE[@]}" -c "
    UPDATE matches SET score_home = $SCORE_H, score_away = $SCORE_A, status = '$STATUS'
    WHERE id = $MID;"
  echo "Result saved."
}

# --- Main menu -----------------------------------------------------------
while true; do
  echo ""
  echo "What to edit for $USER_NAME?"
  echo "  [1] Champion (Weltmeister)"
  echo "  [2] Match prediction"
  echo "  [3] Match result (admin override)"
  echo "  [q] Quit"
  read -p "> " CHOICE
  case "$CHOICE" in
    1) edit_champion || true ;;
    2) edit_match || true ;;
    3) edit_result || true ;;
    q|Q) break ;;
    *) echo "Invalid choice." ;;
  esac
done

echo "Done."
