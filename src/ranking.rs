// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Standings ranking shared by the leaderboard and the AI matchday reports.
//!
//! Both surfaces order players by total points, so they must agree on how ties
//! are ranked — otherwise a recap can claim a player sits at rank 6 while the
//! table shows rank 8. We use **standard competition ranking** ("1224"):
//! players with equal points share the same rank, and the next lower score is
//! pushed down by the number of players tied above it. For scores
//! `[10, 10, 8, 5]` the ranks are `1, 1, 3, 4` — there is no rank 2, because two
//! players occupy rank 1.

/// 1-based standard competition rank of a player holding `points`, given every
/// score in the standings (`all_points`, the player's own score included).
///
/// The rank is one plus the number of players with *strictly more* points, so
/// ties share a rank and the following rank skips accordingly. Returns `1` when
/// no one scores higher (including the empty-standings case, though callers
/// normally pass a slice that contains `points`).
pub fn competition_rank(points: i32, all_points: &[i32]) -> i32 {
    1 + all_points.iter().filter(|&&p| p > points).count() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_ties_yields_consecutive_ranks() {
        let pts = [10, 8, 5, 3];
        assert_eq!(competition_rank(10, &pts), 1);
        assert_eq!(competition_rank(8, &pts), 2);
        assert_eq!(competition_rank(5, &pts), 3);
        assert_eq!(competition_rank(3, &pts), 4);
    }

    #[test]
    fn tie_shares_rank_and_skips_the_next() {
        // Two players tied on 10 both rank 1; there is no rank 2, the next
        // distinct score is rank 3.
        let pts = [10, 10, 8, 5];
        assert_eq!(competition_rank(10, &pts), 1);
        assert_eq!(competition_rank(8, &pts), 3);
        assert_eq!(competition_rank(5, &pts), 4);
    }

    #[test]
    fn tie_in_the_middle_skips_correctly() {
        // The user's reported scenario: a pair tied for second leaves no rank 3.
        let pts = [20, 15, 15, 12, 9];
        assert_eq!(competition_rank(20, &pts), 1);
        assert_eq!(competition_rank(15, &pts), 2);
        assert_eq!(competition_rank(12, &pts), 4); // not 3 — two share rank 2
        assert_eq!(competition_rank(9, &pts), 5);
    }

    #[test]
    fn three_way_tie_skips_two_ranks() {
        let pts = [7, 7, 7, 4];
        assert_eq!(competition_rank(7, &pts), 1);
        assert_eq!(competition_rank(4, &pts), 4); // three share rank 1
    }

    #[test]
    fn all_tied_share_rank_one() {
        let pts = [5, 5, 5, 5];
        for &p in &pts {
            assert_eq!(competition_rank(p, &pts), 1);
        }
    }

    #[test]
    fn last_place_tie_shares_rank() {
        let pts = [10, 6, 6];
        assert_eq!(competition_rank(6, &pts), 2);
    }

    #[test]
    fn order_of_all_points_does_not_matter() {
        let unsorted = [9, 20, 12, 15, 15];
        assert_eq!(competition_rank(12, &unsorted), 4);
        assert_eq!(competition_rank(15, &unsorted), 2);
    }

    #[test]
    fn negative_and_zero_scores_rank_normally() {
        let pts = [3, 0, 0, -2];
        assert_eq!(competition_rank(3, &pts), 1);
        assert_eq!(competition_rank(0, &pts), 2);
        assert_eq!(competition_rank(-2, &pts), 4);
    }

    #[test]
    fn single_player_is_rank_one() {
        assert_eq!(competition_rank(42, &[42]), 1);
    }
}
