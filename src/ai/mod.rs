// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! AI matchday recaps.
//!
//! Once the last match of a matchday has finished, the background worker
//! generates exactly one recap per league (in the league's default language)
//! and stores it. The recap is shown at the top of the "Current" tab. See
//! [`prompt`] for the prompt, [`data`] for the structured input, and
//! [`client`] for the provider-agnostic model call.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::badges::PredictionRow;
use crate::repo::{MatchdayReport, Repos};
use crate::translations::{self, T};

pub mod client;
pub mod data;
pub mod prompt;

pub use client::{AiConfig, AiError};

#[async_trait]
trait RecapGenerator: Send + Sync {
    async fn generate(&self, cfg: &AiConfig, system: &str, user: &str) -> Result<String, AiError>;
}

struct ClientRecapGenerator;

#[async_trait]
impl RecapGenerator for ClientRecapGenerator {
    async fn generate(&self, cfg: &AiConfig, system: &str, user: &str) -> Result<String, AiError> {
        client::generate(cfg, system, user).await
    }
}

/// Generate any due recaps for every league. A league has a due recap when its
/// most recent fully-finished matchday has no stored recap yet. Per-league
/// failures are logged and skipped so one league cannot block the others.
pub async fn generate_due_reports(
    repos: &Repos,
    cfg: &AiConfig,
    translations: &HashMap<String, T>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    generate_due_reports_with(repos, cfg, translations, &ClientRecapGenerator).await
}

async fn generate_due_reports_with(
    repos: &Repos,
    cfg: &AiConfig,
    translations: &HashMap<String, T>,
    generator: &(dyn RecapGenerator),
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let leagues = repos.leagues.list().await?;
    for league in leagues {
        if let Err(e) = generate_for_league(repos, cfg, translations, league.id, generator).await {
            tracing::error!(
                "AI recap generation failed for league {} ({}): {:?}",
                league.name,
                league.id,
                e
            );
        }
    }
    Ok(())
}

async fn generate_for_league(
    repos: &Repos,
    cfg: &AiConfig,
    translations: &HashMap<String, T>,
    league_id: Uuid,
    generator: &(dyn RecapGenerator),
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let league_cfg = repos.leagues.get_config(league_id).await?;
    let summaries = repos.matches.list_all_summaries().await?;

    let Some(day) = data::latest_finished_matchday(
        &summaries,
        cfg.matchday_tz,
        league_cfg.predict_knockout_only,
    ) else {
        return Ok(());
    };
    if repos.reports.exists(league_id, day).await? {
        return Ok(());
    }

    let users = repos.users.list_basic(league_id).await?;
    let users_id_name: Vec<(Uuid, String)> = users.iter().map(|u| (u.id, u.name.clone())).collect();

    let finished: Vec<PredictionRow> = repos
        .predictions
        .list_finished_join(league_id)
        .await?
        .into_iter()
        .map(|r| PredictionRow {
            user_id: r.user_id,
            match_id: r.match_id,
            stage: r.stage,
            kickoff: r.kickoff,
            score_h: r.score_home,
            score_a: r.score_away,
            pred_h: r.predicted_home,
            pred_a: r.predicted_away,
            scoring_system: league_cfg.match_scoring_system,
        })
        .collect();

    let special_picks = repos.special_predictions.list_all_picks(league_id).await?;
    let team_names: HashMap<i32, String> = repos
        .teams
        .list_real_for_dropdown()
        .await?
        .into_iter()
        .map(|t| (t.id, t.name))
        .collect();
    let actual_champion = repos.matches.actual_champion().await?;

    let now = Utc::now();
    let started_total = repos.matches.started_with_both_teams_count(now).await? as i32;
    let mut started_by_user = HashMap::new();
    for (uid, _) in &users_id_name {
        let n = repos.predictions.count_user_started(*uid, now).await? as i32;
        started_by_user.insert(*uid, n);
    }

    // Badge display names are read in English: the prompt is English and player
    // data is fed to the model in English regardless of the output language.
    let badge_t = translations::resolve(translations, "en");

    let source = data::ReportSource {
        matchday_date: day,
        tz: cfg.matchday_tz,
        scoring_system: league_cfg.match_scoring_system,
        summaries: &summaries,
        finished,
        users: users_id_name,
        special_picks,
        team_names,
        actual_champion,
        started_total,
        started_by_user,
        badge_t: &badge_t,
        now,
    };
    let input = data::build_report_input(&source);
    let json = serde_json::to_string_pretty(&input)?;

    let language = league_cfg.default_language.clone();
    let system = prompt::system_prompt(prompt::language_name(&language));
    let user = prompt::user_prompt(&json);

    // On failure: store nothing. The next worker tick retries automatically.
    let content = generator.generate(cfg, &system, &user).await?;

    repos
        .reports
        .insert(&MatchdayReport {
            league_id,
            matchday_date: day,
            language,
            content,
            model: cfg.model_ref(),
            generated_at: Utc::now(),
        })
        .await?;

    tracing::info!("Generated AI matchday recap for league {league_id} on {day}");
    Ok(())
}

