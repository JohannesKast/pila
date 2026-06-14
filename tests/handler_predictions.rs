//! Fake-backed handler tests. Constructs `AppState` from in-memory repo
//! impls so the handler logic — score validation, lock window, persistence
//! — runs without a live database.
//!
//! These tests are the strongest argument for the repo abstraction: every
//! branch of the handler is covered without needing a Postgres connection
//! or fixture cleanup.

use std::sync::Arc;

use axum::extract::{Form, Path, State};
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use uuid::Uuid;

use pila::auth::AuthenticatedUser;
use pila::handlers::predictions::{
    predict_match, predict_special, PredictionForm, SpecialPredictionForm,
};
use pila::handlers::util::league_scope_path;
use pila::repo::fixture::{FakeMatch, MemoryMatchRepo};
use pila::repo::league::{League, MemoryLeagueRepo};
use pila::repo::team::TeamOption;
use pila::repo::{
    MemoryBootstrapRepo, MemoryInviteRepo, MemoryNotificationRepo, MemoryPredictionRepo,
    MemorySettingsRepo, MemorySpecialPredictionRepo, MemoryTeamRepo, MemoryUserRepo, Repos,
    DEFAULT_LEAGUE_ID,
};
use pila::stage::Stage;
use pila::AppState;

struct Harness {
    state: AppState,
    matches: Arc<MemoryMatchRepo>,
    predictions: Arc<MemoryPredictionRepo>,
    special_predictions: Arc<MemorySpecialPredictionRepo>,
    teams: Arc<MemoryTeamRepo>,
}

fn build_harness() -> Harness {
    let users = Arc::new(MemoryUserRepo::new());
    let matches = Arc::new(MemoryMatchRepo::new());
    let predictions = Arc::new(MemoryPredictionRepo::new());
    let special_predictions = Arc::new(MemorySpecialPredictionRepo::new());
    let teams = Arc::new(MemoryTeamRepo::new());
    let settings = Arc::new(MemorySettingsRepo::new());
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
        special_predictions: special_predictions.clone(),
        teams: teams.clone(),
        settings,
        invites: Arc::new(MemoryInviteRepo::new()),
        notifications: Arc::new(MemoryNotificationRepo::new()),
    };

    let state = AppState {
        jerseys: pila::jersey::load(),
        news: pila::news::NewsCache::from_env(),
        repos,
        translations: std::collections::HashMap::new(),
        concurrency_limit: Arc::new(tokio::sync::Semaphore::new(100)),
        base_url: "http://localhost:8000".into(),
        signal_api_url: None,
        signal_from_number: None,
        signal_group_id: None,
        http_client: reqwest::Client::new(),
        smtp_config: None,
        mock_now: pila::time::new_mock_time(),
        dev_mode: false,
    };

    Harness {
        state,
        matches,
        predictions,
        special_predictions,
        teams,
    }
}

fn fake_user() -> AuthenticatedUser {
    AuthenticatedUser {
        id: Uuid::new_v4(),
        name: "Tester".into(),
        is_admin: false,
        can_create_league: false,
        phone_number: None,
        email: None,
        jersey_preset: "classic".into(),
        language: "de".into(),
        league_id: DEFAULT_LEAGUE_ID,
    }
}

// ─── predict_match ────────────────────────────────────────────────────────────

