use chrono::{DateTime, NaiveDate, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

use crate::notifier::{in_quiet_hours_now, NotificationEvent, Notifier};
use crate::repo::match_::EspnMatchUpsert;
use crate::repo::team::EspnTeamUpsert;
use crate::repo::{NotificationRepo, Repos};
use crate::stage::Stage;

// ─── ESPN Scoreboard structs (soccer/fifa.world) ──────────────────────────────

#[derive(Deserialize, Default)]
struct ScoreboardResponse {
    #[serde(default)]
    events: Vec<Event>,
}

#[derive(Deserialize)]
struct Event {
    id: String,
    #[serde(default)]
    season: Option<EventSeason>,
    #[serde(default)]
    competitions: Vec<Competition>,
}

#[derive(Deserialize, Default)]
struct EventSeason {
    #[serde(default)]
    slug: String,
}

#[derive(Deserialize)]
struct Competition {
    #[serde(rename = "startDate")]
    start_date: String,
    #[serde(default)]
    competitors: Vec<Competitor>,
    #[serde(default)]
    notes: Vec<Note>,
    status: Option<CompStatus>,
}

#[derive(Deserialize)]
struct Note {
    #[serde(default)]
    headline: String,
}

#[derive(Deserialize)]
struct CompStatus {
    #[serde(rename = "type")]
    type_: StatusType,
}

#[derive(Deserialize)]
struct StatusType {
    #[serde(default)]
    state: String, // "pre" | "in" | "post"
}

#[derive(Deserialize)]
struct Competitor {
    #[serde(rename = "homeAway")]
    home_away: String,
    team: EspnTeam,
    score: Option<String>,
}

#[derive(Deserialize)]
struct EspnTeam {
    id: String,
    #[serde(rename = "displayName", default)]
    display_name: String,
    #[serde(default)]
    abbreviation: String,
}

#[derive(Deserialize, Default)]
struct StandingsResponse {
    #[serde(default)]
    children: Vec<StandingsGroup>,
}

#[derive(Deserialize)]
struct StandingsGroup {
    #[serde(default)]
    name: String,
    #[serde(default)]
    standings: Option<StandingsBody>,
}

#[derive(Deserialize)]
struct StandingsBody {
    #[serde(default)]
    entries: Vec<StandingsEntry>,
}

#[derive(Deserialize)]
struct StandingsEntry {
    team: EspnTeam,
}

// ─── Public entry points ─────────────────────────────────────────────────────

/// Spawn the 30-minute background loop. Pulls fixtures from ESPN, upserts
/// them via the repo layer, and dispatches due notifications.
pub async fn start_background_worker(repos: Repos, notifier: Arc<dyn Notifier>) {
    let client = Client::new();

    tokio::spawn(async move {
        loop {
            tracing::info!("Running ESPN soccer (fifa.world) update...");
            if let Err(e) = update_data(&client, &repos).await {
                tracing::error!("ESPN soccer worker error: {:?}", e);
            }
            if let Err(e) = process_notifications(&repos, &*notifier).await {
                tracing::error!("Notification processing error: {:?}", e);
            }
            tokio::time::sleep(Duration::from_secs(1800)).await;
        }
    });
}

/// Mark all currently-known matches as already notified on first deploy of
/// the notifier so an existing fixture list does not cause a flood of
/// MatchClosingSoon notifications. Idempotent via a settings flag.
pub async fn bootstrap_notifications(
    notifications: &dyn NotificationRepo,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if notifications.was_bootstrapped().await? {
        return Ok(());
    }

    notifications.silence_existing_matches().await?;
    notifications.mark_bootstrapped().await?;

    tracing::info!("Notification bootstrap complete — current open matches silenced");
    Ok(())
}

// ─── Main update logic ────────────────────────────────────────────────────────

async fn update_data(
    client: &Client,
    repos: &Repos,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let groups_map = fetch_groups_map(client).await;
    if !groups_map.is_empty() {
        tracing::info!("Loaded group letters for {} teams from standings", groups_map.len());
    }

    let (window_start, window_end) = tournament_window();
    let mut current = window_start;
    let mut total_events = 0usize;

    while current <= window_end {
        let date = current.format("%Y%m%d").to_string();
        let url = format!(
            "https://site.api.espn.com/apis/site/v2/sports/soccer/fifa.world/scoreboard?dates={date}"
        );

        let resp: ScoreboardResponse = match client.get(&url).send().await {
            Ok(r) => r.json().await.unwrap_or_default(),
            Err(e) => {
                tracing::warn!("ESPN fetch failed for {}: {:?}", date, e);
                if current == window_end {
                    break;
                }
                current = current.succ_opt().unwrap_or(window_end);
                continue;
            }
        };

        total_events += resp.events.len();

        for event in resp.events {
            if let Err(e) = process_event(repos, &event, &groups_map).await {
                tracing::warn!("event {} processing failed: {:?}", event.id, e);
            }
        }

        if current == window_end {
            break;
        }
        current = current.succ_opt().unwrap_or(window_end);
    }

    tracing::info!("Scoreboard sync complete: {} events", total_events);
    Ok(())
}

async fn process_event(
    repos: &Repos,
    event: &Event,
    groups_map: &std::collections::HashMap<i32, String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let comp = match event.competitions.first() {
        Some(c) => c,
        None => return Ok(()),
    };

    let espn_event_id: i64 = match event.id.parse() {
        Ok(v) => v,
        Err(_) => {
            tracing::debug!("skip: unparseable event id {}", event.id);
            return Ok(());
        }
    };

    let headline = comp
        .notes
        .iter()
        .map(|n| n.headline.as_str())
        .find(|s| !s.is_empty())
        .unwrap_or("");
    let slug = event.season.as_ref().map(|s| s.slug.as_str()).unwrap_or("");

    // Slug is authoritative for stage; headline only as fallback. Headline still
    // useful for group-letter (when present), but ESPN frequently leaves notes empty.
    let (stage, mut group_letter) = match classify_slug(slug) {
        Some(s) => (s, classify_stage(headline).1),
        None => classify_stage(headline),
    };

    let home = comp.competitors.iter().find(|c| c.home_away == "home");
    let away = comp.competitors.iter().find(|c| c.home_away == "away");
    let (home_id, away_id) = match (home, away) {
        (Some(h), Some(a)) => {
            let h_id: Option<i32> = h.team.id.parse().ok();
            let a_id: Option<i32> = a.team.id.parse().ok();
            (h_id, a_id)
        }
        _ => (None, None),
    };

    // For group matches with no headline letter, derive from standings map.
    if stage == Stage::Group && group_letter.is_none() {
        group_letter = home_id
            .and_then(|id| groups_map.get(&id).cloned())
            .or_else(|| away_id.and_then(|id| groups_map.get(&id).cloned()));
    }

    if let (Some(h), Some(a), Some(hid), Some(aid)) = (home, away, home_id, away_id) {
        let hflag = flag_code_for_abbr(&h.team.abbreviation);
        let aflag = flag_code_for_abbr(&a.team.abbreviation);
        // Only attach group_letter on group-stage events (knockout teams may
        // already carry their group letter from earlier syncs).
        let group_for_team: Option<&str> = match stage {
            Stage::Group => group_letter.as_deref(),
            _ => None,
        };

        repos
            .teams
            .upsert_from_espn(EspnTeamUpsert {
                espn_id: hid,
                name: &h.team.display_name,
                short_name: if h.team.abbreviation.is_empty() {
                    None
                } else {
                    Some(h.team.abbreviation.as_str())
                },
                flag_code: hflag.as_deref(),
                group_letter: group_for_team,
            })
            .await?;
        repos
            .teams
            .upsert_from_espn(EspnTeamUpsert {
                espn_id: aid,
                name: &a.team.display_name,
                short_name: if a.team.abbreviation.is_empty() {
                    None
                } else {
                    Some(a.team.abbreviation.as_str())
                },
                flag_code: aflag.as_deref(),
                group_letter: group_for_team,
            })
            .await?;
    }

    let score_h = home.and_then(|c| c.score.as_deref()).and_then(|s| s.parse::<i32>().ok());
    let score_a = away.and_then(|c| c.score.as_deref()).and_then(|s| s.parse::<i32>().ok());

    let kickoff = parse_espn_datetime(&comp.start_date);
    let status = comp
        .status
        .as_ref()
        .map(|s| match s.type_.state.as_str() {
            "in" => "live",
            "post" => "finished",
            _ => "scheduled",
        })
        .unwrap_or("scheduled");

    repos
        .matches
        .upsert_from_espn(EspnMatchUpsert {
            espn_event_id,
            stage,
            group_letter: group_letter.as_deref(),
            team_home_id: home_id,
            team_away_id: away_id,
            score_home: score_h,
            score_away: score_a,
            kickoff_time: kickoff,
            status,
        })
        .await?;

    Ok(())
}

/// Heuristic stage classification from the ESPN competition headline.
/// Examples observed in practice:
///   "Group A - Matchday 1"
///   "FIFA World Cup - Group F"
///   "Round of 32"
///   "Round of 16"
///   "Quarterfinals"
///   "Semifinals"
///   "Third-Place Match" / "3rd Place"
///   "Final"
fn classify_stage(headline: &str) -> (Stage, Option<String>) {
    let lc = headline.to_lowercase();

    if lc.contains("third") || lc.contains("3rd place") {
        return (Stage::ThirdPlace, None);
    }
    if lc.contains("semifinal") || lc.contains("semi-final") {
        return (Stage::SemiFinal, None);
    }
    if lc.contains("quarterfinal") || lc.contains("quarter-final") {
        return (Stage::QuarterFinal, None);
    }
    if lc.contains("final") {
        // Standalone "Final" — quarter/semi already returned above.
        return (Stage::Final, None);
    }
    if lc.contains("round of 16") || lc.contains("eighth-final") {
        return (Stage::RoundOf16, None);
    }
    if lc.contains("round of 32") || lc.contains("sixteenth") {
        return (Stage::RoundOf32, None);
    }
    if let Some(letter) = extract_group_letter(&lc) {
        return (Stage::Group, Some(letter));
    }
    (Stage::Group, None)
}

fn extract_group_letter(lc: &str) -> Option<String> {
    let idx = lc.find("group ")?;
    let rest = &lc[idx + 6..];
    rest.chars().next().filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_uppercase().to_string())
}

