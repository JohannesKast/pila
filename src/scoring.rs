//! Pure Rust scoring engine for the Pila WM Tippspiel.
//! No SQL dependency — easy to test and extend.

use crate::stage::Stage;

/// Points for a single match prediction (Kicktipp-style):
/// - Exact score: 4 × stage multiplier
/// - Correct goal difference (and not exact): 2 × multiplier
/// - Correct tendency only (winner / draw): 1 × multiplier
/// - Otherwise: 0
///
/// Score reflects the result including extra time (before penalty shootout).
pub fn calculate_match_points(
    stage: Stage,
    actual_h: i32,
    actual_a: i32,
    pred_h: i32,
    pred_a: i32,
) -> i32 {
    let base = base_points(actual_h, actual_a, pred_h, pred_a);
    base * stage.multiplier()
}

fn base_points(actual_h: i32, actual_a: i32, pred_h: i32, pred_a: i32) -> i32 {
    if pred_h == actual_h && pred_a == actual_a {
        return 4;
    }

    let actual_diff = actual_h - actual_a;
    let pred_diff = pred_h - pred_a;

    if actual_diff == pred_diff {
        return 2;
    }

    if actual_diff.signum() == pred_diff.signum() {
        return 1;
    }

    0
}

/// Maximum points still achievable when the match has not started yet
/// (or has not finished). For an unfinished match the user can still score
/// the exact-result points, so this is `4 × multiplier`.
pub fn max_potential_points(stage: Stage) -> i32 {
    4 * stage.multiplier()
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

    #[test]
    fn group_exact_score() {
        assert_eq!(calculate_match_points(Stage::Group, 2, 1, 2, 1), 4);
    }

    #[test]
    fn group_goal_diff() {
        // predicted 2:1, actual 3:2 — same diff +1 → 2
        assert_eq!(calculate_match_points(Stage::Group, 3, 2, 2, 1), 2);
    }

    #[test]
    fn group_tendency_only() {
        // predicted 2:0, actual 4:1 — both home wins, different diff → 1
        assert_eq!(calculate_match_points(Stage::Group, 4, 1, 2, 0), 1);
    }

    #[test]
    fn group_wrong_tendency() {
        // predicted home win 2:1, actual away win 1:2 → 0
        assert_eq!(calculate_match_points(Stage::Group, 1, 2, 2, 1), 0);
    }

    #[test]
    fn group_draw_exact() {
        assert_eq!(calculate_match_points(Stage::Group, 1, 1, 1, 1), 4);
    }

    #[test]
    fn group_draw_diff_match() {
        // predicted 1:1, actual 2:2 — diff match (0=0) but draws are exact-only;
        // actually goal-diff 0 == 0, so 2 points.
        assert_eq!(calculate_match_points(Stage::Group, 2, 2, 1, 1), 2);
    }

    #[test]
    fn group_predicted_draw_actual_win() {
        // predicted 1:1, actual 2:1 → 0 (no tendency match)
        assert_eq!(calculate_match_points(Stage::Group, 2, 1, 1, 1), 0);
    }

    #[test]
    fn group_predicted_win_actual_draw() {
        // predicted 2:1, actual 1:1 → 0
        assert_eq!(calculate_match_points(Stage::Group, 1, 1, 2, 1), 0);
    }

    #[test]
    fn round_of_32_multiplier() {
        // exact 4 × 2 = 8
        assert_eq!(calculate_match_points(Stage::RoundOf32, 1, 0, 1, 0), 8);
    }

    #[test]
    fn round_of_16_multiplier() {
        assert_eq!(calculate_match_points(Stage::RoundOf16, 2, 1, 2, 1), 12);
    }

    #[test]
    fn quarter_final_multiplier() {
        assert_eq!(calculate_match_points(Stage::QuarterFinal, 3, 0, 3, 0), 16);
    }

    #[test]
    fn semi_final_multiplier() {
        assert_eq!(calculate_match_points(Stage::SemiFinal, 1, 0, 1, 0), 20);
    }

    #[test]
    fn third_place_multiplier() {
        assert_eq!(calculate_match_points(Stage::ThirdPlace, 2, 1, 2, 1), 16);
    }

    #[test]
    fn final_exact() {
        // 4 × 6 = 24
        assert_eq!(calculate_match_points(Stage::Final, 2, 1, 2, 1), 24);
    }

    #[test]
    fn final_tendency_only() {
        // 1 × 6 = 6
        assert_eq!(calculate_match_points(Stage::Final, 5, 0, 2, 1), 6);
    }

    #[test]
    fn ko_draw_after_extra_time_is_valid() {
        // K.O. matches can end in a draw (before penalty shootout)
        assert_eq!(calculate_match_points(Stage::QuarterFinal, 1, 1, 1, 1), 16);
    }

    #[test]
    fn max_potential_group() {
        assert_eq!(max_potential_points(Stage::Group), 4);
    }

    #[test]
    fn max_potential_final() {
        assert_eq!(max_potential_points(Stage::Final), 24);
    }

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
}
