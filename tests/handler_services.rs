//! Tests for the cross-handler orchestration functions in
//! `pila::handlers::services` — the building blocks the index, leaderboard,
//! and badge handlers all share.
//!
//! These are not endpoint tests (Pila has no JSON `/services` route); they
//! pin the shape and ordering of the values handlers consume. League-scope
//! isolation is covered in `multi_league_isolation.rs`.

use std::sync::Arc;

use chrono::{Duration, Utc};
use uuid::Uuid;

use pila::handlers::services::{
    build_badge_context, fetch_actual_champion, fetch_group_standings, fetch_leaderboard,
};
use pila::repo::fixture::{FakeMatch, MemoryMatchRepo};
use pila::repo::league::{League, MemoryLeagueRepo};
use pila::repo::prediction::{FakeFinishedRow, FakeLeaderboardRow};
use pila::repo::user::UserFull;
use pila::repo::{
    MemoryBootstrapRepo, MemoryInviteRepo, MemoryNotificationRepo, MemoryPredictionRepo,
    MemorySettingsRepo, MemorySpecialPredictionRepo, MemoryTeamRepo, MemoryUserRepo, Repos,
    DEFAULT_LEAGUE_ID,
};
use pila::stage::Stage;

// ─── Harness ─────────────────────────────────────────────────────────────────

struct Bag {
    repos: Repos,
    users: Arc<MemoryUserRepo>,
    matches: Arc<MemoryMatchRepo>,
    predictions: Arc<MemoryPredictionRepo>,
}

fn build_bag() -> Bag {
    let users = Arc::new(MemoryUserRepo::new());
    let matches = Arc::new(MemoryMatchRepo::new());
    let predictions = Arc::new(MemoryPredictionRepo::new());
    let leagues = Arc::new(MemoryLeagueRepo::new());
    leagues.seed(League {
        id: DEFAULT_LEAGUE_ID,
        name: "Default".into(),
        notifications_bootstrapped: true,
    });

    let repos = Repos {
        bootstrap: Arc::new(MemoryBootstrapRepo::new()),
        users: users.clone(),
        leagues,
        matches: matches.clone(),
        predictions: predictions.clone(),
        special_predictions: Arc::new(MemorySpecialPredictionRepo::new()),
        teams: Arc::new(MemoryTeamRepo::new()),
        settings: Arc::new(MemorySettingsRepo::new()),
        invites: Arc::new(MemoryInviteRepo::new()),
        notifications: Arc::new(MemoryNotificationRepo::new()),
        reports: Arc::new(pila::repo::MemoryMatchdayReportRepo::new()),
    };

    Bag {
        repos,
        users,
        matches,
        predictions,
    }
}

fn jerseys() -> std::collections::HashMap<String, pila::jersey::JerseyPreset> {
    pila::jersey::load().as_ref().clone()
}

fn seed_user(repo: &MemoryUserRepo, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    repo.seed(
        UserFull {
            id,
            name: name.into(),
            real_name: name.into(),
            token: name.to_lowercase(),
            phone_number: None,
            email: None,
            is_admin: false,
            can_create_league: false,
            league_id: DEFAULT_LEAGUE_ID,
            language: "de".into(),
        },
        "classic",
    );
    id
}

// ─── fetch_actual_champion ───────────────────────────────────────────────────

#[tokio::test]
async fn fetch_actual_champion_returns_none_when_final_not_finished() {
    let bag = build_bag();
    assert_eq!(fetch_actual_champion(&bag.repos).await, None);
}

#[tokio::test]
async fn fetch_actual_champion_returns_winner_of_finished_final() {
    let bag = build_bag();
    let mut m = FakeMatch::locked_unfinished(99, Utc::now() - Duration::days(1));
    m.stage = Stage::Final;
    m.status = "finished".into();
    m.score_home = Some(3);
    m.score_away = Some(1);
    m.team_home_id = Some(101);
    m.team_away_id = Some(202);
    bag.matches.seed(m);

    assert_eq!(fetch_actual_champion(&bag.repos).await, Some(101));
}

// ─── fetch_leaderboard ───────────────────────────────────────────────────────