/// Map ESPN `event.season.slug` → tournament stage. Authoritative when set —
/// the scoreboard endpoint frequently returns empty `notes[]` so the headline
/// heuristic alone is unreliable for WC 2026.
fn classify_slug(slug: &str) -> Option<Stage> {
    match slug {
        "group-stage" => Some(Stage::Group),
        "round-of-32" => Some(Stage::RoundOf32),
        "round-of-16" => Some(Stage::RoundOf16),
        "quarterfinals" => Some(Stage::QuarterFinal),
        "semifinals" => Some(Stage::SemiFinal),
        "3rd-place-match" | "third-place-match" => Some(Stage::ThirdPlace),
        "final" => Some(Stage::Final),
        _ => None,
    }
}

/// Fetch the WC standings endpoint and build a `team_id → group letter (A–L)` map.
/// Used to enrich match.group_letter when the scoreboard payload omits it.
async fn fetch_groups_map(client: &Client) -> std::collections::HashMap<i32, String> {
    let url = "https://site.api.espn.com/apis/v2/sports/soccer/fifa.world/standings";
    let resp: StandingsResponse = match client.get(url).send().await {
        Ok(r) => r.json().await.unwrap_or_default(),
        Err(e) => {
            tracing::warn!("ESPN standings fetch failed: {:?}", e);
            return Default::default();
        }
    };
    let mut map = std::collections::HashMap::new();
    for group in resp.children {
        let Some(letter) = group
            .name
            .strip_prefix("Group ")
            .and_then(|s| s.chars().next())
            .filter(|c| c.is_ascii_alphabetic())
            .map(|c| c.to_ascii_uppercase().to_string())
        else {
            continue;
        };
        let Some(body) = group.standings else { continue };
        for entry in body.entries {
            if let Ok(tid) = entry.team.id.parse::<i32>() {
                map.insert(tid, letter.clone());
            }
        }
    }
    map
}

