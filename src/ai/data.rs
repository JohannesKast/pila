// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Pure assembly of the structured input the matchday-recap prompt consumes.
//!
//! Everything here is side-effect free: the worker gathers rows from the repos
//! and hands them in, this module turns them into the serializable
//! [`ReportInput`]. That keeps the (interesting) aggregation logic unit-testable
//! without a database or a live model.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use serde::Serialize;
use uuid::Uuid;

use crate::badges::{
    self, Badge, BadgeContextOwned, BadgeDisplay, BadgeValue, ChampionView, PredictionRow,
};
use crate::repo::fixture::MatchSummary;
use crate::scoring::{self, MatchScoringSystem};
use crate::stage::Stage;
use crate::translations::T;

// ─── Serializable prompt input ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ReportInput {
    pub matchday_date: String,
    pub scoring_system: &'static str,
    pub matches: Vec<MatchInput>,
    pub players: Vec<PlayerInput>,
    pub leader: Option<String>,
    pub biggest_climber: Option<String>,
    pub biggest_faller: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchInput {
    pub home: String,
    pub away: String,
    pub score_home: i32,
    pub score_away: i32,
    pub stage: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlayerInput {
    pub name: String,
    pub total_points: i32,
    pub rank: i32,
    pub rank_delta: i32,
    pub matchday_points: i32,
    pub tendency_pct: Option<i32>,
    pub discipline_pct: Option<i32>,
    pub current_streak: i32,
    pub badges: Vec<BadgeInput>,
    pub champion_pick: Option<String>,
    pub matchday_tips: Vec<TipInput>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BadgeInput {
    pub name: String,
    pub count: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TipInput {
    pub home: String,
    pub away: String,
    pub predicted: String,
    pub points: i32,
}

// ─── Inputs gathered by the worker ─────────────────────────────────────────────

/// Everything the worker reads from the repos for one league's matchday recap.
pub struct ReportSource<'a> {
    pub matchday_date: NaiveDate,
    pub tz: Tz,
    pub scoring_system: MatchScoringSystem,
    /// All matches (any stage/status) as summaries — used to map matches to days.
    pub summaries: &'a [MatchSummary],
    /// Every finished prediction in the league.
    pub finished: Vec<PredictionRow>,
    /// `(user_id, display_name)` for every league member.
    pub users: Vec<(Uuid, String)>,
    /// `(user_id, champion_team_id)` champion picks.
    pub special_picks: Vec<(Uuid, i32)>,
    /// `team_id -> team_name` for resolving champion picks.
    pub team_names: HashMap<i32, String>,
    /// Team that actually won the cup, if the final is decided.
    pub actual_champion: Option<i32>,
    /// Denominator of the discipline metric: started matches with both teams.
    pub started_total: i32,
    /// `user_id -> tips placed on started matches` (discipline numerator).
    pub started_by_user: HashMap<Uuid, i32>,
    /// English translation bundle, used for badge display names in the prompt.
    pub badge_t: &'a T,
    /// Clock used for the (metric-irrelevant) badge context fields.
    pub now: DateTime<Utc>,
}

// ─── Matchday detection ────────────────────────────────────────────────────────

/// The most recent matchday (a calendar day in `tz`) whose every relevant match
/// has finished, or `None` if no such day exists. KO-only leagues only consider
/// knockout matches. Matches without a kickoff or with a TBD team are ignored.
///
/// Only the *latest* such day is returned, never the whole history — that is how
/// the feature avoids backfilling matchdays that finished before it existed.
pub fn latest_finished_matchday(
    summaries: &[MatchSummary],
    tz: Tz,
    knockout_only: bool,
) -> Option<NaiveDate> {
    // day -> (all_finished, any_match)
    let mut by_day: HashMap<NaiveDate, (bool, bool)> = HashMap::new();
    for m in summaries {
        let Some(kt) = m.kickoff_time else { continue };
        if m.home_name.is_none() || m.away_name.is_none() {
            continue;
        }
        if knockout_only && !m.stage.is_knockout() {
            continue;
        }
        let day = kt.with_timezone(&tz).date_naive();
        let entry = by_day.entry(day).or_insert((true, false));
        entry.1 = true;
        if m.status != "finished" {
            entry.0 = false;
        }
    }
    by_day
        .into_iter()
        .filter(|(_, (all_finished, any))| *all_finished && *any)
        .map(|(day, _)| day)
        .max()
}

// ─── Assembly ──────────────────────────────────────────────────────────────────

/// Build the structured prompt input for one league's matchday.
pub fn build_report_input(src: &ReportSource<'_>) -> ReportInput {
    let day = src.matchday_date;

    // match_id -> day, and match_id -> (home, away) for matches with both teams.
    let mut match_day: HashMap<i32, NaiveDate> = HashMap::new();
    let mut match_teams: HashMap<i32, (String, String)> = HashMap::new();
    for m in src.summaries {
        if let Some(kt) = m.kickoff_time {
            match_day.insert(m.id, kt.with_timezone(&src.tz).date_naive());
        }
        if let (Some(h), Some(a)) = (&m.home_name, &m.away_name) {
            match_teams.insert(m.id, (h.clone(), a.clone()));
        }
    }
    let matchday_ids: HashSet<i32> = match_day
        .iter()
        .filter(|(_, d)| **d == day)
        .map(|(id, _)| *id)
        .collect();

    // Finished fixtures of the matchday, chronological.
    let mut matches: Vec<(&MatchSummary, MatchInput)> = src
        .summaries
        .iter()
        .filter(|m| matchday_ids.contains(&m.id) && m.status == "finished")
        .filter_map(|m| {
            let (h, a) = match_teams.get(&m.id)?;
            let (sh, sa) = (m.score_home?, m.score_away?);
            Some((
                m,
                MatchInput {
                    home: h.clone(),
                    away: a.clone(),
                    score_home: sh,
                    score_away: sa,
                    stage: stage_label(m.stage),
                },
            ))
        })
        .collect();
    matches.sort_by_key(|(m, _)| m.kickoff_time);
    let matches: Vec<MatchInput> = matches.into_iter().map(|(_, mi)| mi).collect();

    let all_user_ids: Vec<Uuid> = src.users.iter().map(|(id, _)| *id).collect();

    // Standings now vs. before this matchday — to derive rank movement.
    let final_day = final_match_day(src.summaries, &match_day);
    let champ_now = champion_bonus(&src.special_picks, src.actual_champion);
    let champ_prev = if final_day.is_some_and(|d| d < day) {
        champ_now.clone()
    } else {
        HashMap::new()
    };
    let totals_now = totals(&all_user_ids, src.finished.iter(), &champ_now);
    let totals_prev = totals(
        &all_user_ids,
        src.finished
            .iter()
            .filter(|r| match_day.get(&r.match_id).is_some_and(|d| *d < day)),
        &champ_prev,
    );

    // Shared badge context (only user_id / user_started_tips / champion vary).
    let owned = BadgeContextOwned {
        user_id: Uuid::nil(),
        now: src.now,
        berlin_today: src.now.date_naive(),
        finished_predictions: src.finished.clone(),
        all_user_ids: all_user_ids.clone(),
        started_matches_total: src.started_total,
        user_started_tips: 0,
        actual_champion_id: src.actual_champion,
        all_special_picks: src.special_picks.clone(),
        user_champion: None,
    };

    let pick_by_user: HashMap<Uuid, i32> = src.special_picks.iter().copied().collect();

    let mut players: Vec<PlayerInput> = src
        .users
        .iter()
        .map(|(uid, name)| {
            let mut ctx = owned.as_ctx();
            ctx.user_id = *uid;
            ctx.user_started_tips = src.started_by_user.get(uid).copied().unwrap_or(0);
            let champion_view = pick_by_user.get(uid).and_then(|tid| {
                src.team_names.get(tid).map(|n| ChampionView {
                    team_name: n.clone(),
                    flag_url: String::new(),
                })
            });
            ctx.user_champion = champion_view.as_ref();

            let tendency_pct = percent(badges::TendencyPctBadge.compute(&ctx));
            let discipline_pct = percent(badges::DisciplinePctBadge.compute(&ctx));
            let current_streak = streak(badges::CurrentStreakBadge.compute(&ctx));
            let badges_list = badges::achievement_badges_for(&ctx, src.badge_t)
                .into_iter()
                .map(|b| BadgeInput {
                    name: b.title,
                    count: b.count,
                })
                .collect();

            let matchday_points = src
                .finished
                .iter()
                .filter(|r| r.user_id == *uid && matchday_ids.contains(&r.match_id))
                .map(|r| r.base_points())
                .sum();

            let mut matchday_tips: Vec<TipInput> = src
                .finished
                .iter()
                .filter(|r| r.user_id == *uid && matchday_ids.contains(&r.match_id))
                .filter_map(|r| {
                    let (h, a) = match_teams.get(&r.match_id)?;
                    Some(TipInput {
                        home: h.clone(),
                        away: a.clone(),
                        predicted: prediction_label(src.scoring_system, h, a, r.pred_h, r.pred_a),
                        points: r.base_points(),
                    })
                })
                .collect();
            matchday_tips.sort_by(|x, y| x.home.cmp(&y.home));

            let rank = rank_of(*uid, &totals_now).unwrap_or(0);
            let rank_delta = match (rank_of(*uid, &totals_prev), rank_of(*uid, &totals_now)) {
                (Some(p), Some(c)) => p - c,
                _ => 0,
            };

            PlayerInput {
                name: name.clone(),
                total_points: *totals_now.get(uid).unwrap_or(&0),
                rank,
                rank_delta,
                matchday_points,
                tendency_pct,
                discipline_pct,
                current_streak,
                badges: badges_list,
                champion_pick: champion_view.map(|c| c.team_name),
                matchday_tips,
            }
        })
        .collect();

    players.sort_by(|a, b| a.rank.cmp(&b.rank).then(a.name.cmp(&b.name)));

    let leader = players.iter().find(|p| p.rank == 1).map(|p| p.name.clone());
    let biggest_climber = players
        .iter()
        .filter(|p| p.rank_delta > 0)
        .max_by(|a, b| a.rank_delta.cmp(&b.rank_delta).then(b.name.cmp(&a.name)))
        .map(|p| p.name.clone());
    let biggest_faller = players
        .iter()
        .filter(|p| p.rank_delta < 0)
        .min_by(|a, b| a.rank_delta.cmp(&b.rank_delta).then(a.name.cmp(&b.name)))
        .map(|p| p.name.clone());

    ReportInput {
        matchday_date: day.to_string(),
        scoring_system: if src.scoring_system.is_winner_only() {
            "winner_only"
        } else {
            "exact_score"
        },
        matches,
        players,
        leader,
        biggest_climber,
        biggest_faller,
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────────────

fn stage_label(stage: Stage) -> &'static str {
    match stage {
        Stage::Group => "Group stage",
        Stage::RoundOf32 => "Round of 32",
        Stage::RoundOf16 => "Round of 16",
        Stage::QuarterFinal => "Quarter-final",
        Stage::SemiFinal => "Semi-final",
        Stage::ThirdPlace => "Third-place play-off",
        Stage::Final => "Final",
    }
}

fn prediction_label(
    system: MatchScoringSystem,
    home: &str,
    away: &str,
    pred_h: i32,
    pred_a: i32,
) -> String {
    if system.is_winner_only() {
        match pred_h.cmp(&pred_a) {
            std::cmp::Ordering::Greater => format!("{home} to win"),
            std::cmp::Ordering::Less => format!("{away} to win"),
            std::cmp::Ordering::Equal => "Draw".to_string(),
        }
    } else {
        format!("{pred_h}:{pred_a}")
    }
}

fn percent(d: BadgeDisplay) -> Option<i32> {
    match d {
        BadgeDisplay::Metric(BadgeValue::Percent(p)) => p,
        _ => None,
    }
}

fn streak(d: BadgeDisplay) -> i32 {
    match d {
        BadgeDisplay::Metric(BadgeValue::Streak(n)) => n,
        _ => 0,
    }
}

/// Champion-bonus points per user, only if the cup has actually been won.
fn champion_bonus(special: &[(Uuid, i32)], actual: Option<i32>) -> HashMap<Uuid, i32> {
    let mut m = HashMap::new();
    if let Some(a) = actual {
        for (uid, pick) in special {
            if *pick == a {
                *m.entry(*uid).or_insert(0) += scoring::champion_points(Some(*pick), Some(a));
            }
        }
    }
    m
}

fn final_match_day(
    summaries: &[MatchSummary],
    match_day: &HashMap<i32, NaiveDate>,
) -> Option<NaiveDate> {
    summaries
        .iter()
        .filter(|m| m.stage == Stage::Final && m.status == "finished")
        .filter_map(|m| match_day.get(&m.id).copied())
        .max()
}

fn totals<'a>(
    all_user_ids: &[Uuid],
    rows: impl Iterator<Item = &'a PredictionRow>,
    champion: &HashMap<Uuid, i32>,
) -> HashMap<Uuid, i32> {
    let mut totals: HashMap<Uuid, i32> = all_user_ids.iter().map(|u| (*u, 0)).collect();
    for r in rows {
        *totals.entry(r.user_id).or_insert(0) += r.base_points();
    }
    for (uid, bonus) in champion {
        *totals.entry(*uid).or_insert(0) += bonus;
    }
    totals
}

/// 1-based standard competition rank of `user` in `totals`: tied players share
/// a rank and the next rank skips. Shares [`crate::ranking::competition_rank`]
/// with the leaderboard so the recap and the table agree.
fn rank_of(user: Uuid, totals: &HashMap<Uuid, i32>) -> Option<i32> {
    let user_pts = *totals.get(&user)?;
    let all_points: Vec<i32> = totals.values().copied().collect();
    Some(crate::ranking::competition_rank(user_pts, &all_points))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn uid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    fn ny() -> Tz {
        "America/New_York".parse().unwrap()
    }

    fn summary(
        id: i32,
        day: u32,
        hour: u32,
        status: &str,
        score: Option<(i32, i32)>,
    ) -> MatchSummary {
        MatchSummary {
            id,
            stage: Stage::Group,
            kickoff_time: Some(Utc.with_ymd_and_hms(2026, 6, day, hour, 0, 0).unwrap()),
            status: status.into(),
            score_home: score.map(|s| s.0),
            score_away: score.map(|s| s.1),
            home_name: Some(format!("Home{id}")),
            away_name: Some(format!("Away{id}")),
        }
    }

    fn pred(
        user: Uuid,
        match_id: i32,
        day: u32,
        score: (i32, i32),
        p: (i32, i32),
    ) -> PredictionRow {
        PredictionRow {
            user_id: user,
            match_id,
            stage: Stage::Group,
            kickoff: Utc.with_ymd_and_hms(2026, 6, day, 18, 0, 0).unwrap(),
            score_h: score.0,
            score_a: score.1,
            pred_h: p.0,
            pred_a: p.1,
            scoring_system: MatchScoringSystem::ExactScore,
        }
    }

    #[test]
    fn latest_finished_matchday_picks_newest_fully_finished_day() {
        // 18:00 UTC on June 11 is still June 11 in New York (14:00 EDT).
        let summaries = vec![
            summary(1, 11, 18, "finished", Some((1, 0))),
            summary(2, 12, 18, "finished", Some((2, 2))),
            // June 13 not all finished → excluded.
            summary(3, 13, 18, "finished", Some((0, 0))),
            summary(4, 13, 20, "in_progress", None),
        ];
        let day = latest_finished_matchday(&summaries, ny(), false).unwrap();
        assert_eq!(day, NaiveDate::from_ymd_opt(2026, 6, 12).unwrap());
    }

    #[test]
    fn latest_finished_matchday_respects_us_timezone_rollover() {
        // 02:00 UTC June 12 = 22:00 EDT June 11 → belongs to June 11 in NY.
        let m = MatchSummary {
            kickoff_time: Some(Utc.with_ymd_and_hms(2026, 6, 12, 2, 0, 0).unwrap()),
            ..summary(1, 11, 18, "finished", Some((1, 0)))
        };
        let day = latest_finished_matchday(&[m], ny(), false).unwrap();
        assert_eq!(day, NaiveDate::from_ymd_opt(2026, 6, 11).unwrap());
    }

    #[test]
    fn latest_finished_matchday_knockout_only_ignores_group_days() {
        let mut ko = summary(2, 20, 18, "finished", Some((1, 0)));
        ko.stage = Stage::RoundOf16;
        let summaries = vec![summary(1, 11, 18, "finished", Some((1, 0))), ko];
        let day = latest_finished_matchday(&summaries, ny(), true).unwrap();
        assert_eq!(day, NaiveDate::from_ymd_opt(2026, 6, 20).unwrap());
    }

    #[test]
    fn build_report_input_computes_points_rank_and_movement() {
        let a = uid(1);
        let b = uid(2);
        let t = crate::translations::load_all().remove("en").unwrap();
        // Day 1 (June 11): A nails 1:0 (4 pts), B gets tendency (group = 2 pts).
        // Day 2 (June 12, the matchday): A misses (0), B nails 2:1 (4 pts).
        // Cumulative: A=4, B=6 → B leads. Before day 2: A=4, B=2 → A led.
        let summaries = vec![
            summary(1, 11, 18, "finished", Some((1, 0))),
            summary(2, 12, 18, "finished", Some((2, 1))),
        ];
        let finished = vec![
            pred(a, 1, 11, (1, 0), (1, 0)),
            pred(b, 1, 11, (1, 0), (2, 0)),
            pred(a, 2, 12, (2, 1), (0, 0)),
            pred(b, 2, 12, (2, 1), (2, 1)),
        ];
        let src = ReportSource {
            matchday_date: NaiveDate::from_ymd_opt(2026, 6, 12).unwrap(),
            tz: ny(),
            scoring_system: MatchScoringSystem::ExactScore,
            summaries: &summaries,
            finished,
            users: vec![(a, "Anna".into()), (b, "Ben".into())],
            special_picks: vec![],
            team_names: HashMap::new(),
            actual_champion: None,
            started_total: 2,
            started_by_user: HashMap::from([(a, 2), (b, 2)]),
            badge_t: &t,
            now: Utc.with_ymd_and_hms(2026, 6, 12, 22, 0, 0).unwrap(),
        };
        let input = build_report_input(&src);

        assert_eq!(input.matchday_date, "2026-06-12");
        assert_eq!(input.matches.len(), 1);
        assert_eq!(input.matches[0].score_home, 2);

        let ben = input.players.iter().find(|p| p.name == "Ben").unwrap();
        let anna = input.players.iter().find(|p| p.name == "Anna").unwrap();
        assert_eq!(ben.total_points, 6);
        assert_eq!(anna.total_points, 4);
        assert_eq!(ben.rank, 1);
        assert_eq!(anna.rank, 2);
        // Ben climbed from rank 2 to rank 1 on this matchday.
        assert_eq!(ben.rank_delta, 1);
        assert_eq!(anna.rank_delta, -1);
        assert_eq!(ben.matchday_points, 4);
        assert_eq!(anna.matchday_points, 0);
        assert_eq!(input.leader.as_deref(), Some("Ben"));
        assert_eq!(input.biggest_climber.as_deref(), Some("Ben"));
        assert_eq!(input.biggest_faller.as_deref(), Some("Anna"));
        // Ben's matchday tip is recorded with its exact-score label and points.
        assert_eq!(ben.matchday_tips.len(), 1);
        assert_eq!(ben.matchday_tips[0].predicted, "2:1");
        assert_eq!(ben.matchday_tips[0].points, 4);
    }

    #[test]
    fn build_report_input_shares_rank_on_ties() {
        let a = uid(1);
        let b = uid(2);
        let c = uid(3);
        let t = crate::translations::load_all().remove("en").unwrap();
        // Single matchday (June 12), one match finishing 2:1. Anna and Bea both
        // nail it (4 pts each, tied); Cara only gets the tendency (2 pts).
        let summaries = vec![summary(1, 12, 18, "finished", Some((2, 1)))];
        let finished = vec![
            pred(a, 1, 12, (2, 1), (2, 1)),
            pred(b, 1, 12, (2, 1), (2, 1)),
            pred(c, 1, 12, (2, 1), (3, 0)),
        ];
        let src = ReportSource {
            matchday_date: NaiveDate::from_ymd_opt(2026, 6, 12).unwrap(),
            tz: ny(),
            scoring_system: MatchScoringSystem::ExactScore,
            summaries: &summaries,
            finished,
            users: vec![(a, "Anna".into()), (b, "Bea".into()), (c, "Cara".into())],
            special_picks: vec![],
            team_names: HashMap::new(),
            actual_champion: None,
            started_total: 1,
            started_by_user: HashMap::from([(a, 1), (b, 1), (c, 1)]),
            badge_t: &t,
            now: Utc.with_ymd_and_hms(2026, 6, 12, 22, 0, 0).unwrap(),
        };
        let input = build_report_input(&src);

        let anna = input.players.iter().find(|p| p.name == "Anna").unwrap();
        let bea = input.players.iter().find(|p| p.name == "Bea").unwrap();
        let cara = input.players.iter().find(|p| p.name == "Cara").unwrap();

        assert_eq!(anna.total_points, 4);
        assert_eq!(bea.total_points, 4);
        assert_eq!(cara.total_points, 2);
        // Anna and Bea tie on 4 points → both rank 1. Standard competition
        // ranking skips rank 2, so Cara (2 pts) lands on rank 3, not rank 2 —
        // matching what the leaderboard table shows.
        assert_eq!(anna.rank, 1);
        assert_eq!(bea.rank, 1);
        assert_eq!(cara.rank, 3);
    }

    #[test]
    fn build_report_input_resolves_champion_pick_name() {
        let a = uid(1);
        let t = crate::translations::load_all().remove("en").unwrap();
        let summaries = vec![summary(1, 12, 18, "finished", Some((1, 0)))];
        let src = ReportSource {
            matchday_date: NaiveDate::from_ymd_opt(2026, 6, 12).unwrap(),
            tz: ny(),
            scoring_system: MatchScoringSystem::ExactScore,
            summaries: &summaries,
            finished: vec![pred(a, 1, 12, (1, 0), (1, 0))],
            users: vec![(a, "Anna".into())],
            special_picks: vec![(a, 99)],
            team_names: HashMap::from([(99, "Brazil".to_string())]),
            actual_champion: None,
            started_total: 1,
            started_by_user: HashMap::from([(a, 1)]),
            badge_t: &t,
            now: Utc.with_ymd_and_hms(2026, 6, 12, 22, 0, 0).unwrap(),
        };
        let input = build_report_input(&src);
        let anna = input.players.iter().find(|p| p.name == "Anna").unwrap();
        assert_eq!(anna.champion_pick.as_deref(), Some("Brazil"));
    }
}
