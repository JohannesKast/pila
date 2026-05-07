use askama::Template;
use axum::{extract::State, routing::get, Router};
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use pila::{auth, notifier, scoring, stage::Stage, worker, AppState};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting Pila Application...");

    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    tracing::info!("Running database migrations...");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run DB migrations");

    let state = AppState { db: pool.clone() };

    if let Err(e) = worker::bootstrap_notifications(&pool).await {
        tracing::warn!("Notification bootstrap failed: {:?}", e);
    }

    let notifier = notifier::from_env();
    worker::start_background_worker(pool, notifier).await;

    let app = Router::new()
        .route("/", get(handlers::index))
        .route("/play/me/:token", get(handlers::login_magic_link))
        .route(
            "/predict/:match_id",
            axum::routing::post(handlers::predict_match),
        )
        .route(
            "/predict_special",
            axum::routing::post(handlers::predict_special),
        )
        .route("/leaderboard", get(handlers::leaderboard))
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8000".into());
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse().expect("Invalid PORT");
    tracing::info!("Listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

mod handlers {
    use super::*;
    use crate::auth::AuthenticatedUser;
    use axum::{
        extract::{Form, Path},
        http::StatusCode,
        response::{Html, Redirect},
    };
    use axum_extra::extract::CookieJar;
    use serde::Deserialize;

    // ─── View types ───────────────────────────────────────────────────────────

    pub struct MatchView {
        pub id: i32,
        pub stage: Stage,
        pub stage_label: String,
        pub group_letter: Option<String>,
        pub home_name: String,
        pub away_name: String,
        pub home_flag: String,
        pub away_flag: String,
        pub score_home: Option<i32>,
        pub score_away: Option<i32>,
        pub predicted_home: Option<i32>,
        pub predicted_away: Option<i32>,
        pub kickoff_display: String,
        pub locked: bool,
        pub own_points: Option<i32>,
        pub multiplier: i32,
        pub other_preds: Vec<UserPrediction>,
    }

    impl MatchView {
        pub fn is_final(&self) -> bool {
            matches!(self.stage, Stage::Final)
        }
        pub fn is_third_place(&self) -> bool {
            matches!(self.stage, Stage::ThirdPlace)
        }
        pub fn is_knockout(&self) -> bool {
            self.stage.is_knockout()
        }
    }

    pub struct UserPrediction {
        pub name: String,
        pub home: i32,
        pub away: i32,
        pub points: Option<i32>,
    }

    #[derive(Default)]
    pub struct StageGroups {
        pub groups: Vec<(String, Vec<MatchView>)>,
        pub round_of_32: Vec<MatchView>,
        pub round_of_16: Vec<MatchView>,
        pub quarter_final: Vec<MatchView>,
        pub semi_final: Vec<MatchView>,
        pub third_place: Vec<MatchView>,
        pub final_: Vec<MatchView>,
    }

    impl StageGroups {
        pub fn push(&mut self, m: MatchView) {
            match m.stage {
                Stage::Group => {
                    let letter = m.group_letter.clone().unwrap_or_default();
                    if let Some(slot) = self.groups.iter_mut().find(|(k, _)| *k == letter) {
                        slot.1.push(m);
                    } else {
                        self.groups.push((letter, vec![m]));
                    }
                }
                Stage::RoundOf32 => self.round_of_32.push(m),
                Stage::RoundOf16 => self.round_of_16.push(m),
                Stage::QuarterFinal => self.quarter_final.push(m),
                Stage::SemiFinal => self.semi_final.push(m),
                Stage::ThirdPlace => self.third_place.push(m),
                Stage::Final => self.final_.push(m),
            }
        }
        pub fn sort_groups(&mut self) {
            self.groups.sort_by(|a, b| a.0.cmp(&b.0));
        }
        pub fn is_empty(&self) -> bool {
            self.groups.is_empty()
                && self.round_of_32.is_empty()
                && self.round_of_16.is_empty()
                && self.quarter_final.is_empty()
                && self.semi_final.is_empty()
                && self.third_place.is_empty()
                && self.final_.is_empty()
        }
        pub fn len(&self) -> usize {
            self.groups.iter().map(|(_, v)| v.len()).sum::<usize>()
                + self.round_of_32.len()
                + self.round_of_16.len()
                + self.quarter_final.len()
                + self.semi_final.len()
                + self.third_place.len()
                + self.final_.len()
        }
    }

    #[derive(Clone)]
    pub struct TeamView {
        pub id: i32,
        pub name: String,
    }

    pub struct SpecialPredictionsView {
        pub champion_id: Option<i32>,
    }

    impl SpecialPredictionsView {
        pub fn is_champion(&self, team_id: &i32) -> bool {
            self.champion_id == Some(*team_id)
        }
    }

    pub struct ChampPrediction {
        pub name: String,
        pub team_name: String,
        pub team_flag: String,
        pub points: Option<i32>,
    }

    pub struct GroupRow {
        pub team_name: String,
        pub flag: String,
        pub played: i32,
        pub wins: i32,
        pub draws: i32,
        pub losses: i32,
        pub goals_for: i32,
        pub goals_against: i32,
        pub goal_diff: i32,
        pub points: i32,
    }

    pub struct GroupStandingsTable {
        pub letter: String,
        pub rows: Vec<GroupRow>,
    }

    pub struct LeaderboardEntry {
        pub name: String,
        pub total_points: i32,
        pub max_potential_points: i32,
    }

    // ─── Helpers ──────────────────────────────────────────────────────────────

    fn flag_url(code: &Option<String>) -> String {
        match code {
            Some(c) if !c.is_empty() => format!("https://flagcdn.com/w40/{c}.png"),
            _ => String::new(),
        }
    }

    fn format_kickoff(dt: Option<chrono::DateTime<chrono::Utc>>) -> String {
        match dt {
            Some(d) => d
                .with_timezone(&chrono_tz::Europe::Berlin)
                .format("%d.%m.%Y %H:%M")
                .to_string(),
            None => "TBD".to_string(),
        }
    }

    async fn fetch_actual_champion(db: &sqlx::PgPool) -> Option<i32> {
        let row = sqlx::query!(
            r#"
            SELECT team_home_id as "team_home_id?",
                   team_away_id as "team_away_id?",
                   score_home   as "score_home?",
                   score_away   as "score_away?",
                   status
            FROM matches
            WHERE stage = 'final'::match_stage AND status = 'finished'
            ORDER BY kickoff_time DESC
            LIMIT 1
            "#
        )
        .fetch_optional(db)
        .await
        .unwrap_or_default()?;

        match (row.score_home, row.score_away, row.team_home_id, row.team_away_id) {
            (Some(sh), Some(sa), Some(hid), _) if sh > sa => Some(hid),
            (Some(sh), Some(sa), _, Some(aid)) if sa > sh => Some(aid),
            _ => None,
        }
    }

    async fn fetch_leaderboard(
        db: &sqlx::PgPool,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Vec<LeaderboardEntry> {
        let users = sqlx::query!("SELECT id, name FROM users")
            .fetch_all(db)
            .await
            .unwrap_or_default();

        let mut user_scores: std::collections::BTreeMap<String, (i32, i32)> =
            std::collections::BTreeMap::new();
        for u in &users {
            user_scores.insert(u.name.clone(), (0, 0));
        }

        let pred_rows = sqlx::query!(
            r#"
            SELECT u.name,
                   m.stage as "stage: Stage",
                   m.kickoff_time,
                   m.status,
                   m.score_home as "score_home?",
                   m.score_away as "score_away?",
                   p.predicted_home,
                   p.predicted_away
            FROM predictions p
            JOIN users u ON u.id = p.user_id
            JOIN matches m ON m.id = p.match_id
            "#
        )
        .fetch_all(db)
        .await
        .unwrap_or_default();

        for r in pred_rows {
            let entry = user_scores.entry(r.name.clone()).or_insert((0, 0));
            let started = r.kickoff_time.is_some_and(|dt| dt < now);
            let finished = r.status == "finished";

            if finished {
                if let (Some(sh), Some(sa)) = (r.score_home, r.score_away) {
                    entry.0 += scoring::calculate_match_points(
                        r.stage,
                        sh,
                        sa,
                        r.predicted_home,
                        r.predicted_away,
                    );
                }
            } else if started {
                // locked but not finished — full max remains achievable
                entry.1 += scoring::max_potential_points(r.stage);
            } else {
                // open — also count max as potential (user already tipped)
                entry.1 += scoring::max_potential_points(r.stage);
            }
        }

        let actual_champion = fetch_actual_champion(db).await;
        let sp_rows = sqlx::query!(
            "SELECT u.name, sp.champion_id FROM special_predictions sp
             JOIN users u ON u.id = sp.user_id"
        )
        .fetch_all(db)
        .await
        .unwrap_or_default();

        for sp in sp_rows {
            let entry = user_scores.entry(sp.name).or_insert((0, 0));
            if let Some(cid) = sp.champion_id {
                if actual_champion.is_some() {
                    entry.0 += scoring::champion_points(Some(cid), actual_champion);
                } else {
                    entry.1 += 10;
                }
            }
        }

        let mut leaderboard: Vec<LeaderboardEntry> = user_scores
            .into_iter()
            .map(|(name, (total, potential))| LeaderboardEntry {
                name,
                total_points: total,
                max_potential_points: total + potential,
            })
            .collect();
        leaderboard.sort_by(|a, b| b.total_points.cmp(&a.total_points));
        leaderboard
    }

    async fn fetch_group_standings(db: &sqlx::PgPool) -> Vec<GroupStandingsTable> {
        let rows = sqlx::query!(
            r#"
            SELECT m.group_letter as "letter!",
                   m.team_home_id as "home_id!",
                   m.team_away_id as "away_id!",
                   m.score_home   as "score_home!",
                   m.score_away   as "score_away!",
                   th.name        as "home_name!",
                   th.flag_code   as "home_flag?",
                   ta.name        as "away_name!",
                   ta.flag_code   as "away_flag?"
            FROM matches m
            JOIN teams th ON th.id = m.team_home_id
            JOIN teams ta ON ta.id = m.team_away_id
            WHERE m.stage = 'group'::match_stage
              AND m.status = 'finished'
              AND m.group_letter IS NOT NULL
              AND m.score_home IS NOT NULL
              AND m.score_away IS NOT NULL
            "#
        )
        .fetch_all(db)
        .await
        .unwrap_or_default();

        // letter → team_id → row
        let mut groups: std::collections::BTreeMap<String, std::collections::HashMap<i32, GroupRow>> =
            std::collections::BTreeMap::new();

        for r in rows {
            let letter = r.letter.to_string();
            let group = groups.entry(letter).or_default();

            let home = group.entry(r.home_id).or_insert_with(|| GroupRow {
                team_name: r.home_name.clone(),
                flag: flag_url(&r.home_flag),
                played: 0,
                wins: 0,
                draws: 0,
                losses: 0,
                goals_for: 0,
                goals_against: 0,
                goal_diff: 0,
                points: 0,
            });
            home.played += 1;
            home.goals_for += r.score_home;
            home.goals_against += r.score_away;
            if r.score_home > r.score_away {
                home.wins += 1;
                home.points += 3;
            } else if r.score_home == r.score_away {
                home.draws += 1;
                home.points += 1;
            } else {
                home.losses += 1;
            }
            home.goal_diff = home.goals_for - home.goals_against;

            let away = group.entry(r.away_id).or_insert_with(|| GroupRow {
                team_name: r.away_name.clone(),
                flag: flag_url(&r.away_flag),
                played: 0,
                wins: 0,
                draws: 0,
                losses: 0,
                goals_for: 0,
                goals_against: 0,
                goal_diff: 0,
                points: 0,
            });
            away.played += 1;
            away.goals_for += r.score_away;
            away.goals_against += r.score_home;
            if r.score_away > r.score_home {
                away.wins += 1;
                away.points += 3;
            } else if r.score_away == r.score_home {
                away.draws += 1;
                away.points += 1;
            } else {
                away.losses += 1;
            }
            away.goal_diff = away.goals_for - away.goals_against;
        }

        groups
            .into_iter()
            .map(|(letter, teams)| {
                let mut rows: Vec<GroupRow> = teams.into_values().collect();
                rows.sort_by(|a, b| {
                    b.points
                        .cmp(&a.points)
                        .then(b.goal_diff.cmp(&a.goal_diff))
                        .then(b.goals_for.cmp(&a.goals_for))
                        .then(a.team_name.cmp(&b.team_name))
                });
                GroupStandingsTable { letter, rows }
            })
            .collect()
    }

    // ─── Templates ────────────────────────────────────────────────────────────

    #[derive(Template)]
    #[template(path = "index.html")]
    struct IndexTemplate {
        user_name: String,
        default_tab: String,
        started_in_progress: StageGroups,
        started_finished: StageGroups,
        open_matches: StageGroups,
        open_count: usize,
        started_count: usize,
        leaderboard: Vec<LeaderboardEntry>,
        group_standings: Vec<GroupStandingsTable>,
        team_options: Vec<TeamView>,
        special_preds: SpecialPredictionsView,
        tournament_locked: bool,
        champ_preds: Vec<ChampPrediction>,
    }

    #[derive(Template)]
    #[template(path = "leaderboard.html")]
    struct LeaderboardTemplate {
        entries: Vec<LeaderboardEntry>,
    }

    // ─── Handlers ─────────────────────────────────────────────────────────────

    pub async fn index(State(state): State<AppState>, user: AuthenticatedUser) -> Html<String> {
        let now = chrono::Utc::now();

        let rows = sqlx::query!(
            r#"
            SELECT
                m.id,
                m.stage as "stage: Stage",
                m.group_letter,
                m.kickoff_time,
                m.status,
                m.score_home as "score_home?",
                m.score_away as "score_away?",
                m.team_home_id as "team_home_id?",
                m.team_away_id as "team_away_id?",
                COALESCE(th.name, 'TBD') as "home_name!",
                COALESCE(ta.name, 'TBD') as "away_name!",
                th.flag_code as "home_flag?",
                ta.flag_code as "away_flag?",
                p.predicted_home as "predicted_home?",
                p.predicted_away as "predicted_away?"
            FROM matches m
            LEFT JOIN teams th ON th.id = m.team_home_id
            LEFT JOIN teams ta ON ta.id = m.team_away_id
            LEFT JOIN predictions p ON p.match_id = m.id AND p.user_id = $1
            ORDER BY
                CASE m.stage
                    WHEN 'group' THEN 0
                    WHEN 'round_of_32' THEN 1
                    WHEN 'round_of_16' THEN 2
                    WHEN 'quarter_final' THEN 3
                    WHEN 'semi_final' THEN 4
                    WHEN 'third_place' THEN 5
                    WHEN 'final' THEN 6
                END,
                m.group_letter NULLS LAST,
                m.kickoff_time NULLS LAST,
                m.id
            "#,
            user.id
        )
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

        // Other users' preds for locked matches
        let other_preds_rows = sqlx::query!(
            r#"
            SELECT p.match_id, u.name as user_name,
                   p.predicted_home, p.predicted_away
            FROM predictions p
            JOIN users u ON u.id = p.user_id
            JOIN matches m ON m.id = p.match_id
            WHERE m.kickoff_time IS NOT NULL
              AND m.kickoff_time < $1
              AND u.id != $2
            ORDER BY u.name
            "#,
            now,
            user.id
        )
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

        let mut preds_by_match: std::collections::HashMap<i32, Vec<(String, i32, i32)>> =
            std::collections::HashMap::new();
        for p in other_preds_rows {
            preds_by_match
                .entry(p.match_id)
                .or_default()
                .push((p.user_name, p.predicted_home, p.predicted_away));
        }

        // Tournament lock = first kickoff has passed
        let first_kickoff: Option<chrono::DateTime<chrono::Utc>> =
            sqlx::query_scalar!("SELECT MIN(kickoff_time) FROM matches")
                .fetch_one(&state.db)
                .await
                .unwrap_or_default();
        let tournament_locked = first_kickoff.is_some_and(|dt| dt < now);

        // Special prediction (current user)
        let special_pred_row = sqlx::query!(
            "SELECT champion_id FROM special_predictions WHERE user_id = $1",
            user.id
        )
        .fetch_optional(&state.db)
        .await
        .unwrap_or_default();

        let special_preds = SpecialPredictionsView {
            champion_id: special_pred_row.as_ref().and_then(|r| r.champion_id),
        };

        // Team options (alphabetical) for champion dropdown — exclude ESPN bracket placeholders
        let team_rows = sqlx::query!(
            "SELECT id, name, flag_code FROM teams \
             WHERE name NOT LIKE 'Group %' \
               AND name NOT LIKE 'Quarterfinal %' \
               AND name NOT LIKE 'Semifinal %' \
               AND name NOT LIKE 'Round of %' \
               AND name NOT LIKE 'Third Place %' \
             ORDER BY name"
        )
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

        let team_options: Vec<TeamView> = team_rows
            .iter()
            .map(|t| TeamView {
                id: t.id,
                name: t.name.clone(),
            })
            .collect();

        // All users' champion preds when locked
        let champ_preds: Vec<ChampPrediction> = if tournament_locked {
            let actual = fetch_actual_champion(&state.db).await;
            let rows = sqlx::query!(
                r#"
                SELECT u.name as user_name, sp.champion_id, t.name as "team_name?", t.flag_code
                FROM special_predictions sp
                JOIN users u ON u.id = sp.user_id
                LEFT JOIN teams t ON t.id = sp.champion_id
                ORDER BY u.name
                "#
            )
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();

            rows.into_iter()
                .filter(|r| r.champion_id.is_some())
                .map(|r| {
                    let pts = match (r.champion_id, actual) {
                        (Some(p), Some(a)) => Some(if p == a { 10 } else { 0 }),
                        _ => None,
                    };
                    ChampPrediction {
                        name: r.user_name,
                        team_name: r.team_name.unwrap_or_default(),
                        team_flag: flag_url(&r.flag_code),
                        points: pts,
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        let mut started_in_progress = StageGroups::default();
        let mut started_finished = StageGroups::default();
        let mut open_matches = StageGroups::default();

        for r in rows {
            if r.team_home_id.is_none() || r.team_away_id.is_none() {
                continue; // skip TBD knockout slots
            }

            let locked = r.kickoff_time.is_some_and(|dt| dt < now);
            let finished = r.status == "finished";

            let own_points = if finished {
                match (
                    r.score_home,
                    r.score_away,
                    r.predicted_home,
                    r.predicted_away,
                ) {
                    (Some(sh), Some(sa), Some(ph), Some(pa)) => {
                        Some(scoring::calculate_match_points(r.stage, sh, sa, ph, pa))
                    }
                    _ => None,
                }
            } else {
                None
            };

            let mut other_preds: Vec<UserPrediction> = if locked {
                preds_by_match
                    .get(&r.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(name, home, away)| {
                        let points = if finished {
                            match (r.score_home, r.score_away) {
                                (Some(sh), Some(sa)) => Some(scoring::calculate_match_points(
                                    r.stage, sh, sa, home, away,
                                )),
                                _ => None,
                            }
                        } else {
                            None
                        };
                        UserPrediction {
                            name,
                            home,
                            away,
                            points,
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };
            if finished {
                other_preds.sort_by(|a, b| b.points.cmp(&a.points));
            }

            let mv = MatchView {
                id: r.id,
                stage: r.stage,
                stage_label: r.stage.label_de().to_string(),
                group_letter: r.group_letter.map(|s| s.trim().to_string()),
                home_name: r.home_name,
                away_name: r.away_name,
                home_flag: flag_url(&r.home_flag),
                away_flag: flag_url(&r.away_flag),
                score_home: r.score_home,
                score_away: r.score_away,
                predicted_home: r.predicted_home,
                predicted_away: r.predicted_away,
                kickoff_display: format_kickoff(r.kickoff_time),
                locked,
                own_points,
                multiplier: r.stage.multiplier(),
                other_preds,
            };

            let target = if locked {
                if finished {
                    &mut started_finished
                } else {
                    &mut started_in_progress
                }
            } else {
                &mut open_matches
            };
            target.push(mv);
        }
        started_in_progress.sort_groups();
        started_finished.sort_groups();
        open_matches.sort_groups();

        let group_standings = fetch_group_standings(&state.db).await;
        let leaderboard = fetch_leaderboard(&state.db, now).await;

        let open_count = open_matches.len();
        let started_count = started_in_progress.len() + started_finished.len();

        let special_open = !tournament_locked && special_preds.champion_id.is_none();
        let default_tab = if open_count > 0 {
            "open"
        } else if special_open {
            "special"
        } else {
            "table"
        }
        .to_string();

        let template = IndexTemplate {
            user_name: user.name,
            default_tab,
            started_in_progress,
            started_finished,
            open_matches,
            open_count,
            started_count,
            leaderboard,
            group_standings,
            team_options,
            special_preds,
            tournament_locked,
            champ_preds,
        };
        Html(template.render().unwrap())
    }

    pub async fn login_magic_link(
        State(state): State<AppState>,
        Path(token): Path<String>,
        jar: CookieJar,
    ) -> Result<(CookieJar, Redirect), (StatusCode, &'static str)> {
        let exists = sqlx::query!("SELECT id FROM users WHERE token = $1", token)
            .fetch_optional(&state.db)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        if exists.is_some() {
            let cookie = axum_extra::extract::cookie::Cookie::build(("pila_token", token))
                .path("/")
                .http_only(true)
                .secure(true)
                .same_site(axum_extra::extract::cookie::SameSite::Lax)
                .build();
            let updated_jar = jar.add(cookie);
            Ok((updated_jar, Redirect::to("/")))
        } else {
            Err((StatusCode::UNAUTHORIZED, "Ungültiger oder abgelaufener Link."))
        }
    }

    #[derive(Deserialize)]
    pub struct PredictionForm {
        score_home: i32,
        score_away: i32,
    }

    pub async fn predict_match(
        State(state): State<AppState>,
        user: AuthenticatedUser,
        Path(match_id): Path<i32>,
        Form(form): Form<PredictionForm>,
    ) -> Result<Html<String>, (StatusCode, &'static str)> {
        if !(0..=20).contains(&form.score_home) || !(0..=20).contains(&form.score_away) {
            return Err((
                StatusCode::BAD_REQUEST,
                "Ungültiger Tipp: Werte müssen zwischen 0 und 20 liegen.",
            ));
        }

        let m = sqlx::query!(
            "SELECT kickoff_time, team_home_id, team_away_id FROM matches WHERE id = $1",
            match_id
        )
        .fetch_one(&state.db)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Match not found"))?;

        let now = chrono::Utc::now();

        if m.team_home_id.is_none() || m.team_away_id.is_none() {
            return Err((
                StatusCode::BAD_REQUEST,
                "Begegnung steht noch nicht fest. Tipp nicht möglich.",
            ));
        }

        if let Some(start_time) = m.kickoff_time {
            if start_time < now {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Das Spiel hat bereits begonnen. Tipps sind gesperrt.",
                ));
            }
        }

        sqlx::query!(
            r#"
            INSERT INTO predictions (user_id, match_id, predicted_home, predicted_away, updated_at)
            VALUES ($1, $2, $3, $4, NOW())
            ON CONFLICT (user_id, match_id) DO UPDATE SET
                predicted_home = EXCLUDED.predicted_home,
                predicted_away = EXCLUDED.predicted_away,
                updated_at = NOW()
            "#,
            user.id,
            match_id,
            form.score_home,
            form.score_away
        )
        .execute(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        let html = format!(
            r##"<div class="tip-area" id="tip-form-{id}">
            <form hx-post="/predict/{id}" hx-swap="outerHTML" hx-target="#tip-form-{id}" class="flex items-center gap-1.5">
                <input type="number" name="score_home" value="{h}" min="0" max="20" class="w-12 bg-slate-900/80 border border-emerald-600/50 rounded text-center py-1 text-xs font-bold outline-none focus:border-emerald-500 text-white" required>
                <span class="text-slate-600 text-xs">:</span>
                <input type="number" name="score_away" value="{a}" min="0" max="20" class="w-12 bg-slate-900/80 border border-emerald-600/50 rounded text-center py-1 text-xs font-bold outline-none focus:border-emerald-500 text-white" required>
                <button type="submit" class="ml-auto bg-emerald-600 hover:bg-emerald-500 text-white text-[10px] font-bold px-2.5 py-1 rounded transition-colors">✅</button>
            </form>
            </div>"##,
            id = match_id, h = form.score_home, a = form.score_away
        );
        Ok(Html(html))
    }

    fn deserialize_optional_int<'de, D>(de: D) -> Result<Option<i32>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = Option::<String>::deserialize(de)?;
        match s.as_deref() {
            None | Some("") => Ok(None),
            Some(s) => s.parse().map(Some).map_err(serde::de::Error::custom),
        }
    }

    #[derive(Deserialize)]
    pub struct SpecialPredictionForm {
        #[serde(default, deserialize_with = "deserialize_optional_int")]
        champion_id: Option<i32>,
    }

    pub async fn predict_special(
        State(state): State<AppState>,
        user: AuthenticatedUser,
        Form(form): Form<SpecialPredictionForm>,
    ) -> Result<Redirect, (StatusCode, &'static str)> {
        let now = chrono::Utc::now();

        let first_kickoff: Option<chrono::DateTime<chrono::Utc>> =
            sqlx::query_scalar!("SELECT MIN(kickoff_time) FROM matches")
                .fetch_one(&state.db)
                .await
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

        if first_kickoff.is_some_and(|dt| dt < now) {
            return Err((
                StatusCode::BAD_REQUEST,
                "Turnier hat begonnen. Weltmeister-Tipp ist gesperrt.",
            ));
        }

        if let Some(cid) = form.champion_id {
            let exists = sqlx::query_scalar!(
                "SELECT 1 AS dummy FROM teams \
                 WHERE id = $1 \
                   AND name NOT LIKE 'Group %' \
                   AND name NOT LIKE 'Quarterfinal %' \
                   AND name NOT LIKE 'Semifinal %' \
                   AND name NOT LIKE 'Round of %' \
                   AND name NOT LIKE 'Third Place %'",
                cid
            )
            .fetch_optional(&state.db)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?
            .is_some();
            if !exists {
                return Err((StatusCode::BAD_REQUEST, "Unbekanntes Team."));
            }
        }

        sqlx::query!(
            r#"
            INSERT INTO special_predictions (user_id, champion_id, updated_at)
            VALUES ($1, $2, NOW())
            ON CONFLICT (user_id) DO UPDATE SET
                champion_id = EXCLUDED.champion_id,
                updated_at = NOW()
            "#,
            user.id,
            form.champion_id
        )
        .execute(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

        Ok(Redirect::to("/"))
    }

    pub async fn leaderboard(
        State(state): State<AppState>,
        _user: AuthenticatedUser,
    ) -> Html<String> {
        let now = chrono::Utc::now();
        let entries = fetch_leaderboard(&state.db, now).await;
        let template = LeaderboardTemplate { entries };
        Html(template.render().unwrap())
    }
}