/// World Cup window: poll a generous date range around the tournament so
/// missed days during downtime get backfilled. Worker is idempotent via
/// `espn_event_id` upsert.
fn tournament_window() -> (NaiveDate, NaiveDate) {
    let start_str = std::env::var("WC_WINDOW_START").ok();
    let end_str = std::env::var("WC_WINDOW_END").ok();
    let start = start_str
        .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(2026, 6, 1).unwrap());
    let end = end_str
        .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(2026, 7, 25).unwrap());
    (start, end)
}

fn parse_espn_datetime(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%MZ")
                .map(|nd| nd.and_utc())
        })
        .ok()
}

/// Best-effort 3-letter (ESPN/FIFA) → ISO-3166 alpha-2 flag lookup.
/// Returns None for unknown codes — UI should fall back to short_name text.
fn flag_code_for_abbr(abbr: &str) -> Option<String> {
    let map: &[(&str, &str)] = &[
        ("ARG", "ar"), ("AUS", "au"), ("AUT", "at"), ("BEL", "be"), ("BRA", "br"),
        ("CAN", "ca"), ("CHI", "cl"), ("CHN", "cn"), ("COL", "co"), ("CRC", "cr"),
        ("CRO", "hr"), ("CZE", "cz"), ("DEN", "dk"), ("ECU", "ec"), ("EGY", "eg"),
        ("ENG", "gb-eng"), ("ESP", "es"), ("FRA", "fr"), ("GER", "de"), ("GHA", "gh"),
        ("GRE", "gr"), ("HON", "hn"), ("IRN", "ir"), ("IRQ", "iq"), ("ISL", "is"),
        ("ISR", "il"), ("ITA", "it"), ("JAM", "jm"), ("JPN", "jp"), ("KOR", "kr"),
        ("KSA", "sa"), ("MAR", "ma"), ("MEX", "mx"), ("NED", "nl"), ("NGA", "ng"),
        ("NOR", "no"), ("NZL", "nz"), ("PAN", "pa"), ("PAR", "py"), ("PER", "pe"),
        ("POL", "pl"), ("POR", "pt"), ("QAT", "qa"), ("ROU", "ro"), ("RSA", "za"),
        ("SCO", "gb-sct"), ("SEN", "sn"), ("SRB", "rs"), ("SUI", "ch"), ("SVK", "sk"),
        ("SVN", "si"), ("SWE", "se"), ("TUN", "tn"), ("TUR", "tr"), ("UKR", "ua"),
        ("URU", "uy"), ("USA", "us"), ("WAL", "gb-wls"),
    ];
    let upper = abbr.to_ascii_uppercase();
    map.iter()
        .find(|(k, _)| *k == upper)
        .map(|(_, v)| (*v).to_string())
}