#[tokio::test]
async fn fetch_leaderboard_lists_every_seeded_user_even_without_predictions() {
    let bag = build_bag();
    seed_user(&bag.users, "Alice");
    seed_user(&bag.users, "Bob");

    let lb = fetch_leaderboard(&bag.repos, &jerseys(), DEFAULT_LEAGUE_ID, Utc::now()).await;
    assert_eq!(lb.len(), 2);
    assert!(lb.iter().all(|e| e.total_points == 0));
    assert!(lb.iter().all(|e| e.max_potential_points == 0));
}

#[tokio::test]
async fn fetch_leaderboard_sorts_by_total_points_descending() {
    let bag = build_bag();
    seed_user(&bag.users, "Alice");
    seed_user(&bag.users, "Bob");
    seed_user(&bag.users, "Charlie");

    let now = Utc::now();
    let kickoff = Some(now - Duration::hours(2));

    // Alice: exact 2:1 → 4 points (group).
    bag.predictions.seed_leaderboard(FakeLeaderboardRow {
        league_id: DEFAULT_LEAGUE_ID,
        user_name: "Alice".into(),
        stage: Stage::Group,
        kickoff_time: kickoff,
        status: "finished".into(),
        score_home: Some(2),
        score_away: Some(1),
        predicted_home: 2,
        predicted_away: 1,
    });
    // Bob: correct tendency but wrong score → 2 points.
    bag.predictions.seed_leaderboard(FakeLeaderboardRow {
        league_id: DEFAULT_LEAGUE_ID,
        user_name: "Bob".into(),
        stage: Stage::Group,
        kickoff_time: kickoff,
        status: "finished".into(),
        score_home: Some(2),
        score_away: Some(1),
        predicted_home: 3,
        predicted_away: 0,
    });
    // Charlie: tip with no result yet, contributes only to potential.

    let lb = fetch_leaderboard(&bag.repos, &jerseys(), DEFAULT_LEAGUE_ID, now).await;
    let names: Vec<&str> = lb.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["Alice", "Bob", "Charlie"]);

    let alice = lb.iter().find(|e| e.name == "Alice").unwrap();
    let bob = lb.iter().find(|e| e.name == "Bob").unwrap();
    assert_eq!(alice.total_points, 4);
    assert_eq!(bob.total_points, 2);
}

#[tokio::test]
async fn fetch_leaderboard_unfinished_tip_adds_to_max_potential() {
    let bag = build_bag();
    seed_user(&bag.users, "Alice");

    let now = Utc::now();
    bag.predictions.seed_leaderboard(FakeLeaderboardRow {
        league_id: DEFAULT_LEAGUE_ID,
        user_name: "Alice".into(),
        stage: Stage::Group,
        kickoff_time: Some(now + Duration::hours(2)), // not started yet
        status: "scheduled".into(),
        score_home: None,
        score_away: None,
        predicted_home: 1,
        predicted_away: 0,
    });

    let lb = fetch_leaderboard(&bag.repos, &jerseys(), DEFAULT_LEAGUE_ID, now).await;
    let alice = lb.iter().find(|e| e.name == "Alice").unwrap();
    assert_eq!(alice.total_points, 0);
    // Group-stage exact-score max is 4.
    assert_eq!(alice.max_potential_points, 4);
}

#[tokio::test]
async fn fetch_leaderboard_attaches_user_jersey_preset() {
    let bag = build_bag();
    seed_user(&bag.users, "Alice");

    let presets = jerseys();
    let lb = fetch_leaderboard(&bag.repos, &presets, DEFAULT_LEAGUE_ID, Utc::now()).await;
    assert_eq!(lb.len(), 1);
    let alice = &lb[0];
    let classic = pila::jersey::get(&presets, "classic");
    assert_eq!(alice.jersey_body, classic.body);
    assert_eq!(alice.jersey_accent, classic.accent);
    assert_eq!(alice.jersey_pattern, classic.pattern);
}

// ─── fetch_group_standings ───────────────────────────────────────────────────

fn finished_group_match(
    id: i32,
    letter: &str,
    home: (i32, &str),
    away: (i32, &str),
    score: (i32, i32),
) -> FakeMatch {
    FakeMatch {
        id,
        stage: Stage::Group,
        group_letter: Some(letter.into()),
        kickoff_time: Some(Utc::now() - Duration::days(1)),
        status: "finished".into(),
        score_home: Some(score.0),
        score_away: Some(score.1),
        team_home_id: Some(home.0),
        team_away_id: Some(away.0),
        home_name: home.1.into(),
        away_name: away.1.into(),
        home_flag: None,
        away_flag: None,
    }
}

