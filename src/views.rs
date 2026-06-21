// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Template-facing view types.
//!
//! These structs are the boundary between handler logic and the Askama
//! templates in `templates/`. Keeping them in their own module makes it
//! easier to evolve the data passed to a view without churning every
//! handler file.

use crate::badges::{EntryBadge, MatchAward};
use crate::jersey::JerseyPreset;
use crate::stage::Stage;
use chrono::{DateTime, Utc};
use std::cmp::Ordering;
use uuid::Uuid;

/// Admin-row projection for the admin panel partial.
pub struct AdminUserView {
    pub id: Uuid,
    pub name: String,
    /// Private real first name, shown only in the admin user list.
    pub real_name: String,
    pub phone_number: Option<String>,
    pub email: Option<String>,
    pub is_admin: bool,
    pub can_create_league: bool,
    pub magic_link: String,
    pub is_self: bool,
}

/// One shareable invite link as shown in the admin user-management page.
pub struct InviteLinkView {
    pub id: Uuid,
    pub label: Option<String>,
    pub invite_link: String,
    pub created_display: String,
}

/// Match cell on the index page — the user's view of one fixture.
pub struct MatchView {
    pub id: i32,
    pub stage: Stage,
    pub group_letter: Option<String>,
    pub home_name: String,
    pub away_name: String,
    pub home_flag: String,
    pub away_flag: String,
    pub score_home: Option<i32>,
    pub score_away: Option<i32>,
    pub predicted_home: Option<i32>,
    pub predicted_away: Option<i32>,
    pub prediction_display: Option<String>,
    /// Raw kickoff time, kept for sorting (the template renders
    /// `kickoff_display` instead).
    pub kickoff_time: Option<DateTime<Utc>>,
    pub kickoff_display: String,
    pub locked: bool,
    pub is_live: bool,
    pub is_finished: bool,
    pub own_points: Option<i32>,
    pub max_phase_points: i32,
    pub winner_only_mode: bool,
    pub allow_draw_prediction: bool,
    pub other_preds: Vec<UserPrediction>,
    /// Match-level award the current user earned on this finished match.
    pub own_award: Option<MatchAward>,
}

impl MatchView {
    pub fn predicted_str(&self) -> String {
        self.prediction_display
            .clone()
            .unwrap_or_else(|| "–".to_string())
    }
    pub fn score_str(&self) -> String {
        match (self.score_home, self.score_away) {
            (Some(h), Some(a)) => format!("{h} : {a}"),
            _ => "– : –".to_string(),
        }
    }
    pub fn has_prediction(&self) -> bool {
        self.predicted_home.is_some() && self.predicted_away.is_some()
    }
    pub fn predicts_home_win(&self) -> bool {
        matches!(
            (self.predicted_home, self.predicted_away),
            (Some(home), Some(away)) if home > away
        )
    }
    pub fn predicts_draw(&self) -> bool {
        matches!(
            (self.predicted_home, self.predicted_away),
            (Some(home), Some(away)) if home == away
        )
    }
    pub fn predicts_away_win(&self) -> bool {
        matches!(
            (self.predicted_home, self.predicted_away),
            (Some(home), Some(away)) if home < away
        )
    }
}

/// Other user's tip on a locked match.
pub struct UserPrediction {
    pub name: String,
    pub label: String,
    pub points: Option<i32>,
    /// Match-level award this user earned on this finished match, if any.
    pub award: Option<MatchAward>,
}

/// Matches grouped by tournament stage so the template can iterate each
/// stage in order without doing the partitioning itself.
#[derive(Default)]
pub struct StageGroups {
    pub groups: Vec<MatchView>,
    pub round_of_32: Vec<MatchView>,
    pub round_of_16: Vec<MatchView>,
    pub quarter_final: Vec<MatchView>,
    pub semi_final: Vec<MatchView>,
    pub third_place: Vec<MatchView>,
    pub final_: Vec<MatchView>,
}

