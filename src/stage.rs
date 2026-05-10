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
    pub fn label_de(&self) -> &'static str {
        match self {
            Stage::Group => "Gruppenphase",
            Stage::RoundOf32 => "Sechzehntelfinale",
            Stage::RoundOf16 => "Achtelfinale",
            Stage::QuarterFinal => "Viertelfinale",
            Stage::SemiFinal => "Halbfinale",
            Stage::ThirdPlace => "Spiel um Platz 3",
            Stage::Final => "Finale",
        }
    }

    pub fn short_label_de(&self) -> &'static str {
        match self {
            Stage::Group => "Gruppe",
            Stage::RoundOf32 => "1/16",
            Stage::RoundOf16 => "1/8",
            Stage::QuarterFinal => "1/4",
            Stage::SemiFinal => "1/2",
            Stage::ThirdPlace => "Platz 3",
            Stage::Final => "Finale",
        }
    }

    pub fn multiplier(&self) -> i32 {
        match self {
            Stage::Group => 1,
            Stage::RoundOf32 => 2,
            Stage::RoundOf16 => 3,
            Stage::QuarterFinal => 4,
            Stage::ThirdPlace => 4,
            Stage::SemiFinal => 5,
            Stage::Final => 6,
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

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label_de())
    }
}
