// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Scoring rules for match and champion predictions.
//!
//! The match scorer is deliberately modular: leagues select a
//! [`MatchScoringSystem`], and the app dispatches into the corresponding
//! strategy. This keeps future scoring systems additive instead of forcing
//! more conditionals into the call sites.

use crate::stage::Stage;

/// Per-league scoring system for ordinary match predictions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatchScoringSystem {
    /// Classic exact-score mode: exact result, goal difference, tendency.
    #[default]
    ExactScore,
    /// Winner-only mode: tip only home win, away win, or draw in groups.
    WinnerOnly,
}

impl MatchScoringSystem {
    pub const EXACT_SCORE_VALUE: &'static str = "exact_score";
    pub const WINNER_ONLY_VALUE: &'static str = "winner_only";

    pub fn as_setting_value(self) -> &'static str {
        match self {
            MatchScoringSystem::ExactScore => Self::EXACT_SCORE_VALUE,
            MatchScoringSystem::WinnerOnly => Self::WINNER_ONLY_VALUE,
        }
    }

    pub fn from_setting_value(value: &str) -> Option<Self> {
        match value {
            Self::EXACT_SCORE_VALUE => Some(Self::ExactScore),
            Self::WINNER_ONLY_VALUE => Some(Self::WinnerOnly),
            _ => None,
        }
    }

    pub fn is_winner_only(self) -> bool {
        matches!(self, Self::WinnerOnly)
    }
}

/// Match phases used by the scoring tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TournamentPhase {
    Group,
    R32,
    R16,
    QF,
    SF,
    Finals,
}

/// Classic exact-score categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BetCategory {
    Exact,
    Difference,
    Tendency,
    Wrong,
}

/// Winner-only result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BetResult {
    Correct,
    Wrong,
}

/// Full result of the classic exact-score scorer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoringResult {
    pub category: BetCategory,
    pub points: i32,
}

/// Full result of the winner-only scorer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WinnerOnlyScoringResult {
    pub result: BetResult,
    pub points: i32,
}

/// Winner-only domain object used to distinguish a real draw from invalid data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinnerSelection {
    Team(i32),
    Draw,
}

/// Winner-only prediction choices stored as canonical pseudo-scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeBet {
    HomeWin,
    Draw,
    AwayWin,
}

impl OutcomeBet {
    pub fn to_stored_scores(self) -> (i32, i32) {
        match self {
            OutcomeBet::HomeWin => (1, 0),
            OutcomeBet::Draw => (0, 0),
            OutcomeBet::AwayWin => (0, 1),
        }
    }

    pub fn from_stored_scores(home: i32, away: i32, allow_draw: bool) -> Option<Self> {
        match home.cmp(&away) {
            std::cmp::Ordering::Greater => Some(Self::HomeWin),
            std::cmp::Ordering::Equal if allow_draw => Some(Self::Draw),
            std::cmp::Ordering::Less => Some(Self::AwayWin),
            std::cmp::Ordering::Equal => None,
        }
    }

    pub fn from_form_value(value: &str, allow_draw: bool) -> Option<Self> {
        match value {
            "home" => Some(Self::HomeWin),
            "away" => Some(Self::AwayWin),
            "draw" if allow_draw => Some(Self::Draw),
            _ => None,
        }
    }

}

/// Winner-only service kept separate from the classic exact-score rules.
pub struct WinnerOnlyScoringService;

impl WinnerOnlyScoringService {
    /// Compares two team identifiers. Invalid or missing ids are scored wrong.
    pub fn evaluate_bet_result(
        actual_winner_id: Option<i32>,
        predicted_winner_id: Option<i32>,
    ) -> BetResult {
        match (
            normalize_team_id(actual_winner_id),
            normalize_team_id(predicted_winner_id),
        ) {
            (Some(actual), Some(predicted)) if actual == predicted => BetResult::Correct,
            _ => BetResult::Wrong,
        }
    }

    /// Compares explicit winner selections. `None` is treated as invalid data.
    pub fn evaluate_selection(
        actual: Option<WinnerSelection>,
        predicted: Option<WinnerSelection>,
    ) -> BetResult {
        match (actual, predicted) {
            (Some(actual), Some(predicted)) if actual == predicted => BetResult::Correct,
            _ => BetResult::Wrong,
        }
    }