/// Render trusted-but-model-authored Markdown to HTML, dropping any embedded raw
/// HTML so a model response can never inject markup. Used by the dashboard to
/// display a stored recap.
pub fn markdown_to_safe_html(markdown: &str) -> String {
    use pulldown_cmark::{html, Event, Options, Parser};
    let markdown = strip_outer_code_fence(markdown);
    let parser = Parser::new_ext(&markdown, Options::ENABLE_STRIKETHROUGH).map(|ev| match ev {
        // Strip raw HTML blocks and inline HTML — render them as nothing.
        Event::Html(_) | Event::InlineHtml(_) => Event::Text("".into()),
        other => other,
    });
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

/// Some models wrap their whole answer in a ```` ``` ```` / ````` ```markdown `````
/// fence. Rendered as-is that becomes a literal grey code block, so unwrap a
/// fence that spans the entire response before parsing. Leaves normal content
/// (and inner code blocks) untouched.
fn strip_outer_code_fence(markdown: &str) -> String {
    let trimmed = markdown.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return markdown.to_string();
    };
    // The opening fence may carry an info string (e.g. "markdown"); drop its line.
    let Some((info, body)) = rest.split_once('\n') else {
        return markdown.to_string();
    };
    // Only treat it as a wrapper when the info string is a bare language tag,
    // never a fence that immediately opens an inner block.
    if info.trim().contains('`') {
        return markdown.to_string();
    }
    match body.trim_end().strip_suffix("```") {
        Some(inner) => inner.trim().to_string(),
        None => markdown.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::FakeMatch;
    use crate::repo::prediction::FakeFinishedRow;
    use crate::repo::user::UserFull;
    use crate::repo::{
        League, LeagueConfig, LeagueRepo, MatchdayReportRepo, MemoryBootstrapRepo,
        MemoryInviteRepo, MemoryLeagueRepo, MemoryMatchRepo, MemoryMatchdayReportRepo,
        MemoryNotificationRepo, MemoryPredictionRepo, MemorySettingsRepo,
        MemorySpecialPredictionRepo, MemoryTeamRepo, MemoryUserRepo, PredictionRepo, Repos,
    };
    use crate::stage::Stage;
    use async_trait::async_trait;
    use chrono::{NaiveDate, TimeZone};
    use std::sync::{Arc, Mutex};

    struct TestRepos {
        repos: Repos,
        leagues: Arc<MemoryLeagueRepo>,
        users: Arc<MemoryUserRepo>,
        matches: Arc<MemoryMatchRepo>,
        predictions: Arc<MemoryPredictionRepo>,
        special_predictions: Arc<MemorySpecialPredictionRepo>,
        reports: Arc<MemoryMatchdayReportRepo>,
    }

    fn test_repos() -> TestRepos {
        let leagues = Arc::new(MemoryLeagueRepo::new());
        let users = Arc::new(MemoryUserRepo::new());
        let matches = Arc::new(MemoryMatchRepo::new());
        let predictions = Arc::new(MemoryPredictionRepo::new());
        let special_predictions = Arc::new(MemorySpecialPredictionRepo::new());
        let reports = Arc::new(MemoryMatchdayReportRepo::new());

        let repos = Repos {
            bootstrap: Arc::new(MemoryBootstrapRepo::new()),
            users: users.clone(),
            leagues: leagues.clone(),
            matches: matches.clone(),
            predictions: predictions.clone(),
            special_predictions: special_predictions.clone(),
            teams: Arc::new(MemoryTeamRepo::new()),
            settings: Arc::new(MemorySettingsRepo::new()),
            invites: Arc::new(MemoryInviteRepo::new()),
            notifications: Arc::new(MemoryNotificationRepo::new()),
            reports: reports.clone(),
        };

        TestRepos {
            repos,
            leagues,
            users,
            matches,
            predictions,
            special_predictions,
            reports,
        }
    }

    fn ai_cfg() -> AiConfig {
        AiConfig {
            provider: "gemini".to_string(),
            model: "gemini-2.5-flash".to_string(),
            api_key: "test-key".to_string(),
            base_url: None,
            matchday_tz: chrono_tz::America::New_York,
        }
    }

    fn translations() -> HashMap<String, T> {
        HashMap::from([
            ("de".to_string(), T::default()),
            ("en".to_string(), T::default()),
        ])
    }

    fn user(id: Uuid, name: &str, league_id: Uuid) -> UserFull {
        UserFull {
            id,
            name: name.to_string(),
            real_name: format!("{name} Real"),
            token: format!("token-{id}"),
            phone_number: None,
            email: None,
            is_admin: false,
            can_create_league: false,
            league_id,
            language: "en".to_string(),
        }
    }

    async fn seed_due_matchday(ctx: &TestRepos, league_id: Uuid) -> Uuid {
        ctx.leagues.seed(League {
            id: league_id,
            name: "League".to_string(),
            notifications_bootstrapped: false,
        });
        ctx.leagues
            .set_setting(league_id, LeagueConfig::KEY_DEFAULT_LANGUAGE, Some("en"))
            .await
            .unwrap();

        let alice = Uuid::new_v4();
        ctx.users.seed(user(alice, "Alice", league_id), "classic");
        ctx.special_predictions.seed_user(alice, league_id, "Alice");

        let kickoff = chrono::Utc.with_ymd_and_hms(2026, 6, 11, 20, 0, 0).unwrap();
        ctx.matches.seed(FakeMatch {
            id: 1,
            stage: Stage::Group,
            group_letter: Some("A".to_string()),
            kickoff_time: Some(kickoff),
            status: "finished".to_string(),
            score_home: Some(2),
            score_away: Some(1),
            team_home_id: Some(10),
            team_away_id: Some(20),
            home_name: "Team A".to_string(),
            away_name: "Team B".to_string(),
            home_flag: None,
            away_flag: None,
        });
        ctx.predictions.upsert(alice, 1, 2, 1).await.unwrap();
        ctx.predictions.seed_finished(FakeFinishedRow {
            user_id: alice,
            league_id,
            match_id: 1,
            stage: Stage::Group,
            kickoff,
            score_home: 2,
            score_away: 1,
            predicted_home: 2,
            predicted_away: 1,
        });

        alice
    }

    struct FakeGenerator {
        content: &'static str,
        fail: bool,
        calls: Mutex<Vec<(String, String)>>,
    }

    impl FakeGenerator {
        fn ok(content: &'static str) -> Self {
            Self {
                content,
                fail: false,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn failing() -> Self {
            Self {
                content: "",
                fail: true,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl RecapGenerator for FakeGenerator {
        async fn generate(
            &self,
            _cfg: &AiConfig,
            system: &str,
            user: &str,
        ) -> Result<String, AiError> {
            self.calls
                .lock()
                .unwrap()
                .push((system.to_string(), user.to_string()));
            if self.fail {
                Err(AiError::Failed("model offline".to_string()))
            } else {
                Ok(self.content.to_string())
            }
        }
    }

    #[test]
    fn markdown_renders_headings_and_paragraphs() {
        let html = markdown_to_safe_html("## Title\n\nHello **world**.");
        assert!(html.contains("<h2>"));
        assert!(html.contains("<strong>world</strong>"));
    }

    #[test]
    fn markdown_strips_raw_html() {
        let html = markdown_to_safe_html("ok <script>alert(1)</script> done");
        assert!(!html.contains("<script>"));
        assert!(html.contains("ok"));
        assert!(html.contains("done"));
    }

    #[test]
    fn markdown_unwraps_outer_code_fence() {
        // A model wrapping the whole answer in a ```markdown fence must still
        // render as real headings, not a literal code block.
        let html = markdown_to_safe_html("```markdown\n## Title\n\nHi **there**.\n```");
        assert!(html.contains("<h2>"));
        assert!(html.contains("<strong>there</strong>"));
        assert!(!html.contains("<pre>"));
    }

    #[test]
    fn markdown_keeps_inner_code_blocks() {
        let html = markdown_to_safe_html("Text\n\n```\ncode\n```\n");
        assert!(html.contains("<pre>"));
        assert!(html.contains("code"));
    }

    #[tokio::test]
    async fn generate_due_reports_stores_generated_recap() {
        let ctx = test_repos();
        let league_id = Uuid::new_v4();
        seed_due_matchday(&ctx, league_id).await;

        let generator = FakeGenerator::ok("## Recap\n\nAlice owns the matchday.");
        generate_due_reports_with(&ctx.repos, &ai_cfg(), &translations(), &generator)
            .await
            .unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 6, 11).unwrap();
        let report = ctx.reports.get(league_id, date).await.unwrap().unwrap();
        assert_eq!(report.content, "## Recap\n\nAlice owns the matchday.");
        assert_eq!(report.language, "en");
        assert_eq!(report.model, "gemini::gemini-2.5-flash");

        let calls = generator.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].1.contains("\"name\": \"Alice\""));
        assert!(calls[0].1.contains("\"home\": \"Team A\""));
    }

    #[tokio::test]
    async fn generate_due_reports_skips_existing_recap() {
        let ctx = test_repos();
        let league_id = Uuid::new_v4();
        seed_due_matchday(&ctx, league_id).await;
        let date = NaiveDate::from_ymd_opt(2026, 6, 11).unwrap();
        ctx.reports
            .insert(&MatchdayReport {
                league_id,
                matchday_date: date,
                language: "en".to_string(),
                content: "already done".to_string(),
                model: "manual".to_string(),
                generated_at: Utc::now(),
            })
            .await
            .unwrap();

        let generator = FakeGenerator::ok("new content");
        generate_due_reports_with(&ctx.repos, &ai_cfg(), &translations(), &generator)
            .await
            .unwrap();

        let report = ctx.reports.get(league_id, date).await.unwrap().unwrap();
        assert_eq!(report.content, "already done");
        assert!(generator.calls().is_empty());
    }

    #[tokio::test]
    async fn generate_due_reports_ignores_leagues_without_finished_matchday() {
        let ctx = test_repos();
        let league_id = Uuid::new_v4();
        ctx.leagues.seed(League {
            id: league_id,
            name: "Empty".to_string(),
            notifications_bootstrapped: false,
        });

        let generator = FakeGenerator::ok("unused");
        generate_due_reports_with(&ctx.repos, &ai_cfg(), &translations(), &generator)
            .await
            .unwrap();

        assert!(ctx.reports.latest_date(league_id).await.unwrap().is_none());
        assert!(generator.calls().is_empty());
    }

    #[tokio::test]
    async fn generate_due_reports_logs_model_failure_and_continues() {
        let ctx = test_repos();
        let league_id = Uuid::new_v4();
        seed_due_matchday(&ctx, league_id).await;

        let generator = FakeGenerator::failing();
        generate_due_reports_with(&ctx.repos, &ai_cfg(), &translations(), &generator)
            .await
            .unwrap();

        assert!(ctx.reports.latest_date(league_id).await.unwrap().is_none());
        assert_eq!(generator.calls().len(), 1);
    }
}
