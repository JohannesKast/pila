//! ESPN `soccer/fifa.world` scoreboard implementation of `ScoreboardClient`.
//!
//! All ESPN-specific quirks live here:
//!   - Two-endpoint design: scoreboard returns fixtures, standings returns
//!     team→group letter. The standings response is fetched lazily on the
//!     first `fetch_events` call and cached for the lifetime of the
//!     `EspnClient` (group draws are stable for the tournament).
//!   - `notes[].headline` heuristic + `season.slug` for stage classification.
//!   - 3-letter FIFA abbreviation → ISO-3166 alpha-2 flag mapping.
//!
//! Worker code never sees the raw shapes; this module hands back clean
//! `SportsEvent`s.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::{MatchStatus, ProviderError, ScoreboardClient, SportsEvent, SportsTeam};
use crate::stage::Stage;

pub struct EspnClient {
    http: Client,
    /// Lazily-populated team-id → group-letter cache. ESPN's standings
    /// endpoint is the only authoritative source for group assignments
    /// when the scoreboard's `notes[].headline` field is empty (frequent).
    /// Held in an async mutex so cache fills do not block other tokio
    /// tasks waiting on a sync mutex.
    groups_cache: Arc<Mutex<Option<HashMap<i32, String>>>>,
}

impl EspnClient {
    pub fn new() -> Self {
        Self {
            http: Client::new(),
            groups_cache: Arc::new(Mutex::new(None)),
        }
    }

    async fn groups_map(&self) -> HashMap<i32, String> {
        let mut guard = self.groups_cache.lock().await;
        if let Some(map) = guard.as_ref() {
            return map.clone();
        }
        let map = fetch_groups_map(&self.http).await;
        if !map.is_empty() {
            tracing::info!(
                "Loaded group letters for {} teams from standings",
                map.len()
            );
        }
        *guard = Some(map.clone());
        map
    }
}

impl Default for EspnClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ScoreboardClient for EspnClient {
    async fn fetch_events(&self, date: NaiveDate) -> Result<Vec<SportsEvent>, ProviderError> {
        let date_str = date.format("%Y%m%d").to_string();
        let url = format!(
            "https://site.api.espn.com/apis/site/v2/sports/soccer/fifa.world/scoreboard?dates={date_str}"
        );

        let resp = self.http.get(&url).send().await?;
        let body: ScoreboardResponse = resp.json().await.unwrap_or_default();

        let groups_map = self.groups_map().await;

        Ok(body
            .events
            .into_iter()
            .filter_map(|e| event_to_sports_event(e, &groups_map))
            .collect())
    }
}

// ─── ESPN response structs (private) ──────────────────────────────────────────

#[derive(Deserialize, Default)]
struct ScoreboardResponse {
    #[serde(default)]
    events: Vec<EspnEvent>,
}

#[derive(Deserialize)]
struct EspnEvent {
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
    team: StandingsEntryTeam,
}

#[derive(Deserialize)]
struct StandingsEntryTeam {
    id: String,
}

// ─── Mapping ESPN → SportsEvent ──────────────────────────────────────────────