    pub fn calculate_points(phase: TournamentPhase, result: BetResult) -> i32 {
        match (phase, result) {
            (TournamentPhase::Group, BetResult::Correct) => 1,
            (TournamentPhase::R32, BetResult::Correct) => 2,
            (TournamentPhase::R16, BetResult::Correct) => 3,
            (TournamentPhase::QF, BetResult::Correct) => 5,
            (TournamentPhase::SF, BetResult::Correct) => 7,
            (TournamentPhase::Finals, BetResult::Correct) => 7,
            (_, BetResult::Wrong) => 0,
        }
    }

    pub fn score_by_ids(
        actual_winner_id: Option<i32>,
        predicted_winner_id: Option<i32>,
        phase: TournamentPhase,
    ) -> WinnerOnlyScoringResult {
        let result = Self::evaluate_bet_result(actual_winner_id, predicted_winner_id);
        let points = Self::calculate_points(phase, result);
        WinnerOnlyScoringResult { result, points }
    }

    pub fn score_selection(
        actual: Option<WinnerSelection>,
        predicted: Option<WinnerSelection>,
        phase: TournamentPhase,
    ) -> WinnerOnlyScoringResult {
        let result = Self::evaluate_selection(actual, predicted);
        let points = Self::calculate_points(phase, result);
        WinnerOnlyScoringResult { result, points }
    }
}

fn normalize_team_id(team_id: Option<i32>) -> Option<i32> {
    team_id.filter(|id| *id > 0)
}

pub fn outcome_bet_from_form(value: &str, allow_draw: bool) -> Option<OutcomeBet> {
    OutcomeBet::from_form_value(value, allow_draw)
}

pub fn outcome_bet_from_stored_scores(
    home: i32,
    away: i32,
    allow_draw: bool,
) -> Option<OutcomeBet> {
    OutcomeBet::from_stored_scores(home, away, allow_draw)
}

pub fn outcome_bet_to_stored_scores(outcome: OutcomeBet) -> (i32, i32) {
    outcome.to_stored_scores()
}

pub fn winner_only_prediction_label(
    outcome: OutcomeBet,
    home_name: &str,
    away_name: &str,
) -> String {
    match outcome {
        OutcomeBet::HomeWin => home_name.to_string(),
        OutcomeBet::Draw => "=".to_string(),
        OutcomeBet::AwayWin => away_name.to_string(),
    }
}

pub fn winner_only_score_match(
    stage: Stage,
    actual_h: i32,
    actual_a: i32,
    pred_h: i32,
    pred_a: i32,
) -> WinnerOnlyScoringResult {
    let phase = stage.to_tournament_phase();
    let actual = selection_from_scores(actual_h, actual_a, stage == Stage::Group);
    let predicted = selection_from_scores(pred_h, pred_a, stage == Stage::Group);
    WinnerOnlyScoringService::score_selection(actual, predicted, phase)
}

fn selection_from_scores(home: i32, away: i32, allow_draw: bool) -> Option<WinnerSelection> {
    match home.cmp(&away) {
        std::cmp::Ordering::Greater => Some(WinnerSelection::Team(1)),
        std::cmp::Ordering::Less => Some(WinnerSelection::Team(2)),
        std::cmp::Ordering::Equal if allow_draw => Some(WinnerSelection::Draw),
        std::cmp::Ordering::Equal => None,
    }
}

/// Determines the highest applicable `BetCategory` for a classic exact-score bet.
pub fn evaluate_bet_category(
    goals_home: i32,
    goals_away: i32,
    bet_home: i32,
    bet_away: i32,
) -> BetCategory {
    if bet_home == goals_home && bet_away == goals_away {
        return BetCategory::Exact;
    }

    let actual_diff = goals_home - goals_away;
    let pred_diff = bet_home - bet_away;

    if actual_diff == pred_diff {
        return BetCategory::Difference;
    }

    if actual_diff.signum() == pred_diff.signum() {
        return BetCategory::Tendency;
    }

    BetCategory::Wrong
}