#[tokio::test]
async fn fetch_group_standings_empty_when_no_finished_matches() {
    let bag = build_bag();
    assert!(fetch_group_standings(&bag.repos).await.is_empty());
}

#[tokio::test]
async fn fetch_group_standings_awards_three_points_for_win_one_for_draw() {
    let bag = build_bag();
    // Group A: TeamA beats TeamB 2:1, TeamA draws TeamC 1:1.
    bag.matches.seed(finished_group_match(
        1,
        "A",
        (1, "TeamA"),
        (2, "TeamB"),
        (2, 1),
    ));
    bag.matches.seed(finished_group_match(
        2,
        "A",
        (1, "TeamA"),
        (3, "TeamC"),
        (1, 1),
    ));

    let tables = fetch_group_standings(&bag.repos).await;
    assert_eq!(tables.len(), 1);
    let group_a = &tables[0];
    assert_eq!(group_a.letter, "A");

    let team_a = group_a
        .rows
        .iter()
        .find(|r| r.team_name == "TeamA")
        .unwrap();
    assert_eq!(team_a.points, 4); // 3 (win) + 1 (draw)
    assert_eq!(team_a.wins, 1);
    assert_eq!(team_a.draws, 1);
    assert_eq!(team_a.losses, 0);
    assert_eq!(team_a.goals_for, 3);
    assert_eq!(team_a.goals_against, 2);
    assert_eq!(team_a.goal_diff, 1);

    let team_b = group_a
        .rows
        .iter()
        .find(|r| r.team_name == "TeamB")
        .unwrap();
    assert_eq!(team_b.points, 0);
    assert_eq!(team_b.losses, 1);

    let team_c = group_a
        .rows
        .iter()
        .find(|r| r.team_name == "TeamC")
        .unwrap();
    assert_eq!(team_c.points, 1);
    assert_eq!(team_c.draws, 1);
}

#[tokio::test]
async fn fetch_group_standings_sorts_within_group_by_points_then_goal_diff() {
    let bag = build_bag();
    // Same points (3 each), but TeamX has +5 goal diff and TeamY has +1.
    bag.matches.seed(finished_group_match(
        1,
        "B",
        (10, "TeamX"),
        (11, "Loser1"),
        (5, 0),
    ));
    bag.matches.seed(finished_group_match(
        2,
        "B",
        (12, "TeamY"),
        (13, "Loser2"),
        (1, 0),
    ));

    let tables = fetch_group_standings(&bag.repos).await;
    let group_b = tables.iter().find(|t| t.letter == "B").unwrap();
    assert_eq!(
        group_b.rows[0].team_name, "TeamX",
        "better diff sorts first"
    );
    assert_eq!(group_b.rows[1].team_name, "TeamY");
}

#[tokio::test]
async fn fetch_group_standings_returns_multiple_groups_keyed_by_letter() {
    let bag = build_bag();
    bag.matches.seed(finished_group_match(
        1,
        "A",
        (1, "TeamA"),
        (2, "TeamB"),
        (1, 0),
    ));
    bag.matches.seed(finished_group_match(
        2,
        "C",
        (3, "TeamC"),
        (4, "TeamD"),
        (2, 2),
    ));

    let tables = fetch_group_standings(&bag.repos).await;
    let letters: Vec<&str> = tables.iter().map(|t| t.letter.as_str()).collect();
    assert_eq!(letters, vec!["A", "C"], "groups returned in letter order");
}

// ─── build_badge_context ─────────────────────────────────────────────────────

#[tokio::test]
async fn build_badge_context_collects_finished_predictions_for_league() {
    let bag = build_bag();
    let alice = seed_user(&bag.users, "Alice");

    let kickoff = Utc::now() - Duration::days(1);
    bag.predictions.seed_finished(FakeFinishedRow {
        user_id: alice,
        league_id: DEFAULT_LEAGUE_ID,
        match_id: 1,
        stage: Stage::Group,
        kickoff,
        score_home: 2,
        score_away: 1,
        predicted_home: 2,
        predicted_away: 1,
    });

    let ctx = build_badge_context(&bag.repos, alice, DEFAULT_LEAGUE_ID, Utc::now()).await;
    assert_eq!(ctx.finished_predictions.len(), 1);
    assert_eq!(ctx.finished_predictions[0].user_id, alice);
    assert!(ctx.all_user_ids.contains(&alice));
}
