//! Pure Rust scoring engine for the Pila WM Tippspiel.
//!
//! Domain model uses `TournamentPhase` (four phases) and `BetCategory`
//! (four gain levels). The legacy `Stage`-based helper `calculate_match_points`
//! delegates to the new logic internally.

use crate::stage::Stage;

// ─── Domain Types ─────────────────────────────────────────────────────────────

/// The four tournament phases that determine point weights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TournamentPhase {
    /// Gruppenphase
    Group,
    /// Sechzehntelfinale & Achtelfinale
    R32R16,
    /// Viertelfinale & Halbfinale
    QFSF,
    /// Finale & Spiel um Platz 3
    Finals,
}

/// The four possible gain levels for a single match bet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BetCategory {
    /// Exact score (home AND away goals match).
    Exact,
    /// Correct goal difference, but not exact.
    Difference,
    /// Correct tendency (winner / draw), but neither exact nor diff.
    Tendency,
    /// Neither exact, diff, nor tendency — 0 points.
    Wrong,
}

/// Full result of scoring a single match bet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoringResult {
    pub category: BetCategory,
    pub points: i32,
}

// ─── Bet Evaluation ───────────────────────────────────────────────────────────

/// Determines the highest applicable `BetCategory` for a match bet.
///
/// The hierarchy is strict: EXACT > DIFFERENCE > TENDENCY > WRONG.
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

// ─── Points Matrix ────────────────────────────────────────────────────────────

/// Returns the point value for a given (phase, category) pair.
///
/// | Phase    | Exact | Diff | Tendency | Wrong |
/// |----------|-------|------|----------|-------|
/// | GROUP    |   4   |   3  |    2     |   0   |
/// | R32_R16  |   6   |   4  |    3     |   0   |
/// | QF_SF    |   8   |   6  |    5     |   0   |
/// | FINALS   |  11   |   8  |    6     |   0   |
pub fn calculate_points(phase: TournamentPhase, category: BetCategory) -> i32 {
    match (phase, category) {
        (TournamentPhase::Group, BetCategory::Exact) => 4,
        (TournamentPhase::Group, BetCategory::Difference) => 3,
        (TournamentPhase::Group, BetCategory::Tendency) => 2,
        (TournamentPhase::Group, BetCategory::Wrong) => 0,

        (TournamentPhase::R32R16, BetCategory::Exact) => 6,
        (TournamentPhase::R32R16, BetCategory::Difference) => 4,
        (TournamentPhase::R32R16, BetCategory::Tendency) => 3,
        (TournamentPhase::R32R16, BetCategory::Wrong) => 0,

        (TournamentPhase::QFSF, BetCategory::Exact) => 8,
        (TournamentPhase::QFSF, BetCategory::Difference) => 6,
        (TournamentPhase::QFSF, BetCategory::Tendency) => 5,
        (TournamentPhase::QFSF, BetCategory::Wrong) => 0,

        (TournamentPhase::Finals, BetCategory::Exact) => 11,
        (TournamentPhase::Finals, BetCategory::Difference) => 8,
        (TournamentPhase::Finals, BetCategory::Tendency) => 6,
        (TournamentPhase::Finals, BetCategory::Wrong) => 0,
    }
}

/// Maximum achievable points for a given phase (= points for EXACT).
pub fn max_points_for_phase(phase: TournamentPhase) -> i32 {
    calculate_points(phase, BetCategory::Exact)
}

// ─── Main Scoring Entry Point ────────────────────────────────────────────────

/// Score a single match bet. Returns both the `BetCategory` and the
/// integer point value.
///
/// Result always includes extra time and penalty shootout — e.g. a final
/// that ends 5:4 after penalties is stored as 5:4.
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

// ─── Legacy Convenience Wrappers ─────────────────────────────────────────────

