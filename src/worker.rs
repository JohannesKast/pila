// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Background tournament-data sync + notification dispatch.
//!
//! Worker depends only on the repo abstraction (DB) and the
//! `ScoreboardClient` abstraction (sports data). Neither sqlx nor reqwest
//! is referenced from this file — each tick is exercisable end-to-end
//! against in-memory fakes.

use chrono::{DateTime, NaiveDate, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::mail;
use crate::notifier::{
    in_quiet_hours_now, NoopNotifier, NotificationEvent, Notifier, SignalNotifier,
};
use crate::repo::fixture::EspnMatchUpsert;
use crate::repo::league::League;
use crate::repo::team::EspnTeamUpsert;
use crate::repo::Repos;
use crate::scoreboard::{ScoreboardClient, SportsEvent};
use crate::translations::{self, T};

// ─── Public entry points ─────────────────────────────────────────────────────

/// Spawn the 30-minute background loop. Pulls fixtures from the configured
/// scoreboard provider, upserts them via the repo layer, and dispatches due
/// notifications **per league**: every league iterates its own missing-tip
/// list and uses its own Signal config (or a no-op if the league has none).
pub async fn start_background_worker(
    repos: Repos,
    scoreboard: Arc<dyn ScoreboardClient>,
    base_url: String,
    signal_api_url: Option<String>,
    smtp_config: Option<crate::mail::SmtpConfig>,
    translations: HashMap<String, T>,
) {
    tokio::spawn(async move {
        loop {
            tracing::info!("Running scoreboard update...");
            if let Err(e) = update_data(&*scoreboard, &repos).await {
                tracing::error!("Scoreboard worker error: {:?}", e);
            }
            if let Err(e) = process_notifications(
                &repos,
                &base_url,
                &signal_api_url,
                &smtp_config,
                &translations,
            )
            .await
            {
                tracing::error!("Notification processing error: {:?}", e);
            }
            tokio::time::sleep(Duration::from_secs(1800)).await;
        }
    });
}

/// On every startup, walk every league and silence any not-yet-bootstrapped
/// one. New leagues created after deploy go through this path on the next
/// boot so a freshly created league does not flood its group with stale
/// reminders.
pub async fn bootstrap_notifications(
    repos: &Repos,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let leagues = repos.leagues.list().await?;
    for league in leagues {
        if league.notifications_bootstrapped {
            continue;
        }
        repos
            .notifications
            .silence_existing_matches(league.id)
            .await?;
        repos.leagues.set_bootstrapped(league.id).await?;
        tracing::info!(
            "Notification bootstrap complete for league {} ({}) — current open matches silenced",
            league.name,
            league.id
        );
    }
    Ok(())
}

// ─── Sync loop ───────────────────────────────────────────────────────────────

pub async fn update_data(
    scoreboard: &dyn ScoreboardClient,
    repos: &Repos,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (window_start, window_end) = tournament_window();
    let mut current = window_start;
    let mut total_events = 0usize;

    while current <= window_end {
        match scoreboard.fetch_events(current).await {
            Ok(events) => {
                total_events += events.len();
                for ev in events {
                    if let Err(e) = upsert_event(repos, &ev).await {
                        tracing::warn!("event {} processing failed: {:?}", ev.provider_event_id, e);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Scoreboard fetch failed for {}: {:?}", current, e);
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

async fn upsert_event(
    repos: &Repos,
    event: &SportsEvent,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Only attach group_letter on group-stage events; knockout teams may
    // already carry their group letter from earlier syncs.
    let group_for_team: Option<&str> = match event.stage {
        crate::stage::Stage::Group => event.group_letter.as_deref(),
        _ => None,
    };

    if let Some(home) = &event.home_team {
        repos
            .teams
            .upsert_from_espn(EspnTeamUpsert {
                espn_id: home.provider_team_id,
                name: &home.display_name,
                short_name: home.short_name.as_deref(),
                flag_code: home.flag_code.as_deref(),
                group_letter: group_for_team,
            })
            .await?;
    }
    if let Some(away) = &event.away_team {
        repos
            .teams
            .upsert_from_espn(EspnTeamUpsert {
                espn_id: away.provider_team_id,
                name: &away.display_name,
                short_name: away.short_name.as_deref(),
                flag_code: away.flag_code.as_deref(),
                group_letter: group_for_team,
            })
            .await?;
    }

    repos
        .matches
        .upsert_from_espn(EspnMatchUpsert {
            espn_event_id: event.provider_event_id,
            stage: event.stage,
            group_letter: event.group_letter.as_deref(),
            team_home_id: event.home_team.as_ref().map(|t| t.provider_team_id),
            team_away_id: event.away_team.as_ref().map(|t| t.provider_team_id),
            score_home: event.score_home,
            score_away: event.score_away,
            kickoff_time: event.kickoff,
            status: event.status.as_db_str(),
        })
        .await?;

    Ok(())
}

/// World Cup window: poll a generous date range around the tournament so
/// missed days during downtime get backfilled. Worker is idempotent via
/// `espn_event_id` upsert.
fn tournament_window() -> (NaiveDate, NaiveDate) {
    let start_str = std::env::var("WC_WINDOW_START").ok();
    let end_str = std::env::var("WC_WINDOW_END").ok();
    let start = start_str
        .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(2026, 6, 1).expect("2026-06-01 is valid"));
    let end = end_str
        .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(2026, 7, 25).expect("2026-07-25 is valid"));
    (start, end)
}

// ─── Notifications ────────────────────────────────────────────────────────────

pub(crate) async fn process_notifications(
    repos: &Repos,
    base_url: &str,
    signal_api_url: &Option<String>,
    smtp_config: &Option<mail::SmtpConfig>,
    translations: &HashMap<String, T>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if in_quiet_hours_now() {
        tracing::debug!("Quiet hours: skipping notification dispatch");
        return Ok(());
    }
    dispatch_pending_for_all_leagues(
        repos,
        Utc::now(),
        base_url,
        signal_api_url,
        smtp_config,
        translations,
    )
    .await
}

/// Iterate every league, build a per-league notifier from its `LeagueConfig`,
/// and dispatch pending notifications using that notifier. Leagues without a
/// Signal group fall back to `NoopNotifier` and stay silent.
pub(crate) async fn dispatch_pending_for_all_leagues(
    repos: &Repos,
    now: DateTime<Utc>,
    base_url: &str,
    signal_api_url: &Option<String>,
    smtp_config: &Option<mail::SmtpConfig>,
    translations: &HashMap<String, T>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let leagues = repos.leagues.list().await?;
    for league in leagues {
        let cfg = repos.leagues.get_config(league.id).await?;
        let t = translations::resolve(translations, &cfg.default_language);
        let notifier: Box<dyn Notifier> = match (
            signal_api_url,
            &cfg.signal_group_id,
            &cfg.signal_from_number,
        ) {
            (Some(api), Some(gid), Some(from))
                if !api.is_empty() && !gid.is_empty() && !from.is_empty() =>
            {
                Box::new(SignalNotifier::new(api, from, gid, base_url, t.clone()))
            }
            _ => Box::new(NoopNotifier),
        };
        if let Err(e) = dispatch_pending_for_league(
            repos,
            &league,
            notifier.as_ref(),
            now,
            base_url,
            smtp_config,
            &t,
        )
        .await
        {
            tracing::error!(
                "Notification dispatch failed for league {} ({}): {:?}",
                league.name,
                league.id,
                e
            );
        }
    }
    Ok(())
}

/// Dispatch pending notifications for one league. Split out so tests can
/// drive a single league with a recording notifier.
pub(crate) async fn dispatch_pending_for_league(
    repos: &Repos,
    league: &League,
    notifier: &dyn Notifier,
    now: DateTime<Utc>,
    base_url: &str,
    smtp_config: &Option<mail::SmtpConfig>,
    t: &T,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cfg = repos.leagues.get_config(league.id).await?;

    // 1) Match closing in <24h with at least one league member missing a tip.
    let closing = repos
        .notifications
        .list_closing_soon_unnotified(league.id)
        .await?;

    for r in &closing {
        if cfg.predict_knockout_only && r.stage == crate::stage::Stage::Group {
            continue;
        }
        let names = repos
            .notifications
            .users_missing_prediction_for(league.id, r.match_id)
            .await?;

        if names.is_empty() {
            continue;
        }

        let event = NotificationEvent::MatchClosingSoon {
            match_id: r.match_id,
            home: r.home.clone(),
            away: r.away.clone(),
            stage: r.stage,
            group_letter: r.group_letter.clone(),
            lock_at: r.kickoff_time,
            missing_names: names,
        };
        let _ = repos
            .notifications
            .try_send(
                notifier,
                league.id,
                "match_closing_soon",
                r.match_id,
                None,
                event,
            )
            .await?;
    }

    // 2) Champion-tip lock approaching — anchored on the first match (or first KO match for KO-only leagues).
    let first_match_lock = if cfg.predict_knockout_only {
        repos.matches.first_knockout_kickoff().await?
    } else {
        repos.matches.first_kickoff().await?
    };
    if let Some(lock_at) = first_match_lock {
        if lock_at > now && lock_at <= now + chrono::Duration::hours(24) {
            let already = repos
                .notifications
                .already_sent(league.id, "special_lock_soon", 0, None)
                .await?;

            if !already {
                let names = repos
                    .notifications
                    .users_missing_champion(league.id)
                    .await?;

                if !names.is_empty() {
                    let event = NotificationEvent::SpecialPredictionsLock {
                        lock_at,
                        missing_names: names,
                    };
                    let _ = repos
                        .notifications
                        .try_send(notifier, league.id, "special_lock_soon", 0, None, event)
                        .await?;
                }
            }
        }
    }

    // 3) Individual email reminders — only if SMTP is configured.
    if let Some(ref smtp) = smtp_config {
        for r in &closing {
            if cfg.predict_knockout_only && r.stage == crate::stage::Stage::Group {
                continue;
            }
            let users = repos
                .users
                .users_missing_prediction_with_email(league.id, r.match_id)
                .await?;
            for (user_id, name, email, token) in users {
                let already = repos
                    .notifications
                    .already_sent(
                        league.id,
                        "email_match_closing_soon",
                        r.match_id,
                        Some(user_id),
                    )
                    .await?;
                if already {
                    continue;
                }
                let link = format!("{}/play/me/{}", base_url.trim_end_matches('/'), token);
                let stage_label = t.get(r.stage.ftl_key());
                if let Err(e) = mail::send_reminder_email(
                    smtp,
                    &name,
                    &email,
                    &r.home,
                    &r.away,
                    &stage_label,
                    &link,
                    t,
                )
                .await
                {
                    tracing::warn!("Email reminder to {} failed: {:?}", email, e);
                    continue;
                }
                // Record success in a best-effort way; use a no-op event since
                // the email itself is the notification.
                let _ = repos
                    .notifications
                    .try_send(
                        &NoopNotifier,
                        league.id,
                        "email_match_closing_soon",
                        r.match_id,
                        Some(user_id),
                        NotificationEvent::MatchClosingSoon {
                            match_id: r.match_id,
                            home: r.home.clone(),
                            away: r.away.clone(),
                            stage: r.stage,
                            group_letter: r.group_letter.clone(),
                            lock_at: r.kickoff_time,
                            missing_names: vec![name.clone()],
                        },
                    )
                    .await?;
            }
        }

        // Champion email reminders
        if let Some(lock_at) = first_match_lock {
            if lock_at > now && lock_at <= now + chrono::Duration::hours(24) {
                let users = repos
                    .users
                    .users_missing_champion_with_email(league.id)
                    .await?;
                for (user_id, name, email, token) in users {
                    let already = repos
                        .notifications
                        .already_sent(league.id, "email_special_lock_soon", 0, Some(user_id))
                        .await?;
                    if already {
                        continue;
                    }
                    let link = format!("{}/play/me/{}", base_url.trim_end_matches('/'), token);
                    if let Err(e) =
                        mail::send_champion_reminder_email(smtp, &name, &email, &link, t).await
                    {
                        tracing::warn!("Champion email reminder to {} failed: {:?}", email, e);
                        continue;
                    }
                    let _ = repos
                        .notifications
                        .try_send(
                            &NoopNotifier,
                            league.id,
                            "email_special_lock_soon",
                            0,
                            Some(user_id),
                            NotificationEvent::SpecialPredictionsLock {
                                lock_at,
                                missing_names: vec![name.clone()],
                            },
                        )
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
    use crate::notifier::NotifierError;
    use crate::repo::fixture::{FakeMatch, MemoryMatchRepo};
    use crate::repo::league::{League as LeagueRow, MemoryLeagueRepo};
    use crate::repo::notification::{ClosingSoonMatch, NotificationRepo};
    use crate::repo::{
        MemoryBootstrapRepo, MemoryNotificationRepo, MemoryPredictionRepo, MemorySettingsRepo,
        MemorySpecialPredictionRepo, MemoryTeamRepo, MemoryUserRepo, Repos, DEFAULT_LEAGUE_ID,
    };
    use crate::scoreboard::{FakeScoreboardClient, MatchStatus, SportsEvent, SportsTeam};
    use crate::stage::Stage;
    use chrono::TimeZone;
    use std::sync::Arc;
    use std::sync::Mutex;

    #[test]
    fn tournament_window_default() {
        let (start, end) = tournament_window();
        assert!(start <= end);
    }

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
        let leagues = Arc::new(MemoryLeagueRepo::new());
        leagues.seed(LeagueRow {
            id: DEFAULT_LEAGUE_ID,
            name: "Default".into(),
            notifications_bootstrapped: false,
        });
        Repos {
            bootstrap: Arc::new(MemoryBootstrapRepo::new()),
            users: Arc::new(MemoryUserRepo::new()),
            leagues,
            matches: Arc::new(MemoryMatchRepo::new()),
            predictions: Arc::new(MemoryPredictionRepo::new()),
            special_predictions: Arc::new(MemorySpecialPredictionRepo::new()),
            teams: Arc::new(MemoryTeamRepo::new()),
            settings: Arc::new(MemorySettingsRepo::new()),
            notifications,
        }
    }

    fn default_league() -> LeagueRow {
        LeagueRow {
            id: DEFAULT_LEAGUE_ID,
            name: "Default".into(),
            notifications_bootstrapped: false,
        }
    }

    fn group_event(id: i64, kickoff: DateTime<Utc>) -> SportsEvent {
        SportsEvent {
            provider_event_id: id,
            stage: Stage::Group,
            group_letter: Some("A".into()),
            home_team: Some(SportsTeam {
                provider_team_id: 100,
                display_name: "Argentina".into(),
                short_name: Some("ARG".into()),
                flag_code: Some("ar".into()),
            }),
            away_team: Some(SportsTeam {
                provider_team_id: 200,
                display_name: "Brazil".into(),
                short_name: Some("BRA".into()),
                flag_code: Some("br".into()),
            }),
            score_home: None,
            score_away: None,
            kickoff: Some(kickoff),
            status: MatchStatus::Scheduled,
        }
    }

    // ─── update_data ────────────────────────────────────────────────────────

    /// `update_data` walks every date in the configured window. Override
    /// the env vars to a single-day window so the test runs in O(1) calls.
    fn single_day_window(date: NaiveDate) {
        std::env::set_var("WC_WINDOW_START", date.format("%Y-%m-%d").to_string());
        std::env::set_var("WC_WINDOW_END", date.format("%Y-%m-%d").to_string());
    }

    #[tokio::test]
    async fn update_data_upserts_every_event_returned_by_provider() {
        let date = NaiveDate::from_ymd_opt(2026, 6, 11).unwrap();
        single_day_window(date);

        let scoreboard = FakeScoreboardClient::new();
        let kickoff = Utc.with_ymd_and_hms(2026, 6, 11, 18, 0, 0).unwrap();
        scoreboard.seed(date, vec![group_event(1, kickoff), group_event(2, kickoff)]);

        let notifications = Arc::new(MemoryNotificationRepo::new());
        let repos = build_repos(notifications);

        update_data(&scoreboard, &repos).await.unwrap();

        // Two team upserts per event × two events = four teams (with overlap
        // collapsed by upsert id).
        let teams = repos.teams.list_real_for_dropdown().await.unwrap();
        let names: Vec<&str> = teams.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"Argentina"));
        assert!(names.contains(&"Brazil"));
    }

    #[tokio::test]
    async fn update_data_logs_and_continues_on_provider_failure() {
        let date = NaiveDate::from_ymd_opt(2026, 6, 11).unwrap();
        single_day_window(date);

        let scoreboard = FakeScoreboardClient::new();
        scoreboard.fail_once(date);
        // Even after the failed call, no events to seed → result is empty.

        let notifications = Arc::new(MemoryNotificationRepo::new());
        let repos = build_repos(notifications);

        // Crucially: this must NOT propagate the error — the worker is
        // designed to log and move on so a single bad day doesn't kill the
        // background task.
        update_data(&scoreboard, &repos).await.unwrap();
    }

    #[tokio::test]
    async fn update_data_handles_empty_window_dates() {
        let date = NaiveDate::from_ymd_opt(2026, 6, 11).unwrap();
        single_day_window(date);

        let scoreboard = FakeScoreboardClient::new();
        // No seed → empty events for that day.

        let notifications = Arc::new(MemoryNotificationRepo::new());
        let repos = build_repos(notifications);

        update_data(&scoreboard, &repos).await.unwrap();
        assert_eq!(scoreboard.call_count(), 1, "single-day window = 1 call");
    }

    // ─── bootstrap + process_notifications ─────────────────────────────────

    #[tokio::test]
    async fn bootstrap_silences_each_league_once() {
        let notifications = Arc::new(MemoryNotificationRepo::new());
        notifications.seed_match_with_both_teams(1);
        notifications.seed_match_with_both_teams(2);

        let repos = build_repos(notifications.clone());

        bootstrap_notifications(&repos).await.unwrap();
        assert_eq!(notifications.sent_count(), 2);

        // Second call must be a no-op for the (now bootstrapped) Default league.
        bootstrap_notifications(&repos).await.unwrap();
        assert_eq!(notifications.sent_count(), 2);
    }

    #[tokio::test]
    async fn process_notifications_dispatches_match_closing_soon() {
        let notifications = Arc::new(MemoryNotificationRepo::new());
        notifications.seed_user(DEFAULT_LEAGUE_ID, "Anna");
        notifications.seed_user(DEFAULT_LEAGUE_ID, "Ben");
        notifications.seed_prediction(DEFAULT_LEAGUE_ID, "Ben", 7);
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
        dispatch_pending_for_league(
            &repos,
            &default_league(),
            &notifier,
            Utc::now(),
            "https://test.example",
            &None,
            &T::default(),
        )
        .await
        .unwrap();

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
        notifications.seed_user(DEFAULT_LEAGUE_ID, "Anna");
        notifications.seed_prediction(DEFAULT_LEAGUE_ID, "Anna", 7);
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
        dispatch_pending_for_league(
            &repos,
            &default_league(),
            &notifier,
            Utc::now(),
            "https://test.example",
            &None,
            &T::default(),
        )
        .await
        .unwrap();
        assert!(
            notifier.sent.lock().unwrap().is_empty(),
            "no missing tippers → no notification"
        );
        assert_eq!(notifications.sent_count(), 0);
    }

    #[tokio::test]
    async fn process_notifications_does_not_repeat_sent_match_closing_soon() {
        let notifications = Arc::new(MemoryNotificationRepo::new());
        notifications.seed_user(DEFAULT_LEAGUE_ID, "Anna");
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
        dispatch_pending_for_league(
            &repos,
            &default_league(),
            &notifier,
            Utc::now(),
            "https://test.example",
            &None,
            &T::default(),
        )
        .await
        .unwrap();
        assert_eq!(notifier.sent.lock().unwrap().len(), 1);

        // Second tick: must be a no-op for the same (league, match_id).
        dispatch_pending_for_league(
            &repos,
            &default_league(),
            &notifier,
            Utc::now(),
            "https://test.example",
            &None,
            &T::default(),
        )
        .await
        .unwrap();
        assert_eq!(notifier.sent.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn process_notifications_dispatches_special_lock_when_first_kickoff_is_soon() {
        let notifications = Arc::new(MemoryNotificationRepo::new());
        notifications.seed_user(DEFAULT_LEAGUE_ID, "Anna");
        notifications.seed_user_with_champion(DEFAULT_LEAGUE_ID, "Ben");

        let matches = Arc::new(MemoryMatchRepo::new());
        matches.seed(FakeMatch::locked_unfinished(
            1,
            Utc::now() + chrono::Duration::hours(6),
        ));
        let mut repos = build_repos(notifications.clone());
        repos.matches = matches;

        let notifier = RecordingNotifier::default();
        dispatch_pending_for_league(
            &repos,
            &default_league(),
            &notifier,
            Utc::now(),
            "https://test.example",
            &None,
            &T::default(),
        )
        .await
        .unwrap();

        {
            let sent = notifier.sent.lock().unwrap();
            assert_eq!(sent.len(), 1);
            match &sent[0] {
                NotificationEvent::SpecialPredictionsLock { missing_names, .. } => {
                    assert_eq!(missing_names, &vec!["Anna".to_string()]);
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
        assert!(notifications
            .already_sent(DEFAULT_LEAGUE_ID, "special_lock_soon", 0, None)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn process_notifications_skips_special_lock_when_kickoff_is_far_away() {
        let notifications = Arc::new(MemoryNotificationRepo::new());
        notifications.seed_user(DEFAULT_LEAGUE_ID, "Anna");

        let matches = Arc::new(MemoryMatchRepo::new());
        matches.seed(FakeMatch::locked_unfinished(
            1,
            Utc::now() + chrono::Duration::hours(48),
        ));
        let mut repos = build_repos(notifications.clone());
        repos.matches = matches;

        let notifier = RecordingNotifier::default();
        dispatch_pending_for_league(
            &repos,
            &default_league(),
            &notifier,
            Utc::now(),
            "https://test.example",
            &None,
            &T::default(),
        )
        .await
        .unwrap();
        assert!(notifier.sent.lock().unwrap().is_empty());
        assert!(!notifications
            .already_sent(DEFAULT_LEAGUE_ID, "special_lock_soon", 0, None)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn process_notifications_skips_special_lock_when_no_users_pending() {
        let notifications = Arc::new(MemoryNotificationRepo::new());
        notifications.seed_user_with_champion(DEFAULT_LEAGUE_ID, "Anna");
        notifications.seed_user_with_champion(DEFAULT_LEAGUE_ID, "Ben");

        let matches = Arc::new(MemoryMatchRepo::new());
        matches.seed(FakeMatch::locked_unfinished(
            1,
            Utc::now() + chrono::Duration::hours(6),
        ));
        let mut repos = build_repos(notifications.clone());
        repos.matches = matches;

        let notifier = RecordingNotifier::default();
        dispatch_pending_for_league(
            &repos,
            &default_league(),
            &notifier,
            Utc::now(),
            "https://test.example",
            &None,
            &T::default(),
        )
        .await
        .unwrap();
        assert!(notifier.sent.lock().unwrap().is_empty());
        assert!(!notifications
            .already_sent(DEFAULT_LEAGUE_ID, "special_lock_soon", 0, None)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn process_notifications_skips_group_stage_for_ko_only_league() {
        use crate::repo::league::{LeagueConfig, LeagueRepo, MemoryLeagueRepo};

        let notifications = Arc::new(MemoryNotificationRepo::new());
        notifications.seed_user(DEFAULT_LEAGUE_ID, "Anna");
        notifications.seed_closing_soon(ClosingSoonMatch {
            match_id: 7,
            stage: Stage::Group,
            group_letter: Some("A".into()),
            kickoff_time: Utc::now() + chrono::Duration::hours(2),
            home: "X".into(),
            away: "Y".into(),
        });

        let mut repos = build_repos(notifications.clone());
        let leagues = Arc::new(MemoryLeagueRepo::new());
        leagues.seed(default_league());
        leagues
            .set_setting(DEFAULT_LEAGUE_ID, LeagueConfig::KEY_KO_ONLY, Some("true"))
            .await
            .unwrap();
        repos.leagues = leagues;

        let notifier = RecordingNotifier::default();
        dispatch_pending_for_league(
            &repos,
            &default_league(),
            &notifier,
            Utc::now(),
            "https://test.example",
            &None,
            &T::default(),
        )
        .await
        .unwrap();
        assert!(
            notifier.sent.lock().unwrap().is_empty(),
            "group-stage match should be skipped for KO-only league"
        );
    }
}