/// Points matrix for the classic exact-score mode.
pub fn calculate_points(phase: TournamentPhase, category: BetCategory) -> i32 {
    match (phase, category) {
        (TournamentPhase::Group, BetCategory::Exact) => 4,
        (TournamentPhase::Group, BetCategory::Difference) => 3,
        (TournamentPhase::Group, BetCategory::Tendency) => 2,
        (TournamentPhase::Group, BetCategory::Wrong) => 0,

        (TournamentPhase::R32, BetCategory::Exact) => 6,
        (TournamentPhase::R32, BetCategory::Difference) => 4,
        (TournamentPhase::R32, BetCategory::Tendency) => 3,
        (TournamentPhase::R32, BetCategory::Wrong) => 0,

        (TournamentPhase::R16, BetCategory::Exact) => 6,
        (TournamentPhase::R16, BetCategory::Difference) => 4,
        (TournamentPhase::R16, BetCategory::Tendency) => 3,
        (TournamentPhase::R16, BetCategory::Wrong) => 0,

        (TournamentPhase::QF, BetCategory::Exact) => 8,
        (TournamentPhase::QF, BetCategory::Difference) => 6,
        (TournamentPhase::QF, BetCategory::Tendency) => 5,
        (TournamentPhase::QF, BetCategory::Wrong) => 0,

        (TournamentPhase::SF, BetCategory::Exact) => 8,
        (TournamentPhase::SF, BetCategory::Difference) => 6,
        (TournamentPhase::SF, BetCategory::Tendency) => 5,
        (TournamentPhase::SF, BetCategory::Wrong) => 0,

        (TournamentPhase::Finals, BetCategory::Exact) => 11,
        (TournamentPhase::Finals, BetCategory::Difference) => 8,
        (TournamentPhase::Finals, BetCategory::Tendency) => 6,
        (TournamentPhase::Finals, BetCategory::Wrong) => 0,
    }
}

pub fn max_points_for_phase_with_system(system: MatchScoringSystem, phase: TournamentPhase) -> i32 {
    match system {
        MatchScoringSystem::ExactScore => calculate_points(phase, BetCategory::Exact),
        MatchScoringSystem::WinnerOnly => {
            WinnerOnlyScoringService::calculate_points(phase, BetResult::Correct)
        }
    }
}

pub fn score_match(
    phase: TournamentPhase,
    goals_home: i32,
    goals_away: i32,
    bet_home: i32,
    bet_away: i32,
) -> ScoringResult {
    let category = evaluate_bet_category(goals_home, goals_away, bet_home, bet_away);
    let points = calculate_points(phase, category);
    ScoringResult { category, points }
}

pub fn calculate_match_points_for_system(
    system: MatchScoringSystem,
    stage: Stage,
    actual_h: i32,
    actual_a: i32,
    pred_h: i32,
    pred_a: i32,
) -> i32 {
    match system {
        MatchScoringSystem::ExactScore => {
            calculate_match_points(stage, actual_h, actual_a, pred_h, pred_a)
        }
        MatchScoringSystem::WinnerOnly => {
            winner_only_score_match(stage, actual_h, actual_a, pred_h, pred_a).points
        }
    }
}

pub(crate) fn calculate_match_points(
    stage: Stage,
    actual_h: i32,
    actual_a: i32,
    pred_h: i32,
    pred_a: i32,
) -> i32 {
    score_match(
        stage.to_tournament_phase(),
        actual_h,
        actual_a,
        pred_h,
        pred_a,
    )
    .points
}

pub fn max_potential_points_for_system(system: MatchScoringSystem, stage: Stage) -> i32 {
    max_points_for_phase_with_system(system, stage.to_tournament_phase())
}