#[tokio::test]
async fn predict_match_rejects_score_out_of_range() {
    let h = build_harness();
    h.matches.seed(FakeMatch::locked_unfinished(
        1,
        Utc::now() + Duration::hours(2),
    ));
    let user = fake_user();
    let form = PredictionForm {
        score_home: Some(21),
        score_away: Some(0),
        outcome: String::new(),
    };
    let res = predict_match(State(h.state.clone()), user, Path(1), Form(form)).await;
    let err = res.unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn predict_match_rejects_negative_score() {
    let h = build_harness();
    h.matches.seed(FakeMatch::locked_unfinished(
        1,
        Utc::now() + Duration::hours(2),
    ));
    let user = fake_user();
    let form = PredictionForm {
        score_home: Some(-1),
        score_away: Some(0),
        outcome: String::new(),
    };
    let res = predict_match(State(h.state.clone()), user, Path(1), Form(form)).await;
    assert_eq!(res.unwrap_err().0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn predict_match_rejects_unknown_match() {
    let h = build_harness();
    let user = fake_user();
    let form = PredictionForm {
        score_home: Some(2),
        score_away: Some(1),
        outcome: String::new(),
    };
    let res = predict_match(State(h.state.clone()), user, Path(404), Form(form)).await;
    assert_eq!(res.unwrap_err().0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn predict_match_rejects_when_kickoff_already_passed() {
    let h = build_harness();
    h.matches.seed(FakeMatch::locked_unfinished(
        1,
        Utc::now() - Duration::hours(1),
    ));
    let user = fake_user();
    let form = PredictionForm {
        score_home: Some(2),
        score_away: Some(1),
        outcome: String::new(),
    };
    let res = predict_match(State(h.state.clone()), user, Path(1), Form(form)).await;
    assert_eq!(res.unwrap_err().0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn predict_match_lock_follows_mock_time() {
    // Regression: mock_now must replace real-time for the lock check.
    // If a future Utc::now() leaks back into the handler, this test fails.
    let h = build_harness();
    let kickoff = Utc::now() + Duration::days(30);
    h.matches.seed(FakeMatch::locked_unfinished(1, kickoff));
    let form = || PredictionForm {
        score_home: Some(2),
        score_away: Some(1),
        outcome: String::new(),
    };

    // Mock time AFTER kickoff → must be locked (400)
    pila::time::set_mock_time(&h.state.mock_now, kickoff + Duration::hours(1));
    let res = predict_match(State(h.state.clone()), fake_user(), Path(1), Form(form())).await;
    assert_eq!(
        res.unwrap_err().0,
        StatusCode::BAD_REQUEST,
        "mock time past kickoff must lock the tip"
    );

    // Mock time BEFORE kickoff → must succeed
    pila::time::set_mock_time(&h.state.mock_now, kickoff - Duration::hours(1));
    let res = predict_match(State(h.state.clone()), fake_user(), Path(1), Form(form())).await;
    assert!(
        res.is_ok(),
        "mock time before kickoff must allow the tip, got {:?}",
        res.err().map(|e| e.0)
    );
}

#[tokio::test]
async fn predict_match_rejects_when_team_is_tbd() {
    let h = build_harness();
    let mut m = FakeMatch::locked_unfinished(1, Utc::now() + Duration::hours(2));
    m.team_away_id = None;
    h.matches.seed(m);
    let user = fake_user();
    let form = PredictionForm {
        score_home: Some(2),
        score_away: Some(1),
        outcome: String::new(),
    };
    let res = predict_match(State(h.state.clone()), user, Path(1), Form(form)).await;
    assert_eq!(res.unwrap_err().0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn predict_match_persists_valid_tip() {
    let h = build_harness();
    h.matches.seed(FakeMatch::locked_unfinished(
        7,
        Utc::now() + Duration::hours(2),
    ));
    let user = fake_user();
    let user_id = user.id;
    let form = PredictionForm {
        score_home: Some(3),
        score_away: Some(1),
        outcome: String::new(),
    };
    let html = predict_match(State(h.state.clone()), user, Path(7), Form(form))
        .await
        .expect("ok");
    let body = html.0;
    assert!(
        body.contains("score_home"),
        "response should be inline form"
    );
    assert!(
        body.contains(&format!(
            r#"hx-post="{}/predict/7""#,
            league_scope_path(DEFAULT_LEAGUE_ID)
        )),
        "response should keep subsequent saves inside the league scope"
    );
    assert!(body.contains("value=\"3\""));
    assert!(body.contains("value=\"1\""));

    let stored = h.predictions.all();
    assert_eq!(stored, vec![(user_id, 7, 3, 1)]);
}

#[tokio::test]
async fn predict_match_persists_winner_only_home_tip_as_canonical_scores() {
    let h = build_harness();
    h.matches.seed(FakeMatch::locked_unfinished(
        8,
        Utc::now() + Duration::hours(2),
    ));

    use pila::repo::league::LeagueConfig;
    h.state
        .repos
        .leagues
        .set_setting(
            DEFAULT_LEAGUE_ID,
            LeagueConfig::KEY_MATCH_SCORING_SYSTEM,
            Some(pila::scoring::MatchScoringSystem::WINNER_ONLY_VALUE),
        )
        .await
        .unwrap();

    let user = fake_user();
    let user_id = user.id;
    let form = PredictionForm {
        score_home: None,
        score_away: None,
        outcome: "home".into(),
    };
    let html = predict_match(State(h.state.clone()), user, Path(8), Form(form))
        .await
        .expect("ok");
    let body = html.0;
    assert!(body.contains("name=\"outcome\""));
    assert!(body.contains("value=\"home\""));
    assert!(body.contains("TeamA"));
    assert!(body.contains("TeamB"));

    let stored = h.predictions.all();
    assert_eq!(stored, vec![(user_id, 8, 1, 0)]);
}

#[tokio::test]
async fn predict_match_persists_winner_only_draw_tip_for_group_stage() {
    let h = build_harness();
    h.matches.seed(FakeMatch::locked_unfinished(
        9,
        Utc::now() + Duration::hours(2),
    ));

    use pila::repo::league::LeagueConfig;
    h.state
        .repos
        .leagues
        .set_setting(
            DEFAULT_LEAGUE_ID,
            LeagueConfig::KEY_MATCH_SCORING_SYSTEM,
            Some(pila::scoring::MatchScoringSystem::WINNER_ONLY_VALUE),
        )
        .await
        .unwrap();

    let user = fake_user();
    let user_id = user.id;
    let form = PredictionForm {
        score_home: None,
        score_away: None,
        outcome: "draw".into(),
    };
    let _ = predict_match(State(h.state.clone()), user, Path(9), Form(form))
        .await
        .expect("ok");

    let stored = h.predictions.all();
    assert_eq!(stored, vec![(user_id, 9, 0, 0)]);
}

#[tokio::test]
async fn predict_match_rejects_winner_only_draw_tip_for_knockout_stage() {
    let h = build_harness();
    let mut m = FakeMatch::locked_unfinished(10, Utc::now() + Duration::hours(2));
    m.stage = Stage::RoundOf16;
    h.matches.seed(m);

    use pila::repo::league::LeagueConfig;
    h.state
        .repos
        .leagues
        .set_setting(
            DEFAULT_LEAGUE_ID,
            LeagueConfig::KEY_MATCH_SCORING_SYSTEM,
            Some(pila::scoring::MatchScoringSystem::WINNER_ONLY_VALUE),
        )
        .await
        .unwrap();

    let form = PredictionForm {
        score_home: None,
        score_away: None,
        outcome: "draw".into(),
    };
    let res = predict_match(State(h.state.clone()), fake_user(), Path(10), Form(form)).await;
    assert_eq!(res.unwrap_err().0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn predict_match_rejects_group_stage_for_ko_only_league() {
    let h = build_harness();
    let mut m = FakeMatch::locked_unfinished(1, Utc::now() + Duration::hours(2));
    m.stage = Stage::Group;
    h.matches.seed(m);

    use pila::repo::league::LeagueConfig;
    h.state
        .repos
        .leagues
        .set_setting(DEFAULT_LEAGUE_ID, LeagueConfig::KEY_KO_ONLY, Some("true"))
        .await
        .unwrap();

    let user = fake_user();
    let form = PredictionForm {
        score_home: Some(2),
        score_away: Some(1),
        outcome: String::new(),
    };
    let res = predict_match(State(h.state.clone()), user, Path(1), Form(form)).await;
    assert_eq!(res.unwrap_err().0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn predict_match_allows_winner_only_knockout_tip_in_ko_only_league() {
    let h = build_harness();
    let mut m = FakeMatch::locked_unfinished(11, Utc::now() + Duration::hours(2));
    m.stage = Stage::QuarterFinal;
    h.matches.seed(m);

    use pila::repo::league::LeagueConfig;
    h.state
        .repos
        .leagues
        .set_setting(DEFAULT_LEAGUE_ID, LeagueConfig::KEY_KO_ONLY, Some("true"))
        .await
        .unwrap();
    h.state
        .repos
        .leagues
        .set_setting(
            DEFAULT_LEAGUE_ID,
            LeagueConfig::KEY_MATCH_SCORING_SYSTEM,
            Some(pila::scoring::MatchScoringSystem::WINNER_ONLY_VALUE),
        )
        .await
        .unwrap();

    let user = fake_user();
    let user_id = user.id;
    let res = predict_match(
        State(h.state.clone()),
        user,
        Path(11),
        Form(PredictionForm {
            score_home: None,
            score_away: None,
            outcome: "away".into(),
        }),
    )
    .await;
    assert!(
        res.is_ok(),
        "winner-only knockout tip should be allowed in KO-only league"
    );
    assert_eq!(h.predictions.all(), vec![(user_id, 11, 0, 1)]);
}

#[tokio::test]
async fn predict_match_overwrites_existing_tip() {
    let h = build_harness();
    h.matches.seed(FakeMatch::locked_unfinished(
        7,
        Utc::now() + Duration::hours(2),
    ));
    let user = fake_user();
    let user_id = user.id;
    let _ = predict_match(
        State(h.state.clone()),
        AuthenticatedUser {
            id: user_id,
            ..fake_user()
        },
        Path(7),
        Form(PredictionForm {
            score_home: Some(1),
            score_away: Some(0),
            outcome: String::new(),
        }),
    )
    .await
    .unwrap();
    let _ = predict_match(
        State(h.state.clone()),
        user,
        Path(7),
        Form(PredictionForm {
            score_home: Some(2),
            score_away: Some(2),
            outcome: String::new(),
        }),
    )
    .await
    .unwrap();

    let stored = h.predictions.all();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].2, 2);
    assert_eq!(stored[0].3, 2);
}

// ─── predict_special ──────────────────────────────────────────────────────────

#[tokio::test]
async fn predict_special_allows_champion_pick_until_first_ko_for_ko_only_league() {
    let h = build_harness();
    // Group match already kicked off
    let mut group = FakeMatch::locked_unfinished(1, Utc::now() - Duration::hours(1));
    group.stage = Stage::Group;
    h.matches.seed(group);
    // First KO match is still in the future
    let mut ko = FakeMatch::locked_unfinished(2, Utc::now() + Duration::hours(2));
    ko.stage = Stage::RoundOf16;
    h.matches.seed(ko);

    use pila::repo::league::LeagueConfig;
    h.state
        .repos
        .leagues
        .set_setting(DEFAULT_LEAGUE_ID, LeagueConfig::KEY_KO_ONLY, Some("true"))
        .await
        .unwrap();

    h.teams.seed(TeamOption {
        id: 11,
        name: "Germany".into(),
        flag_code: Some("de".into()),
    });

    let res = predict_special(
        State(h.state.clone()),
        fake_user(),
        Form(SpecialPredictionForm {
            champion_id: Some(11),
        }),
    )
    .await;
    assert!(
        res.is_ok(),
        "champion pick should be allowed until first KO match"
    );
}

#[tokio::test]
async fn predict_special_allows_champion_pick_after_group_kickoff() {
    let h = build_harness();
    // Group stage has already kicked off, but no knockout match has started yet.
    let mut group = FakeMatch::locked_unfinished(1, Utc::now() - Duration::hours(1));
    group.stage = Stage::Group;
    h.matches.seed(group);
    let mut ko = FakeMatch::locked_unfinished(2, Utc::now() + Duration::hours(2));
    ko.stage = Stage::RoundOf16;
    h.matches.seed(ko);
    h.teams.seed(TeamOption {
        id: 11,
        name: "Germany".into(),
        flag_code: Some("de".into()),
    });
    let res = predict_special(
        State(h.state.clone()),
        fake_user(),
        Form(SpecialPredictionForm {
            champion_id: Some(11),
        }),
    )
    .await;
    assert!(
        res.is_ok(),
        "champion pick should stay editable until the knockout stage begins"
    );
}

#[tokio::test]
async fn predict_special_rejects_after_knockout_kickoff() {
    let h = build_harness();
    let mut m = FakeMatch::locked_unfinished(1, Utc::now() - Duration::hours(1));
    m.stage = Stage::RoundOf16;
    h.matches.seed(m);
    let user = fake_user();
    let res = predict_special(
        State(h.state.clone()),
        user,
        Form(SpecialPredictionForm {
            champion_id: Some(11),
        }),
    )
    .await;
    assert_eq!(res.unwrap_err().0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn predict_special_rejects_unknown_team() {
    let h = build_harness();
    h.matches.seed(FakeMatch::locked_unfinished(
        1,
        Utc::now() + Duration::hours(2),
    ));
    h.teams.seed(TeamOption {
        id: 11,
        name: "Germany".into(),
        flag_code: Some("de".into()),
    });
    let res = predict_special(
        State(h.state.clone()),
        fake_user(),
        Form(SpecialPredictionForm {
            champion_id: Some(99),
        }),
    )
    .await;
    assert_eq!(res.unwrap_err().0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn predict_special_rejects_placeholder_team() {
    let h = build_harness();
    h.matches.seed(FakeMatch::locked_unfinished(
        1,
        Utc::now() + Duration::hours(2),
    ));
    h.teams.seed(TeamOption {
        id: 50,
        name: "Group A Winner".into(),
        flag_code: None,
    });
    let res = predict_special(
        State(h.state.clone()),
        fake_user(),
        Form(SpecialPredictionForm {
            champion_id: Some(50),
        }),
    )
    .await;
    assert_eq!(res.unwrap_err().0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn predict_special_persists_real_team_pick() {
    let h = build_harness();
    h.matches.seed(FakeMatch::locked_unfinished(
        1,
        Utc::now() + Duration::hours(2),
    ));
    h.teams.seed(TeamOption {
        id: 11,
        name: "Germany".into(),
        flag_code: Some("de".into()),
    });
    let user = fake_user();
    let user_id = user.id;
    let res = predict_special(
        State(h.state.clone()),
        user,
        Form(SpecialPredictionForm {
            champion_id: Some(11),
        }),
    )
    .await;
    assert!(res.is_ok());

    use pila::repo::SpecialPredictionRepo;
    let v = SpecialPredictionRepo::get_user_champion(&*h.special_predictions, user_id)
        .await
        .unwrap();
    assert_eq!(v, Some(11));
}

#[tokio::test]
async fn predict_special_allows_clearing_pick() {
    let h = build_harness();
    h.matches.seed(FakeMatch::locked_unfinished(
        1,
        Utc::now() + Duration::hours(2),
    ));
    let user = fake_user();
    let user_id = user.id;
    let _ = predict_special(
        State(h.state.clone()),
        user,
        Form(SpecialPredictionForm { champion_id: None }),
    )
    .await
    .unwrap();

    use pila::repo::SpecialPredictionRepo;
    let v = SpecialPredictionRepo::get_user_champion(&*h.special_predictions, user_id)
        .await
        .unwrap();
    assert_eq!(v, None);
}