// ─── Notifications ────────────────────────────────────────────────────────────

pub(crate) async fn process_notifications(
    repos: &Repos,
    notifier: &dyn Notifier,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if in_quiet_hours_now() {
        tracing::debug!("Quiet hours: skipping notification dispatch");
        return Ok(());
    }
    dispatch_pending(repos, notifier, Utc::now()).await
}

/// Inner dispatch routine — split out from `process_notifications` so tests
/// can drive it without depending on the wall-clock quiet-hours window.
async fn dispatch_pending(
    repos: &Repos,
    notifier: &dyn Notifier,
    now: DateTime<Utc>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1) Match closing in <24h with at least one user missing a tip.
    let closing = repos.notifications.list_closing_soon_unnotified().await?;

    for r in closing {
        let names = repos
            .notifications
            .users_missing_prediction_for(r.match_id)
            .await?;

        if names.is_empty() {
            continue;
        }

        let event = NotificationEvent::MatchClosingSoon {
            match_id: r.match_id,
            home: r.home,
            away: r.away,
            stage: r.stage,
            group_letter: r.group_letter,
            lock_at: r.kickoff_time,
            missing_names: names,
        };
        let _ = repos
            .notifications
            .try_send(notifier, "match_closing_soon", r.match_id, event)
            .await?;
    }

    // 2) Champion-tip lock approaching — anchored on the very first match.
    if let Some(lock_at) = repos.matches.first_kickoff().await? {
        if lock_at > now && lock_at <= now + chrono::Duration::hours(24) {
            let already = repos
                .notifications
                .already_sent("special_lock_soon", 0)
                .await?;

            if !already {
                let names = repos.notifications.users_missing_champion().await?;

                if !names.is_empty() {
                    let event = NotificationEvent::SpecialPredictionsLock {
                        lock_at,
                        missing_names: names,
                    };
                    let _ = repos
                        .notifications
                        .try_send(notifier, "special_lock_soon", 0, event)
                        .await?;
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_group_stage() {
        let (s, g) = classify_stage("Group A - Matchday 1");
        assert_eq!(s, Stage::Group);
        assert_eq!(g, Some("A".to_string()));
    }

    #[test]
    fn classify_group_letter_uppercased() {
        let (s, g) = classify_stage("FIFA World Cup - Group f");
        assert_eq!(s, Stage::Group);
        assert_eq!(g, Some("F".to_string()));
    }

    #[test]
    fn classify_round_of_32() {
        assert_eq!(classify_stage("Round of 32").0, Stage::RoundOf32);
    }

    #[test]
    fn classify_round_of_16() {
        assert_eq!(classify_stage("Round of 16").0, Stage::RoundOf16);
    }

    #[test]
    fn classify_quarter_final() {
        assert_eq!(classify_stage("Quarterfinals").0, Stage::QuarterFinal);
        assert_eq!(classify_stage("Quarter-final").0, Stage::QuarterFinal);
    }

    #[test]
    fn classify_semi_final() {
        assert_eq!(classify_stage("Semifinals").0, Stage::SemiFinal);
    }

    #[test]
    fn classify_third_place() {
        assert_eq!(classify_stage("Third-Place Match").0, Stage::ThirdPlace);
        assert_eq!(classify_stage("3rd Place").0, Stage::ThirdPlace);
    }

    #[test]
    fn classify_final() {
        assert_eq!(classify_stage("Final").0, Stage::Final);
    }

    #[test]
    fn classify_unknown_defaults_to_group() {
        assert_eq!(classify_stage("").0, Stage::Group);
    }

    #[test]
    fn flag_lookup_works_case_insensitive() {
        assert_eq!(flag_code_for_abbr("ger").as_deref(), Some("de"));
        assert_eq!(flag_code_for_abbr("USA").as_deref(), Some("us"));
        assert_eq!(flag_code_for_abbr("ENG").as_deref(), Some("gb-eng"));
    }

    #[test]
    fn flag_lookup_unknown() {
        assert!(flag_code_for_abbr("XYZ").is_none());
    }

    #[test]
    fn parse_espn_datetime_rfc3339() {
        let dt = parse_espn_datetime("2026-06-11T20:00:00Z").unwrap();
        assert_eq!(dt.to_rfc3339(), "2026-06-11T20:00:00+00:00");
    }

    #[test]
    fn parse_espn_datetime_no_seconds() {
        let dt = parse_espn_datetime("2026-06-11T20:00Z").unwrap();
        assert_eq!(dt.to_rfc3339(), "2026-06-11T20:00:00+00:00");
    }

    #[test]
    fn parse_espn_datetime_date_only_rejected() {
        assert!(parse_espn_datetime("2026-06-11").is_none());
    }

    #[test]
    fn tournament_window_default() {
        let (start, end) = tournament_window();
        assert!(start <= end);
    }

    // ─── End-to-end coverage of bootstrap + process_notifications via fakes ──

    use crate::notifier::NotifierError;
    use crate::repo::match_::{FakeMatch, MemoryMatchRepo};
    use crate::repo::notification::ClosingSoonMatch;
    use crate::repo::{
        MemoryNotificationRepo, MemoryPredictionRepo, MemorySettingsRepo,
        MemorySpecialPredictionRepo, MemoryTeamRepo, MemoryUserRepo, Repos,
    };
    use std::sync::Arc;
    use std::sync::Mutex;

    /// Notifier that records every event it receives, so tests can assert
    /// which notifications were dispatched without a real Signal endpoint.
    #[derive(Default)]
    struct RecordingNotifier {
        sent: Mutex<Vec<NotificationEvent>>,
    }

    #[async_trait::async_trait]
    impl Notifier for RecordingNotifier {
        async fn notify(&self, event: NotificationEvent) -> Result<(), NotifierError> {
            self.sent.lock().unwrap().push(event);
            Ok(())
        }
    }

    fn build_repos(notifications: Arc<MemoryNotificationRepo>) -> Repos {
        Repos {
            users: Arc::new(MemoryUserRepo::new()),
            matches: Arc::new(MemoryMatchRepo::new()),
            predictions: Arc::new(MemoryPredictionRepo::new()),
            special_predictions: Arc::new(MemorySpecialPredictionRepo::new()),
            teams: Arc::new(MemoryTeamRepo::new()),
            settings: Arc::new(MemorySettingsRepo::new()),
            notifications,
        }
    }

    #[tokio::test]
    async fn bootstrap_silences_existing_matches_and_is_idempotent() {
        let notifications = Arc::new(MemoryNotificationRepo::new());
        notifications.seed_match_with_both_teams(1);
        notifications.seed_match_with_both_teams(2);

        bootstrap_notifications(&*notifications).await.unwrap();
        assert!(notifications.was_bootstrapped_sync());
        assert_eq!(notifications.sent_count(), 2);

        // Second call must be a no-op — no extra rows, no double-silencing.
        bootstrap_notifications(&*notifications).await.unwrap();
        assert_eq!(notifications.sent_count(), 2);
    }

    #[tokio::test]
    async fn process_notifications_dispatches_match_closing_soon() {
        let notifications = Arc::new(MemoryNotificationRepo::new());
        notifications.seed_user("Anna");
        notifications.seed_user("Ben");
        notifications.seed_prediction("Ben", 7);
        notifications.seed_closing_soon(ClosingSoonMatch {
            match_id: 7,
            stage: Stage::Group,
            group_letter: Some("A".into()),
            kickoff_time: Utc::now() + chrono::Duration::hours(2),
            home: "Argentinien".into(),
            away: "Brasilien".into(),
        });

        let repos = build_repos(notifications.clone());
        let notifier = RecordingNotifier::default();
        dispatch_pending(&repos, &notifier, Utc::now()).await.unwrap();

        let sent = notifier.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        match &sent[0] {
            NotificationEvent::MatchClosingSoon {
                home,
                away,
                missing_names,
                ..
            } => {
                assert_eq!(home, "Argentinien");
                assert_eq!(away, "Brasilien");
                assert_eq!(missing_names, &vec!["Anna".to_string()]);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn process_notifications_skips_matches_with_no_missing_tippers() {
        let notifications = Arc::new(MemoryNotificationRepo::new());
        notifications.seed_user("Anna");
        notifications.seed_prediction("Anna", 7);
        notifications.seed_closing_soon(ClosingSoonMatch {
            match_id: 7,
            stage: Stage::Group,
            group_letter: None,
            kickoff_time: Utc::now() + chrono::Duration::hours(2),
            home: "X".into(),
            away: "Y".into(),
        });

        let repos = build_repos(notifications.clone());
        let notifier = RecordingNotifier::default();
        dispatch_pending(&repos, &notifier, Utc::now()).await.unwrap();
        assert!(
            notifier.sent.lock().unwrap().is_empty(),
            "no missing tippers → no notification"
        );
        assert_eq!(notifications.sent_count(), 0);
    }

    #[tokio::test]
    async fn process_notifications_does_not_repeat_sent_match_closing_soon() {
        let notifications = Arc::new(MemoryNotificationRepo::new());
        notifications.seed_user("Anna");
        notifications.seed_closing_soon(ClosingSoonMatch {
            match_id: 7,
            stage: Stage::Group,
            group_letter: Some("A".into()),
            kickoff_time: Utc::now() + chrono::Duration::hours(2),
            home: "X".into(),
            away: "Y".into(),
        });
        let repos = build_repos(notifications.clone());
        let notifier = RecordingNotifier::default();

        // First tick: dispatches.
        dispatch_pending(&repos, &notifier, Utc::now()).await.unwrap();
        assert_eq!(notifier.sent.lock().unwrap().len(), 1);

        // Second tick: must be a no-op for the same match_id.
        dispatch_pending(&repos, &notifier, Utc::now()).await.unwrap();
        assert_eq!(notifier.sent.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn process_notifications_dispatches_special_lock_when_first_kickoff_is_soon() {
        let notifications = Arc::new(MemoryNotificationRepo::new());
        notifications.seed_user("Anna");
        notifications.seed_user_with_champion("Ben");

        // First-kickoff source is the matches repo, not notifications.
        let matches = Arc::new(MemoryMatchRepo::new());
        matches.seed(FakeMatch::locked_unfinished(
            1,
            Utc::now() + chrono::Duration::hours(6),
        ));
        let mut repos = build_repos(notifications.clone());
        repos.matches = matches;

        let notifier = RecordingNotifier::default();
        dispatch_pending(&repos, &notifier, Utc::now()).await.unwrap();

        let sent = notifier.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        match &sent[0] {
            NotificationEvent::SpecialPredictionsLock { missing_names, .. } => {
                assert_eq!(missing_names, &vec!["Anna".to_string()]);
            }
            other => panic!("unexpected event: {other:?}"),
        }
        assert!(notifications.already_sent("special_lock_soon", 0).await.unwrap());
    }

    #[tokio::test]
    async fn process_notifications_skips_special_lock_when_kickoff_is_far_away() {
        let notifications = Arc::new(MemoryNotificationRepo::new());
        notifications.seed_user("Anna");

        let matches = Arc::new(MemoryMatchRepo::new());
        matches.seed(FakeMatch::locked_unfinished(
            1,
            Utc::now() + chrono::Duration::hours(48),
        ));
        let mut repos = build_repos(notifications.clone());
        repos.matches = matches;

        let notifier = RecordingNotifier::default();
        dispatch_pending(&repos, &notifier, Utc::now()).await.unwrap();
        assert!(notifier.sent.lock().unwrap().is_empty());
        assert!(!notifications.already_sent("special_lock_soon", 0).await.unwrap());
    }

    #[tokio::test]
    async fn process_notifications_skips_special_lock_when_no_users_pending() {
        let notifications = Arc::new(MemoryNotificationRepo::new());
        notifications.seed_user_with_champion("Anna");
        notifications.seed_user_with_champion("Ben");

        let matches = Arc::new(MemoryMatchRepo::new());
        matches.seed(FakeMatch::locked_unfinished(
            1,
            Utc::now() + chrono::Duration::hours(6),
        ));
        let mut repos = build_repos(notifications.clone());
        repos.matches = matches;

        let notifier = RecordingNotifier::default();
        dispatch_pending(&repos, &notifier, Utc::now()).await.unwrap();
        assert!(notifier.sent.lock().unwrap().is_empty());
        assert!(!notifications.already_sent("special_lock_soon", 0).await.unwrap());
    }
}