/// 10 points if the predicted Weltmeister team matches the actual champion.
pub fn champion_points(predicted: Option<i32>, actual: Option<i32>) -> i32 {
    match (predicted, actual) {
        (Some(predicted), Some(actual)) if predicted == actual => 10,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod exact_score {
        use super::*;

        #[test]
        fn evaluate_bet_category_returns_exact_for_identical_score() {
            assert_eq!(evaluate_bet_category(2, 1, 2, 1), BetCategory::Exact);
        }

        #[test]
        fn evaluate_bet_category_returns_difference_for_same_goal_diff() {
            assert_eq!(evaluate_bet_category(3, 2, 2, 1), BetCategory::Difference);
        }

        #[test]
        fn evaluate_bet_category_returns_tendency_for_same_match_outcome() {
            assert_eq!(evaluate_bet_category(4, 1, 2, 0), BetCategory::Tendency);
        }

        #[test]
        fn evaluate_bet_category_returns_wrong_for_opposite_outcome() {
            assert_eq!(evaluate_bet_category(1, 2, 2, 1), BetCategory::Wrong);
        }

        #[test]
        fn calculate_points_returns_group_exact_value() {
            assert_eq!(
                calculate_points(TournamentPhase::Group, BetCategory::Exact),
                4
            );
        }

        #[test]
        fn calculate_points_returns_round_of_16_tendency_value() {
            assert_eq!(
                calculate_points(TournamentPhase::R16, BetCategory::Tendency),
                3
            );
        }

        #[test]
        fn calculate_points_returns_finals_difference_value() {
            assert_eq!(
                calculate_points(TournamentPhase::Finals, BetCategory::Difference),
                8
            );
        }

        #[test]
        fn score_match_returns_category_and_points() {
            let result = score_match(TournamentPhase::QF, 3, 2, 1, 0);
            assert_eq!(
                result,
                ScoringResult {
                    category: BetCategory::Difference,
                    points: 6,
                }
            );
        }

        #[test]
        fn calculate_match_points_keeps_legacy_exact_score_behavior() {
            assert_eq!(calculate_match_points(Stage::Final, 5, 4, 5, 4), 11);
        }

        #[test]
        fn max_potential_points_for_exact_score_semi_final() {
            assert_eq!(
                max_potential_points_for_system(MatchScoringSystem::ExactScore, Stage::SemiFinal),
                8
            );
        }
    }

    mod winner_only {
        use super::*;

        #[test]
        fn evaluate_bet_result_returns_correct_for_matching_team_ids() {
            assert_eq!(
                WinnerOnlyScoringService::evaluate_bet_result(Some(11), Some(11)),
                BetResult::Correct
            );
        }

        #[test]
        fn evaluate_bet_result_returns_wrong_for_different_team_ids() {
            assert_eq!(
                WinnerOnlyScoringService::evaluate_bet_result(Some(11), Some(22)),
                BetResult::Wrong
            );
        }

        #[test]
        fn evaluate_bet_result_returns_wrong_for_none_values() {
            assert_eq!(
                WinnerOnlyScoringService::evaluate_bet_result(None, None),
                BetResult::Wrong
            );
        }

        #[test]
        fn evaluate_bet_result_returns_wrong_for_invalid_team_ids() {
            assert_eq!(
                WinnerOnlyScoringService::evaluate_bet_result(Some(-1), Some(-1)),
                BetResult::Wrong
            );
        }

        #[test]
        fn score_selection_returns_group_point_for_correct_home_win() {
            let result = WinnerOnlyScoringService::score_selection(
                Some(WinnerSelection::Team(1)),
                Some(WinnerSelection::Team(1)),
                TournamentPhase::Group,
            );
            assert_eq!(
                result,
                WinnerOnlyScoringResult {
                    result: BetResult::Correct,
                    points: 1,
                }
            );
        }

        #[test]
        fn score_selection_returns_group_point_for_correct_draw() {
            let result = WinnerOnlyScoringService::score_selection(
                Some(WinnerSelection::Draw),
                Some(WinnerSelection::Draw),
                TournamentPhase::Group,
            );
            assert_eq!(
                result,
                WinnerOnlyScoringResult {
                    result: BetResult::Correct,
                    points: 1,
                }
            );
        }

        #[test]
        fn score_by_ids_returns_two_points_for_correct_r32_tip() {
            let result =
                WinnerOnlyScoringService::score_by_ids(Some(10), Some(10), TournamentPhase::R32);
            assert_eq!(
                result,
                WinnerOnlyScoringResult {
                    result: BetResult::Correct,
                    points: 2,
                }
            );
        }

        #[test]
        fn score_by_ids_returns_three_points_for_correct_r16_tip() {
            let result =
                WinnerOnlyScoringService::score_by_ids(Some(10), Some(10), TournamentPhase::R16);
            assert_eq!(
                result,
                WinnerOnlyScoringResult {
                    result: BetResult::Correct,
                    points: 3,
                }
            );
        }

        #[test]
        fn score_by_ids_returns_five_points_for_correct_qf_tip() {
            let result =
                WinnerOnlyScoringService::score_by_ids(Some(10), Some(10), TournamentPhase::QF);
            assert_eq!(
                result,
                WinnerOnlyScoringResult {
                    result: BetResult::Correct,
                    points: 5,
                }
            );
        }

        #[test]
        fn score_by_ids_returns_seven_points_for_correct_sf_tip() {
            let result =
                WinnerOnlyScoringService::score_by_ids(Some(10), Some(10), TournamentPhase::SF);
            assert_eq!(
                result,
                WinnerOnlyScoringResult {
                    result: BetResult::Correct,
                    points: 7,
                }
            );
        }

        #[test]
        fn score_by_ids_returns_seven_points_for_correct_finals_tip() {
            let result =
                WinnerOnlyScoringService::score_by_ids(Some(10), Some(10), TournamentPhase::Finals);
            assert_eq!(
                result,
                WinnerOnlyScoringResult {
                    result: BetResult::Correct,
                    points: 7,
                }
            );
        }

        #[test]
        fn wrong_tip_returns_zero_points_in_group_phase() {
            let result = WinnerOnlyScoringService::score_selection(
                Some(WinnerSelection::Draw),
                Some(WinnerSelection::Team(1)),
                TournamentPhase::Group,
            );
            assert_eq!(
                result,
                WinnerOnlyScoringResult {
                    result: BetResult::Wrong,
                    points: 0,
                }
            );
        }

        #[test]
        fn wrong_tip_returns_zero_points_in_r32() {
            let result =
                WinnerOnlyScoringService::score_by_ids(Some(11), Some(22), TournamentPhase::R32);
            assert_eq!(
                result,
                WinnerOnlyScoringResult {
                    result: BetResult::Wrong,
                    points: 0,
                }
            );
        }

        #[test]
        fn wrong_tip_returns_zero_points_in_r16() {
            let result =
                WinnerOnlyScoringService::score_by_ids(Some(11), Some(22), TournamentPhase::R16);
            assert_eq!(
                result,
                WinnerOnlyScoringResult {
                    result: BetResult::Wrong,
                    points: 0,
                }
            );
        }

        #[test]
        fn wrong_tip_returns_zero_points_in_qf() {
            let result =
                WinnerOnlyScoringService::score_by_ids(Some(11), Some(22), TournamentPhase::QF);
            assert_eq!(
                result,
                WinnerOnlyScoringResult {
                    result: BetResult::Wrong,
                    points: 0,
                }
            );
        }

        #[test]
        fn wrong_tip_returns_zero_points_in_sf() {
            let result =
                WinnerOnlyScoringService::score_by_ids(Some(11), Some(22), TournamentPhase::SF);
            assert_eq!(
                result,
                WinnerOnlyScoringResult {
                    result: BetResult::Wrong,
                    points: 0,
                }
            );
        }

        #[test]
        fn wrong_tip_returns_zero_points_in_finals() {
            let result =
                WinnerOnlyScoringService::score_by_ids(Some(11), Some(22), TournamentPhase::Finals);
            assert_eq!(
                result,
                WinnerOnlyScoringResult {
                    result: BetResult::Wrong,
                    points: 0,
                }
            );
        }

        #[test]
        fn winner_only_score_match_accepts_group_draw() {
            let result = winner_only_score_match(Stage::Group, 1, 1, 0, 0);
            assert_eq!(
                result,
                WinnerOnlyScoringResult {
                    result: BetResult::Correct,
                    points: 1,
                }
            );
        }

        #[test]
        fn winner_only_score_match_rejects_knockout_draw_prediction() {
            let result = winner_only_score_match(Stage::RoundOf16, 2, 1, 0, 0);
            assert_eq!(
                result,
                WinnerOnlyScoringResult {
                    result: BetResult::Wrong,
                    points: 0,
                }
            );
        }

        #[test]
        fn calculate_match_points_for_system_dispatches_to_winner_only_rules() {
            assert_eq!(
                calculate_match_points_for_system(
                    MatchScoringSystem::WinnerOnly,
                    Stage::QuarterFinal,
                    3,
                    1,
                    1,
                    0,
                ),
                5
            );
        }

        #[test]
        fn max_potential_points_for_system_returns_winner_only_phase_maximum() {
            assert_eq!(
                max_potential_points_for_system(MatchScoringSystem::WinnerOnly, Stage::ThirdPlace,),
                7
            );
        }

        #[test]
        fn outcome_bet_round_trips_through_stored_scores() {
            let outcome = OutcomeBet::AwayWin;
            let (home, away) = outcome_bet_to_stored_scores(outcome);
            assert_eq!(
                outcome_bet_from_stored_scores(home, away, false),
                Some(OutcomeBet::AwayWin)
            );
        }
    }

    #[test]
    fn champion_points_returns_ten_for_matching_pick() {
        assert_eq!(champion_points(Some(7), Some(7)), 10);
    }

    #[test]
    fn champion_points_returns_zero_for_non_matching_pick() {
        assert_eq!(champion_points(Some(7), Some(8)), 0);
    }
}
