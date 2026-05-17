use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, Default)]
#[sqlx(type_name = "match_stage", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    #[default]
    Group,
    #[sqlx(rename = "round_of_32")]
    #[serde(rename = "round_of_32")]
    RoundOf32,
    #[sqlx(rename = "round_of_16")]
    #[serde(rename = "round_of_16")]
    RoundOf16,
    QuarterFinal,
    SemiFinal,
    ThirdPlace,
    Final,
}

impl Stage {
    pub fn to_tournament_phase(&self) -> crate::scoring::TournamentPhase {
        match self {
            Stage::Group => crate::scoring::TournamentPhase::Group,
            Stage::RoundOf32 => crate::scoring::TournamentPhase::R32,
            Stage::RoundOf16 => crate::scoring::TournamentPhase::R16,
            Stage::QuarterFinal => crate::scoring::TournamentPhase::QF,
            Stage::SemiFinal => crate::scoring::TournamentPhase::SF,
            Stage::ThirdPlace | Stage::Final => crate::scoring::TournamentPhase::Finals,
        }
    }

    pub fn is_knockout(&self) -> bool {
        !matches!(self, Stage::Group)
    }

    /// FTL translation key for this stage.
    pub fn ftl_key(&self) -> &'static str {
        match self {
            Stage::Group => "stage-group",
            Stage::RoundOf32 => "stage-round-of-32",
            Stage::RoundOf16 => "stage-round-of-16",
            Stage::QuarterFinal => "stage-quarter-final",
            Stage::SemiFinal => "stage-semi-final",
            Stage::ThirdPlace => "stage-third-place",
            Stage::Final => "stage-final",
        }
    }
}