fn event_to_sports_event(
    event: EspnEvent,
    groups_map: &HashMap<i32, String>,
) -> Option<SportsEvent> {
    let comp = event.competitions.into_iter().next()?;
    let provider_event_id: i64 = event.id.parse().ok()?;

    let headline = comp
        .notes
        .iter()
        .map(|n| n.headline.as_str())
        .find(|s| !s.is_empty())
        .unwrap_or("");
    let slug = event.season.as_ref().map(|s| s.slug.as_str()).unwrap_or("");

    // Slug is authoritative for stage; headline only as fallback. Headline
    // still useful for group-letter (when present), but ESPN frequently
    // leaves notes empty.
    let (stage, mut group_letter) = match classify_slug(slug) {
        Some(s) => (s, classify_stage(headline).1),
        None => classify_stage(headline),
    };

    let home = comp.competitors.iter().find(|c| c.home_away == "home");
    let away = comp.competitors.iter().find(|c| c.home_away == "away");
    let home_id: Option<i32> = home.and_then(|c| c.team.id.parse().ok());
    let away_id: Option<i32> = away.and_then(|c| c.team.id.parse().ok());

    if stage == Stage::Group && group_letter.is_none() {
        group_letter = home_id
            .and_then(|id| groups_map.get(&id).cloned())
            .or_else(|| away_id.and_then(|id| groups_map.get(&id).cloned()));
    }

    let team_for = |c: Option<&Competitor>, id: Option<i32>| -> Option<SportsTeam> {
        let (c, id) = (c?, id?);
        let short_name = if c.team.abbreviation.is_empty() {
            None
        } else {
            Some(c.team.abbreviation.clone())
        };
        Some(SportsTeam {
            provider_team_id: id,
            display_name: c.team.display_name.clone(),
            short_name,
            flag_code: flag_code_for_abbr(&c.team.abbreviation).map(|s| s.to_string()),
        })
    };

    let home_team = team_for(home, home_id);
    let away_team = team_for(away, away_id);

    let score_home = home
        .and_then(|c| c.score.as_deref())
        .and_then(|s| s.parse::<i32>().ok());
    let score_away = away
        .and_then(|c| c.score.as_deref())
        .and_then(|s| s.parse::<i32>().ok());

    let kickoff = parse_espn_datetime(&comp.start_date);
    let status = comp
        .status
        .as_ref()
        .map(|s| match s.type_.state.as_str() {
            "in" => MatchStatus::Live,
            "post" => MatchStatus::Finished,
            _ => MatchStatus::Scheduled,
        })
        .unwrap_or(MatchStatus::Scheduled);

    Some(SportsEvent {
        provider_event_id,
        stage,
        group_letter,
        home_team,
        away_team,
        score_home,
        score_away,
        kickoff,
        status,
    })
}

// ─── Standings endpoint ─────────────────────────────────────────────────────

async fn fetch_groups_map(client: &Client) -> HashMap<i32, String> {
    let url = "https://site.api.espn.com/apis/v2/sports/soccer/fifa.world/standings";
    let resp: StandingsResponse = match client.get(url).send().await {
        Ok(r) => r.json().await.unwrap_or_default(),
        Err(e) => {
            tracing::warn!("ESPN standings fetch failed: {:?}", e);
            return Default::default();
        }
    };
    let mut map = HashMap::new();
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

// ─── Stage classification + datetime + flag helpers ─────────────────────────

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
    rest.chars()
        .next()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_uppercase().to_string())
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

fn parse_espn_datetime(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%MZ").map(|nd| nd.and_utc())
        })
        .ok()
}

/// Best-effort 3-letter (ESPN/FIFA) → ISO-3166 alpha-2 flag lookup.
/// Returns None for unknown codes — UI should fall back to short_name text.
fn flag_code_for_abbr(abbr: &str) -> Option<&'static str> {
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
        .map(|(_, v)| *v)
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
    fn classify_slug_round_of_32() {
        assert_eq!(classify_slug("round-of-32"), Some(Stage::RoundOf32));
    }

    #[test]
    fn classify_slug_third_place_alternates() {
        assert_eq!(classify_slug("3rd-place-match"), Some(Stage::ThirdPlace));
        assert_eq!(
            classify_slug("third-place-match"),
            Some(Stage::ThirdPlace)
        );
    }

    #[test]
    fn classify_slug_unknown_is_none() {
        assert!(classify_slug("preseason").is_none());
    }

    #[test]
    fn flag_lookup_works_case_insensitive() {
        assert_eq!(flag_code_for_abbr("ger"), Some("de"));
        assert_eq!(flag_code_for_abbr("USA"), Some("us"));
        assert_eq!(flag_code_for_abbr("ENG"), Some("gb-eng"));
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
}