impl StageGroups {
    pub fn push(&mut self, m: MatchView) {
        match m.stage {
            Stage::Group => self.groups.push(m),
            Stage::RoundOf32 => self.round_of_32.push(m),
            Stage::RoundOf16 => self.round_of_16.push(m),
            Stage::QuarterFinal => self.quarter_final.push(m),
            Stage::SemiFinal => self.semi_final.push(m),
            Stage::ThirdPlace => self.third_place.push(m),
            Stage::Final => self.final_.push(m),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
            && self.round_of_32.is_empty()
            && self.round_of_16.is_empty()
            && self.quarter_final.is_empty()
            && self.semi_final.is_empty()
            && self.third_place.is_empty()
            && self.final_.is_empty()
    }
    pub fn len(&self) -> usize {
        self.groups.len()
            + self.round_of_32.len()
            + self.round_of_16.len()
            + self.quarter_final.len()
            + self.semi_final.len()
            + self.third_place.len()
            + self.final_.len()
    }

    /// Reorder every stage so the most recent kickoff comes first. Used by the
    /// "Current" tab, where live and just-finished matches should sit at the
    /// top; matches without a kickoff time sink to the bottom.
    pub fn sort_recent_first(&mut self) {
        fn recent_first(a: &MatchView, b: &MatchView) -> Ordering {
            match (a.kickoff_time, b.kickoff_time) {
                (Some(x), Some(y)) => y.cmp(&x),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
        }
        self.groups.sort_by(recent_first);
        self.round_of_32.sort_by(recent_first);
        self.round_of_16.sort_by(recent_first);
        self.quarter_final.sort_by(recent_first);
        self.semi_final.sort_by(recent_first);
        self.third_place.sort_by(recent_first);
        self.final_.sort_by(recent_first);
    }
}

/// Champion-dropdown option.
#[derive(Clone)]
pub struct TeamView {
    pub id: i32,
    pub name: String,
}

/// Snapshot of the current user's special prediction.
pub struct SpecialPredictionsView {
    pub champion_id: Option<i32>,
}

impl SpecialPredictionsView {
    pub fn is_champion(&self, team_id: &i32) -> bool {
        self.champion_id == Some(*team_id)
    }
}

/// Row of the "everyone's champion picks" table shown after lock.
pub struct ChampPrediction {
    pub name: String,
    pub team_name: String,
    pub team_flag: String,
    pub points: Option<i32>,
}

/// Computed standings row for one team in one group.
pub struct GroupRow {
    pub team_name: String,
    pub flag: String,
    pub played: i32,
    pub wins: i32,
    pub draws: i32,
    pub losses: i32,
    pub goals_for: i32,
    pub goals_against: i32,
    pub goal_diff: i32,
    pub points: i32,
}

/// One group's standings table.
pub struct GroupStandingsTable {
    pub letter: String,
    pub rows: Vec<GroupRow>,
}

/// Leaderboard row — total points plus the user's chosen jersey colours.
#[derive(Clone)]
pub struct LeaderboardEntry {
    pub id: Uuid,
    pub name: String,
    pub total_points: i32,
    pub max_potential_points: i32,
    pub jersey_body: String,
    pub jersey_accent: String,
    pub jersey_pattern: String,
    /// Achievement badges this user has earned, for the compact row strip.
    /// Empty unless the handler populated it (the dashboard does).
    pub achievements: Vec<EntryBadge>,
}

/// Picker option for the jersey-customisation dialog.
pub struct JerseyOption {
    pub key: String,
    pub preset: JerseyPreset,
    pub display_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn match_at(id: i32, kickoff: Option<DateTime<Utc>>) -> MatchView {
        MatchView {
            id,
            stage: Stage::Group,
            group_letter: None,
            home_name: String::new(),
            away_name: String::new(),
            home_flag: String::new(),
            away_flag: String::new(),
            score_home: None,
            score_away: None,
            predicted_home: None,
            predicted_away: None,
            prediction_display: None,
            kickoff_time: kickoff,
            kickoff_display: String::new(),
            locked: false,
            is_live: false,
            is_finished: false,
            own_points: None,
            max_phase_points: 0,
            winner_only_mode: false,
            allow_draw_prediction: true,
            other_preds: Vec::new(),
            own_award: None,
        }
    }

    #[test]
    fn sort_recent_first_orders_by_kickoff_descending_with_none_last() {
        let t = |h: i64| Some(Utc::now() + chrono::Duration::hours(h));
        let mut sg = StageGroups::default();
        // Pushed out of order, plus one match without a kickoff time.
        sg.push(match_at(1, t(1)));
        sg.push(match_at(2, t(3)));
        sg.push(match_at(3, None));
        sg.push(match_at(4, t(2)));

        sg.sort_recent_first();

        // Latest kickoff first; the kickoff-less match sinks to the bottom.
        let ids: Vec<i32> = sg.groups.iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![2, 4, 1, 3]);
    }
}