/// Points for a single match prediction (legacy `Stage`-based API).
///
/// Delegates to the new `TournamentPhase`/`BetCategory` logic.
pub fn calculate_match_points(
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

/// Maximum points still achievable when the match has not started yet
/// (or has not finished). Returns the EXACT-score value for the phase.
pub fn max_potential_points(stage: Stage) -> i32 {
    max_points_for_phase(stage.to_tournament_phase())
}

/// 10 points if the predicted Weltmeister team matches the actual champion.
pub fn champion_points(predicted: Option<i32>, actual: Option<i32>) -> i32 {
    match (predicted, actual) {
        (Some(p), Some(a)) if p == a => 10,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── BetCategory Evaluation ──────────────────────────────────────────────

    #[test]
    fn exact_when_both_scores_match() {
        assert_eq!(
            evaluate_bet_category(2, 1, 2, 1),
            BetCategory::Exact
        );
    }

    #[test]
    fn exact_draw() {
        assert_eq!(
            evaluate_bet_category(0, 0, 0, 0),
            BetCategory::Exact
        );
    }

    #[test]
    fn difference_when_diff_matches_but_not_exact() {
        // 3:2 actual, 2:1 predicted — both diff = +1
        assert_eq!(
            evaluate_bet_category(3, 2, 2, 1),
            BetCategory::Difference
        );
    }

    #[test]
    fn difference_draw_wrong_score() {
        // 2:2 actual, 1:1 predicted — both diff = 0, not exact
        assert_eq!(
            evaluate_bet_category(2, 2, 1, 1),
            BetCategory::Difference
        );
    }

    #[test]
    fn tendency_home_win_correct() {
        // actual 4:1 (diff +3), predicted 2:0 (diff +2) → same sign
        assert_eq!(
            evaluate_bet_category(4, 1, 2, 0),
            BetCategory::Tendency
        );
    }

    #[test]
    fn tendency_away_win_correct() {
        // actual 0:3 (diff -3), predicted 1:2 (diff -1) → same sign
        assert_eq!(
            evaluate_bet_category(0, 3, 1, 2),
            BetCategory::Tendency
        );
    }

    #[test]
    fn wrong_when_predicted_draw_actual_win() {
        // actual 2:1, predicted 1:1
        assert_eq!(
            evaluate_bet_category(2, 1, 1, 1),
            BetCategory::Wrong
        );
    }

    #[test]
    fn wrong_when_predicted_win_actual_away_win() {
        // actual 1:2, predicted 2:1
        assert_eq!(
            evaluate_bet_category(1, 2, 2, 1),
            BetCategory::Wrong
        );
    }

    #[test]
    fn wrong_when_predicted_win_actual_draw() {
        // actual 1:1, predicted 2:1
        assert_eq!(
            evaluate_bet_category(1, 1, 2, 1),
            BetCategory::Wrong
        );
    }

    #[test]
    fn wrong_when_predicted_away_win_actual_home_win() {
        // actual 3:0, predicted 0:1
        assert_eq!(
            evaluate_bet_category(3, 0, 0, 1),
            BetCategory::Wrong
        );
    }

    // ─── Points Matrix — All 16 (phase × category) Combinations ─────────────

    #[test]
    fn group_exact_4() {
        assert_eq!(calculate_points(TournamentPhase::Group, BetCategory::Exact), 4);
    }
    #[test]
    fn group_difference_3() {
        assert_eq!(calculate_points(TournamentPhase::Group, BetCategory::Difference), 3);
    }
    #[test]
    fn group_tendency_2() {
        assert_eq!(calculate_points(TournamentPhase::Group, BetCategory::Tendency), 2);
    }
    #[test]
    fn group_wrong_0() {
        assert_eq!(calculate_points(TournamentPhase::Group, BetCategory::Wrong), 0);
    }

    #[test]
    fn r32_r16_exact_6() {
        assert_eq!(calculate_points(TournamentPhase::R32R16, BetCategory::Exact), 6);
    }
    #[test]
    fn r32_r16_difference_4() {
        assert_eq!(calculate_points(TournamentPhase::R32R16, BetCategory::Difference), 4);
    }
    #[test]
    fn r32_r16_tendency_3() {
        assert_eq!(calculate_points(TournamentPhase::R32R16, BetCategory::Tendency), 3);
    }
    #[test]
    fn r32_r16_wrong_0() {
        assert_eq!(calculate_points(TournamentPhase::R32R16, BetCategory::Wrong), 0);
    }

    #[test]
    fn qf_sf_exact_8() {
        assert_eq!(calculate_points(TournamentPhase::QFSF, BetCategory::Exact), 8);
    }
    #[test]
    fn qf_sf_difference_6() {
        assert_eq!(calculate_points(TournamentPhase::QFSF, BetCategory::Difference), 6);
    }
    #[test]
    fn qf_sf_tendency_5() {
        assert_eq!(calculate_points(TournamentPhase::QFSF, BetCategory::Tendency), 5);
    }
    #[test]
    fn qf_sf_wrong_0() {
        assert_eq!(calculate_points(TournamentPhase::QFSF, BetCategory::Wrong), 0);
    }

    #[test]
    fn finals_exact_11() {
        assert_eq!(calculate_points(TournamentPhase::Finals, BetCategory::Exact), 11);
    }
    #[test]
    fn finals_difference_8() {
        assert_eq!(calculate_points(TournamentPhase::Finals, BetCategory::Difference), 8);
    }
    #[test]
    fn finals_tendency_6() {
        assert_eq!(calculate_points(TournamentPhase::Finals, BetCategory::Tendency), 6);
    }
    #[test]
    fn finals_wrong_0() {
        assert_eq!(calculate_points(TournamentPhase::Finals, BetCategory::Wrong), 0);
    }

    // ─── max_points_for_phase ───────────────────────────────────────────────

    #[test]
    fn max_points_group() {
        assert_eq!(max_points_for_phase(TournamentPhase::Group), 4);
    }
    #[test]
    fn max_points_r32_r16() {
        assert_eq!(max_points_for_phase(TournamentPhase::R32R16), 6);
    }
    #[test]
    fn max_points_qf_sf() {
        assert_eq!(max_points_for_phase(TournamentPhase::QFSF), 8);
    }
    #[test]
    fn max_points_finals() {
        assert_eq!(max_points_for_phase(TournamentPhase::Finals), 11);
    }

    // ─── score_match — Integration ──────────────────────────────────────────

    #[test]
    fn score_match_group_exact() {
        let r = score_match(TournamentPhase::Group, 2, 1, 2, 1);
        assert_eq!(r.category, BetCategory::Exact);
        assert_eq!(r.points, 4);
    }

    #[test]
    fn score_match_r32_r16_tendency() {
        // Actual 3:0 (diff +3), bet 2:1 (diff +1) → same sign, different diff → TENDENCY = 3
        let r = score_match(TournamentPhase::R32R16, 3, 0, 2, 1);
        assert_eq!(r.category, BetCategory::Tendency);
        assert_eq!(r.points, 3);
    }

    #[test]
    fn score_match_qf_sf_difference() {
        let r = score_match(TournamentPhase::QFSF, 3, 2, 1, 0);
        assert_eq!(r.category, BetCategory::Difference);
        assert_eq!(r.points, 6);
    }

    #[test]
    fn score_match_finals_wrong() {
        let r = score_match(TournamentPhase::Finals, 1, 2, 2, 0);
        assert_eq!(r.category, BetCategory::Wrong);
        assert_eq!(r.points, 0);
    }

    // ─── Draw Special Cases ─────────────────────────────────────────────────

    #[test]
    fn group_draw_exact_gets_4() {
        assert_eq!(calculate_match_points(Stage::Group, 1, 1, 1, 1), 4);
    }

    #[test]
    fn group_draw_wrong_score_gets_difference_3() {
        // Tipp 1:1, Ergebnis 2:2 — diff 0 == 0, not exact → DIFFERENCE = 3 points
        assert_eq!(calculate_match_points(Stage::Group, 2, 2, 1, 1), 3);
    }

    #[test]
    fn group_draw_predicted_draw_actual_win_0() {
        assert_eq!(calculate_match_points(Stage::Group, 2, 1, 1, 1), 0);
    }

    // ─── KO Phase with High Scores (Penalty Shootout) ───────────────────────

    #[test]
    fn finals_penalty_shootout_tendency() {
        // Real result 5:4 (diff +1), bet 3:1 (diff +2) → same sign, different diff → TENDENCY in FINALS = 6 points
        assert_eq!(calculate_match_points(Stage::Final, 5, 4, 3, 1), 6);
    }

    #[test]
    fn finals_penalty_shootout_difference() {
        // Real 5:4, bet 3:2 → diff +1 == +1 → DIFFERENCE in FINALS = 8 points
        assert_eq!(calculate_match_points(Stage::Final, 5, 4, 3, 2), 8);
    }

    #[test]
    fn finals_penalty_shootout_exact() {
        // Real 5:4, bet 5:4 → EXACT in FINALS = 11 points
        assert_eq!(calculate_match_points(Stage::Final, 5, 4, 5, 4), 11);
    }

    #[test]
    fn third_place_penalty_shootout_tendency() {
        // Real 4:3 (diff +1), bet 2:0 (diff +2) → same sign, different diff → TENDENCY in FINALS phase = 6 points
        assert_eq!(calculate_match_points(Stage::ThirdPlace, 4, 3, 2, 0), 6);
    }

    // ─── Negative Tests — Wrong Tendency in All Phases ────────────────────

    #[test]
    fn wrong_tendency_group_0() {
        assert_eq!(calculate_match_points(Stage::Group, 1, 2, 2, 1), 0);
    }

    #[test]
    fn wrong_tendency_r32_0() {
        assert_eq!(calculate_match_points(Stage::RoundOf32, 0, 1, 1, 0), 0);
    }

    #[test]
    fn wrong_tendency_r16_0() {
        assert_eq!(calculate_match_points(Stage::RoundOf16, 0, 1, 1, 0), 0);
    }

    #[test]
    fn wrong_tendency_qf_0() {
        assert_eq!(calculate_match_points(Stage::QuarterFinal, 0, 1, 1, 0), 0);
    }

    #[test]
    fn wrong_tendency_sf_0() {
        assert_eq!(calculate_match_points(Stage::SemiFinal, 0, 1, 1, 0), 0);
    }

    #[test]
    fn wrong_tendency_third_place_0() {
        assert_eq!(calculate_match_points(Stage::ThirdPlace, 0, 1, 1, 0), 0);
    }

    #[test]
    fn wrong_tendency_final_0() {
        assert_eq!(calculate_match_points(Stage::Final, 0, 1, 1, 0), 0);
    }

    // ─── All Four Categories per Phase — Systematic ───────────────────────

    #[test]
    fn all_categories_group() {
        // EXACT: 2:1 bet, 2:1 actual → 4
        assert_eq!(calculate_match_points(Stage::Group, 2, 1, 2, 1), 4);
        // DIFFERENCE: 2:1 bet, 3:2 actual → 3
        assert_eq!(calculate_match_points(Stage::Group, 3, 2, 2, 1), 3);
        // TENDENCY: 2:0 bet, 4:1 actual → 2
        assert_eq!(calculate_match_points(Stage::Group, 4, 1, 2, 0), 2);
        // WRONG: 2:1 bet, 1:2 actual → 0
        assert_eq!(calculate_match_points(Stage::Group, 1, 2, 2, 1), 0);
    }

    #[test]
    fn all_categories_r32() {
        assert_eq!(calculate_match_points(Stage::RoundOf32, 2, 1, 2, 1), 6);
        assert_eq!(calculate_match_points(Stage::RoundOf32, 3, 2, 2, 1), 4);
        assert_eq!(calculate_match_points(Stage::RoundOf32, 4, 1, 2, 0), 3);
        assert_eq!(calculate_match_points(Stage::RoundOf32, 1, 2, 2, 1), 0);
    }

    #[test]
    fn all_categories_r16() {
        // R16 maps to same phase as R32
        assert_eq!(calculate_match_points(Stage::RoundOf16, 2, 1, 2, 1), 6);
        assert_eq!(calculate_match_points(Stage::RoundOf16, 3, 2, 2, 1), 4);
        assert_eq!(calculate_match_points(Stage::RoundOf16, 4, 1, 2, 0), 3);
        assert_eq!(calculate_match_points(Stage::RoundOf16, 1, 2, 2, 1), 0);
    }

    #[test]
    fn all_categories_qf() {
        assert_eq!(calculate_match_points(Stage::QuarterFinal, 2, 1, 2, 1), 8);
        assert_eq!(calculate_match_points(Stage::QuarterFinal, 3, 2, 2, 1), 6);
        assert_eq!(calculate_match_points(Stage::QuarterFinal, 4, 1, 2, 0), 5);
        assert_eq!(calculate_match_points(Stage::QuarterFinal, 1, 2, 2, 1), 0);
    }

    #[test]
    fn all_categories_sf() {
        // SF maps to same phase as QF
        assert_eq!(calculate_match_points(Stage::SemiFinal, 2, 1, 2, 1), 8);
        assert_eq!(calculate_match_points(Stage::SemiFinal, 3, 2, 2, 1), 6);
        assert_eq!(calculate_match_points(Stage::SemiFinal, 4, 1, 2, 0), 5);
        assert_eq!(calculate_match_points(Stage::SemiFinal, 1, 2, 2, 1), 0);
    }

    #[test]
    fn all_categories_third_place() {
        // Third place maps to FINALS phase
        assert_eq!(calculate_match_points(Stage::ThirdPlace, 2, 1, 2, 1), 11);
        assert_eq!(calculate_match_points(Stage::ThirdPlace, 3, 2, 2, 1), 8);
        assert_eq!(calculate_match_points(Stage::ThirdPlace, 4, 1, 2, 0), 6);
        assert_eq!(calculate_match_points(Stage::ThirdPlace, 1, 2, 2, 1), 0);
    }

    #[test]
    fn all_categories_final() {
        assert_eq!(calculate_match_points(Stage::Final, 2, 1, 2, 1), 11);
        assert_eq!(calculate_match_points(Stage::Final, 3, 2, 2, 1), 8);
        assert_eq!(calculate_match_points(Stage::Final, 4, 1, 2, 0), 6);
        assert_eq!(calculate_match_points(Stage::Final, 1, 2, 2, 1), 0);
    }

    // ─── KO-Draw Edge Cases ───────────────────────────────────────────────

    #[test]
    fn ko_draw_after_extra_time_exact() {
        // K.O. matches can end in a draw (before penalty shootout)
        assert_eq!(calculate_match_points(Stage::QuarterFinal, 1, 1, 1, 1), 8);
    }

    #[test]
    fn ko_draw_after_extra_time_difference() {
        // Actual 2:2, bet 1:1 → DIFFERENCE in QF phase = 6
        assert_eq!(calculate_match_points(Stage::QuarterFinal, 2, 2, 1, 1), 6);
    }

    // ─── Legacy max_potential_points ───────────────────────────────────────

    #[test]
    fn max_potential_group() {
        assert_eq!(max_potential_points(Stage::Group), 4);
    }
    #[test]
    fn max_potential_r32() {
        assert_eq!(max_potential_points(Stage::RoundOf32), 6);
    }
    #[test]
    fn max_potential_r16() {
        assert_eq!(max_potential_points(Stage::RoundOf16), 6);
    }
    #[test]
    fn max_potential_qf() {
        assert_eq!(max_potential_points(Stage::QuarterFinal), 8);
    }
    #[test]
    fn max_potential_sf() {
        assert_eq!(max_potential_points(Stage::SemiFinal), 8);
    }
    #[test]
    fn max_potential_third_place() {
        assert_eq!(max_potential_points(Stage::ThirdPlace), 11);
    }
    #[test]
    fn max_potential_final() {
        assert_eq!(max_potential_points(Stage::Final), 11);
    }

    // ─── Champion ──────────────────────────────────────────────────────────

    #[test]
    fn champion_correct() {
        assert_eq!(champion_points(Some(7), Some(7)), 10);
    }

    #[test]
    fn champion_wrong() {
        assert_eq!(champion_points(Some(7), Some(8)), 0);
    }

    #[test]
    fn champion_unset() {
        assert_eq!(champion_points(None, Some(7)), 0);
        assert_eq!(champion_points(Some(7), None), 0);
        assert_eq!(champion_points(None, None), 0);
    }

    // ─── Edge Cases ────────────────────────────────────────────────────────

    #[test]
    fn zero_zero_exact() {
        assert_eq!(calculate_match_points(Stage::Group, 0, 0, 0, 0), 4);
    }

    #[test]
    fn high_score_exact() {
        assert_eq!(calculate_match_points(Stage::Final, 7, 5, 7, 5), 11);
    }

    #[test]
    fn high_score_tendency() {
        // Actual 7:5 (diff +2), bet 3:1 (diff +2) → same diff → DIFFERENCE, not tendency!
        assert_eq!(calculate_match_points(Stage::Final, 7, 5, 3, 1), 8);
    }

    #[test]
    fn high_score_tendency_different_diff() {
        // Actual 7:5 (diff +2), bet 2:0 (diff +2) → same diff → DIFFERENCE
        // Actual 7:5 (diff +2), bet 2:1 (diff +1) → same sign → TENDENCY
        assert_eq!(calculate_match_points(Stage::Final, 7, 5, 2, 1), 6);
    }

    #[test]
    fn away_win_tendency_in_r32() {
        // Actual 0:2 (diff -2), bet 1:3 (diff -2) → same diff → DIFFERENCE = 4
        assert_eq!(calculate_match_points(Stage::RoundOf32, 0, 2, 1, 3), 4);
    }
}
