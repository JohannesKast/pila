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

use chrono::Utc;
use uuid::Uuid;

use crate::badges::PredictionRow;
use crate::repo::{MatchdayReport, Repos};
use crate::translations::{self, T};

pub mod client;
pub mod data;
pub mod prompt;

pub use client::{AiConfig, AiError};

/// Generate any due recaps for every league. A league has a due recap when its
/// most recent fully-finished matchday has no stored recap yet. Per-league
/// failures are logged and skipped so one league cannot block the others.
pub async fn generate_due_reports(
    repos: &Repos,
    cfg: &AiConfig,
    translations: &HashMap<String, T>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let leagues = repos.leagues.list().await?;
    for league in leagues {
        if let Err(e) = generate_for_league(repos, cfg, translations, league.id).await {
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
    let content = client::generate(cfg, &system, &user).await?;

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
}
