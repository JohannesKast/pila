//! Template-facing view types.
//!
//! These structs are the boundary between handler logic and the Askama
//! templates in `templates/`. Keeping them in their own module makes it
//! easier to evolve the data passed to a view without churning every
//! handler file.

use crate::jersey::JerseyPreset;
use crate::stage::Stage;
use uuid::Uuid;

/// Admin-row projection for the admin panel partial.
pub struct AdminUserView {
    pub id: Uuid,
    pub name: String,
    pub phone_number: Option<String>,
    pub is_admin: bool,
    pub magic_link: String,
    pub is_self: bool,
}

/// Match cell on the index page — the user's view of one fixture.
pub struct MatchView {
    pub id: i32,
    pub stage: Stage,
    pub stage_label: String,
    pub group_letter: Option<String>,
    pub home_name: String,
    pub away_name: String,
    pub home_flag: String,
    pub away_flag: String,
    pub score_home: Option<i32>,
    pub score_away: Option<i32>,
    pub predicted_home: Option<i32>,
    pub predicted_away: Option<i32>,
    pub kickoff_display: String,
    pub locked: bool,
    pub is_live: bool,
    pub is_finished: bool,
    pub own_points: Option<i32>,
    pub multiplier: i32,
    pub other_preds: Vec<UserPrediction>,
}

impl MatchView {
    pub fn predicted_str(&self) -> String {
        match (self.predicted_home, self.predicted_away) {
            (Some(h), Some(a)) => format!("{h}:{a}"),
            _ => "–".to_string(),
        }
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
}

/// Other user's tip on a locked match.
pub struct UserPrediction {
    pub name: String,
    pub home: i32,
    pub away: i32,
    pub points: Option<i32>,
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
    pub name: String,
    pub total_points: i32,
    pub max_potential_points: i32,
    pub jersey_body: String,
    pub jersey_accent: String,
    pub jersey_pattern: String,
}

/// Picker option for the jersey-customisation dialog.
pub struct JerseyOption {
    pub key: String,
    pub preset: JerseyPreset,
}
