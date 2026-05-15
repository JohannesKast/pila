use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

/// Jersey variant — used for grouping in the picker UI.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JerseyVariant {
    #[default]
    Home,
    Away,
    Fan,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JerseyPreset {
    pub body: String,
    pub accent: String,
    pub pattern: String,
    pub name: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub variant: JerseyVariant,
}

#[derive(Debug, Deserialize)]
struct JerseysFile {
    presets: HashMap<String, JerseyPreset>,
}

const JERSEYS_JSON: &str = r##"{
  "presets": {
    "pila":               { "body": "#74ff8c", "accent": "#0a0f0c", "pattern": "solid",    "name": "Pila Home",           "group": "Pila",     "variant": "home" },
    "pila-away":          { "body": "#ffe600", "accent": "#0a0f0c", "pattern": "solid",    "name": "Pila Away",           "group": "Pila",     "variant": "away" },
    "pila-third":         { "body": "#0a0f0c", "accent": "#74ff8c", "pattern": "solid",    "name": "Pila Third",          "group": "Pila",     "variant": "fan"  },
    "classic":            { "body": "#f3f3ed", "accent": "#222222", "pattern": "solid",    "name": "Klassik",             "group": "Pila",     "variant": "home" },

    "argentinien":        { "body": "#75aadb", "accent": "#f3f3ed", "pattern": "vstripes", "name": "Argentinien",         "group": "CONMEBOL", "variant": "home" },
    "argentinien-away":   { "body": "#1a1a2e", "accent": "#75aadb", "pattern": "hoops",    "name": "Argentinien Away",    "group": "CONMEBOL", "variant": "away" },
    "brasilien":          { "body": "#ffdf00", "accent": "#009c3b", "pattern": "solid",    "name": "Brasilien",           "group": "CONMEBOL", "variant": "home" },
    "brasilien-away":     { "body": "#1a1a2e", "accent": "#ffdf00", "pattern": "solid",    "name": "Brasilien Away",      "group": "CONMEBOL", "variant": "away" },
    "uruguay":            { "body": "#5badd9", "accent": "#f3f3ed", "pattern": "solid",    "name": "Uruguay",             "group": "CONMEBOL", "variant": "home" },
    "uruguay-away":       { "body": "#1a1a2e", "accent": "#5badd9", "pattern": "vstripes", "name": "Uruguay Away",        "group": "CONMEBOL", "variant": "away" },
    "kolumbien":          { "body": "#fcd116", "accent": "#003893", "pattern": "solid",    "name": "Kolumbien",           "group": "CONMEBOL", "variant": "home" },
    "kolumbien-away":     { "body": "#003893", "accent": "#fcd116", "pattern": "solid",    "name": "Kolumbien Away",      "group": "CONMEBOL", "variant": "away" },
    "ecuador":            { "body": "#fcd116", "accent": "#ce1126", "pattern": "sash",     "name": "Ecuador",             "group": "CONMEBOL", "variant": "home" },
    "ecuador-away":       { "body": "#1a1a2e", "accent": "#fcd116", "pattern": "vstripes", "name": "Ecuador Away",        "group": "CONMEBOL", "variant": "away" },
    "peru":               { "body": "#f3f3ed", "accent": "#d91023", "pattern": "sash",     "name": "Peru",                "group": "CONMEBOL", "variant": "home" },
    "peru-away":          { "body": "#d91023", "accent": "#f3f3ed", "pattern": "solid",    "name": "Peru Away",           "group": "CONMEBOL", "variant": "away" },
    "paraguay":           { "body": "#dc2626", "accent": "#f3f3ed", "pattern": "vstripes", "name": "Paraguay",            "group": "CONMEBOL", "variant": "home" },
    "paraguay-away":      { "body": "#f3f3ed", "accent": "#dc2626", "pattern": "vstripes", "name": "Paraguay Away",       "group": "CONMEBOL", "variant": "away" },
    "chile":              { "body": "#dc2626", "accent": "#f3f3ed", "pattern": "solid",    "name": "Chile",               "group": "CONMEBOL", "variant": "home" },
    "chile-away":         { "body": "#1a1a2e", "accent": "#dc2626", "pattern": "halves",   "name": "Chile Away",          "group": "CONMEBOL", "variant": "away" },
    "bolivien":           { "body": "#1d6f3f", "accent": "#fcd116", "pattern": "solid",    "name": "Bolivien",            "group": "CONMEBOL", "variant": "home" },
    "bolivien-away":      { "body": "#fcd116", "accent": "#1d6f3f", "pattern": "solid",    "name": "Bolivien Away",       "group": "CONMEBOL", "variant": "away" },
    "venezuela":          { "body": "#7c1d1d", "accent": "#fcd116", "pattern": "solid",    "name": "Venezuela",           "group": "CONMEBOL", "variant": "home" },
    "venezuela-away":     { "body": "#f3f3ed", "accent": "#7c1d1d", "pattern": "vstripes", "name": "Venezuela Away",      "group": "CONMEBOL", "variant": "away" },

    "usa":                { "body": "#f3f3ed", "accent": "#1d4ed8", "pattern": "sash",     "name": "USA",                 "group": "CONCACAF", "variant": "home" },
    "usa-away":           { "body": "#1d4ed8", "accent": "#f3f3ed", "pattern": "solid",    "name": "USA Away",            "group": "CONCACAF", "variant": "away" },
    "mexiko":             { "body": "#1f8a3b", "accent": "#f3f3ed", "pattern": "solid",    "name": "Mexiko",              "group": "CONCACAF", "variant": "home" },
    "mexiko-away":        { "body": "#f3f3ed", "accent": "#1f8a3b", "pattern": "vstripes", "name": "Mexiko Away",         "group": "CONCACAF", "variant": "away" },
    "kanada":             { "body": "#dc2626", "accent": "#f3f3ed", "pattern": "solid",    "name": "Kanada",              "group": "CONCACAF", "variant": "home" },
    "kanada-away":        { "body": "#f3f3ed", "accent": "#dc2626", "pattern": "solid",    "name": "Kanada Away",         "group": "CONCACAF", "variant": "away" },
    "costarica":          { "body": "#dc2626", "accent": "#f3f3ed", "pattern": "vbands",   "name": "Costa Rica",          "group": "CONCACAF", "variant": "home" },
    "costarica-away":     { "body": "#1a1a2e", "accent": "#dc2626", "pattern": "solid",    "name": "Costa Rica Away",     "group": "CONCACAF", "variant": "away" },
    "panama":             { "body": "#dc2626", "accent": "#0033a0", "pattern": "solid",    "name": "Panama",              "group": "CONCACAF", "variant": "home" },
    "panama-away":        { "body": "#0033a0", "accent": "#dc2626", "pattern": "halves",   "name": "Panama Away",         "group": "CONCACAF", "variant": "away" },
    "honduras":           { "body": "#0073cf", "accent": "#f3f3ed", "pattern": "solid",    "name": "Honduras",            "group": "CONCACAF", "variant": "home" },
    "honduras-away":      { "body": "#f3f3ed", "accent": "#0073cf", "pattern": "vstripes", "name": "Honduras Away",       "group": "CONCACAF", "variant": "away" },
    "jamaika":            { "body": "#fcd116", "accent": "#1f8a3b", "pattern": "solid",    "name": "Jamaika",             "group": "CONCACAF", "variant": "home" },
    "jamaika-away":       { "body": "#1f8a3b", "accent": "#fcd116", "pattern": "vstripes", "name": "Jamaika Away",        "group": "CONCACAF", "variant": "away" },

    "deutschland":        { "body": "#f3f3ed", "accent": "#0a0f0c", "pattern": "hoops",    "name": "Deutschland",         "group": "UEFA",     "variant": "home" },
    "deutschland-away":   { "body": "#1a1a2e", "accent": "#dc2626", "pattern": "solid",    "name": "Deutschland Away",    "group": "UEFA",     "variant": "away" },
    "frankreich":         { "body": "#0055a4", "accent": "#ef4135", "pattern": "solid",    "name": "Frankreich",          "group": "UEFA",     "variant": "home" },
    "frankreich-away":    { "body": "#f3f3ed", "accent": "#0055a4", "pattern": "vstripes", "name": "Frankreich Away",     "group": "UEFA",     "variant": "away" },
    "england":            { "body": "#f3f3ed", "accent": "#dc2626", "pattern": "solid",    "name": "England",             "group": "UEFA",     "variant": "home" },
    "england-away":       { "body": "#dc2626", "accent": "#f3f3ed", "pattern": "solid",    "name": "England Away",        "group": "UEFA",     "variant": "away" },
    "spanien":            { "body": "#c8102e", "accent": "#ffd100", "pattern": "solid",    "name": "Spanien",             "group": "UEFA",     "variant": "home" },
    "spanien-away":       { "body": "#1a1a2e", "accent": "#c8102e", "pattern": "halves",   "name": "Spanien Away",        "group": "UEFA",     "variant": "away" },
    "portugal":           { "body": "#a30b0b", "accent": "#1f8a3b", "pattern": "halves",   "name": "Portugal",            "group": "UEFA",     "variant": "home" },
    "portugal-away":      { "body": "#1f8a3b", "accent": "#a30b0b", "pattern": "solid",    "name": "Portugal Away",       "group": "UEFA",     "variant": "away" },
    "niederlande":        { "body": "#ff6f00", "accent": "#0a0f0c", "pattern": "solid",    "name": "Niederlande",         "group": "UEFA",     "variant": "home" },
    "niederlande-away":   { "body": "#1a1a2e", "accent": "#ff6f00", "pattern": "vstripes", "name": "Niederlande Away",    "group": "UEFA",     "variant": "away" },
    "belgien":            { "body": "#a40026", "accent": "#fae042", "pattern": "solid",    "name": "Belgien",             "group": "UEFA",     "variant": "home" },
    "belgien-away":       { "body": "#fae042", "accent": "#a40026", "pattern": "solid",    "name": "Belgien Away",        "group": "UEFA",     "variant": "away" },
    "italien":            { "body": "#1d4ed8", "accent": "#ffd700", "pattern": "solid",    "name": "Italien",             "group": "UEFA",     "variant": "home" },
    "italien-away":       { "body": "#f3f3ed", "accent": "#1d4ed8", "pattern": "vstripes", "name": "Italien Away",        "group": "UEFA",     "variant": "away" },
    "kroatien":           { "body": "#f3f3ed", "accent": "#dc2626", "pattern": "check",    "name": "Kroatien",            "group": "UEFA",     "variant": "home" },
    "kroatien-away":      { "body": "#1a1a2e", "accent": "#dc2626", "pattern": "check",    "name": "Kroatien Away",       "group": "UEFA",     "variant": "away" },
    "schweiz":            { "body": "#dc2626", "accent": "#f3f3ed", "pattern": "solid",    "name": "Schweiz",             "group": "UEFA",     "variant": "home" },
    "schweiz-away":       { "body": "#f3f3ed", "accent": "#dc2626", "pattern": "halves",   "name": "Schweiz Away",        "group": "UEFA",     "variant": "away" },
    "daenemark":          { "body": "#c8102e", "accent": "#f3f3ed", "pattern": "solid",    "name": "Dänemark",            "group": "UEFA",     "variant": "home" },
    "daenemark-away":     { "body": "#f3f3ed", "accent": "#c8102e", "pattern": "vstripes", "name": "Dänemark Away",       "group": "UEFA",     "variant": "away" },
    "polen":              { "body": "#f3f3ed", "accent": "#dc143c", "pattern": "solid",    "name": "Polen",               "group": "UEFA",     "variant": "home" },
    "polen-away":         { "body": "#dc143c", "accent": "#f3f3ed", "pattern": "solid",    "name": "Polen Away",          "group": "UEFA",     "variant": "away" },
    "oesterreich":        { "body": "#dc2626", "accent": "#f3f3ed", "pattern": "vbands",   "name": "Österreich",          "group": "UEFA",     "variant": "home" },
    "oesterreich-away":   { "body": "#f3f3ed", "accent": "#dc2626", "pattern": "vbands",   "name": "Österreich Away",     "group": "UEFA",     "variant": "away" },
    "ungarn":             { "body": "#cd2a3e", "accent": "#436f4d", "pattern": "solid",    "name": "Ungarn",              "group": "UEFA",     "variant": "home" },
    "ungarn-away":        { "body": "#f3f3ed", "accent": "#cd2a3e", "pattern": "vstripes", "name": "Ungarn Away",         "group": "UEFA",     "variant": "away" },
    "tuerkei":            { "body": "#e30a17", "accent": "#f3f3ed", "pattern": "solid",    "name": "Türkei",              "group": "UEFA",     "variant": "home" },
    "tuerkei-away":       { "body": "#f3f3ed", "accent": "#e30a17", "pattern": "halves",   "name": "Türkei Away",         "group": "UEFA",     "variant": "away" },
    "serbien":            { "body": "#c6363c", "accent": "#0c4076", "pattern": "solid",    "name": "Serbien",             "group": "UEFA",     "variant": "home" },
    "serbien-away":       { "body": "#0c4076", "accent": "#c6363c", "pattern": "solid",    "name": "Serbien Away",        "group": "UEFA",     "variant": "away" },
    "ukraine":            { "body": "#fcd116", "accent": "#0057b8", "pattern": "solid",    "name": "Ukraine",             "group": "UEFA",     "variant": "home" },
    "ukraine-away":       { "body": "#0057b8", "accent": "#fcd116", "pattern": "vstripes", "name": "Ukraine Away",        "group": "UEFA",     "variant": "away" },
    "wales":              { "body": "#bb0000", "accent": "#1f8a3b", "pattern": "solid",    "name": "Wales",               "group": "UEFA",     "variant": "home" },
    "wales-away":         { "body": "#1f8a3b", "accent": "#bb0000", "pattern": "solid",    "name": "Wales Away",          "group": "UEFA",     "variant": "away" },
    "schottland":         { "body": "#0d3b66", "accent": "#f3f3ed", "pattern": "solid",    "name": "Schottland",          "group": "UEFA",     "variant": "home" },
    "schottland-away":    { "body": "#f3f3ed", "accent": "#0d3b66", "pattern": "vstripes", "name": "Schottland Away",     "group": "UEFA",     "variant": "away" },
    "norwegen":           { "body": "#ba0c2f", "accent": "#f3f3ed", "pattern": "solid",    "name": "Norwegen",            "group": "UEFA",     "variant": "home" },
    "norwegen-away":      { "body": "#f3f3ed", "accent": "#ba0c2f", "pattern": "halves",   "name": "Norwegen Away",       "group": "UEFA",     "variant": "away" },
    "schweden":           { "body": "#fecc02", "accent": "#005293", "pattern": "solid",    "name": "Schweden",            "group": "UEFA",     "variant": "home" },
    "schweden-away":      { "body": "#005293", "accent": "#fecc02", "pattern": "vstripes", "name": "Schweden Away",       "group": "UEFA",     "variant": "away" },

    "japan":              { "body": "#13205d", "accent": "#f3f3ed", "pattern": "solid",    "name": "Japan",               "group": "AFC",      "variant": "home" },
    "japan-away":         { "body": "#f3f3ed", "accent": "#13205d", "pattern": "solid",    "name": "Japan Away",          "group": "AFC",      "variant": "away" },
    "suedkorea":          { "body": "#dc2626", "accent": "#0a0f0c", "pattern": "solid",    "name": "Südkorea",            "group": "AFC",      "variant": "home" },
    "suedkorea-away":     { "body": "#1a1a2e", "accent": "#dc2626", "pattern": "vstripes", "name": "Südkorea Away",       "group": "AFC",      "variant": "away" },
    "australien":         { "body": "#fde047", "accent": "#1f8a3b", "pattern": "solid",    "name": "Australien",          "group": "AFC",      "variant": "home" },
    "australien-away":    { "body": "#1f8a3b", "accent": "#fde047", "pattern": "solid",    "name": "Australien Away",     "group": "AFC",      "variant": "away" },
    "iran":               { "body": "#f3f3ed", "accent": "#239f40", "pattern": "solid",    "name": "Iran",                "group": "AFC",      "variant": "home" },
    "iran-away":          { "body": "#239f40", "accent": "#f3f3ed", "pattern": "vstripes", "name": "Iran Away",           "group": "AFC",      "variant": "away" },
    "saudi":              { "body": "#f3f3ed", "accent": "#006c35", "pattern": "solid",    "name": "Saudi-Arabien",       "group": "AFC",      "variant": "home" },
    "saudi-away":         { "body": "#006c35", "accent": "#f3f3ed", "pattern": "solid",    "name": "Saudi-Arabien Away",  "group": "AFC",      "variant": "away" },
    "katar":              { "body": "#7a1432", "accent": "#f3f3ed", "pattern": "solid",    "name": "Katar",               "group": "AFC",      "variant": "home" },
    "katar-away":         { "body": "#f3f3ed", "accent": "#7a1432", "pattern": "halves",   "name": "Katar Away",          "group": "AFC",      "variant": "away" },
    "irak":               { "body": "#1f8a3b", "accent": "#f3f3ed", "pattern": "solid",    "name": "Irak",                "group": "AFC",      "variant": "home" },
    "irak-away":          { "body": "#f3f3ed", "accent": "#1f8a3b", "pattern": "vstripes", "name": "Irak Away",           "group": "AFC",      "variant": "away" },
    "usbekistan":         { "body": "#f3f3ed", "accent": "#0099b5", "pattern": "solid",    "name": "Usbekistan",          "group": "AFC",      "variant": "home" },
    "usbekistan-away":    { "body": "#0099b5", "accent": "#f3f3ed", "pattern": "solid",    "name": "Usbekistan Away",     "group": "AFC",      "variant": "away" },

    "marokko":            { "body": "#c1272d", "accent": "#1f8a3b", "pattern": "solid",    "name": "Marokko",             "group": "CAF",      "variant": "home" },
    "marokko-away":       { "body": "#1f8a3b", "accent": "#c1272d", "pattern": "solid",    "name": "Marokko Away",        "group": "CAF",      "variant": "away" },
    "tunesien":           { "body": "#e70013", "accent": "#f3f3ed", "pattern": "solid",    "name": "Tunesien",            "group": "CAF",      "variant": "home" },
    "tunesien-away":      { "body": "#f3f3ed", "accent": "#e70013", "pattern": "vstripes", "name": "Tunesien Away",       "group": "CAF",      "variant": "away" },
    "algerien":           { "body": "#f3f3ed", "accent": "#006233", "pattern": "halves",   "name": "Algerien",            "group": "CAF",      "variant": "home" },
    "algerien-away":      { "body": "#006233", "accent": "#f3f3ed", "pattern": "solid",    "name": "Algerien Away",       "group": "CAF",      "variant": "away" },
    "aegypten":           { "body": "#cf0921", "accent": "#0a0f0c", "pattern": "solid",    "name": "Ägypten",             "group": "CAF",      "variant": "home" },
    "aegypten-away":      { "body": "#0a0f0c", "accent": "#cf0921", "pattern": "vstripes", "name": "Ägypten Away",        "group": "CAF",      "variant": "away" },
    "senegal":            { "body": "#f3f3ed", "accent": "#1f8a3b", "pattern": "solid",    "name": "Senegal",             "group": "CAF",      "variant": "home" },
    "senegal-away":       { "body": "#1f8a3b", "accent": "#f3f3ed", "pattern": "halves",   "name": "Senegal Away",        "group": "CAF",      "variant": "away" },
    "kamerun":            { "body": "#1f8a3b", "accent": "#fcd116", "pattern": "solid",    "name": "Kamerun",             "group": "CAF",      "variant": "home" },
    "kamerun-away":       { "body": "#fcd116", "accent": "#1f8a3b", "pattern": "vstripes", "name": "Kamerun Away",        "group": "CAF",      "variant": "away" },
    "nigeria":            { "body": "#1f8a3b", "accent": "#f3f3ed", "pattern": "vstripes", "name": "Nigeria",             "group": "CAF",      "variant": "home" },
    "nigeria-away":       { "body": "#f3f3ed", "accent": "#1f8a3b", "pattern": "solid",    "name": "Nigeria Away",        "group": "CAF",      "variant": "away" },
    "ghana":              { "body": "#f3f3ed", "accent": "#dc143c", "pattern": "sash",     "name": "Ghana",               "group": "CAF",      "variant": "home" },
    "ghana-away":         { "body": "#1a1a2e", "accent": "#fcd116", "pattern": "vstripes", "name": "Ghana Away",          "group": "CAF",      "variant": "away" },
    "elfenbein":          { "body": "#ff8200", "accent": "#1f8a3b", "pattern": "solid",    "name": "Elfenbeinküste",      "group": "CAF",      "variant": "home" },
    "elfenbein-away":     { "body": "#1f8a3b", "accent": "#ff8200", "pattern": "solid",    "name": "Elfenbeinküste Away", "group": "CAF",      "variant": "away" },
    "suedafrika":         { "body": "#fcd116", "accent": "#1f8a3b", "pattern": "solid",    "name": "Südafrika",           "group": "CAF",      "variant": "home" },
    "suedafrika-away":    { "body": "#1f8a3b", "accent": "#fcd116", "pattern": "vstripes", "name": "Südafrika Away",      "group": "CAF",      "variant": "away" },

    "fan-rainbow":        { "body": "#e40303", "accent": "#ff8c00", "pattern": "hoops",    "name": "Rainbow",             "group": "Fan",      "variant": "fan"  },
    "fan-retro-70s":      { "body": "#d4a574", "accent": "#5c3a1e", "pattern": "hoops",    "name": "Retro 70s",           "group": "Fan",      "variant": "fan"  },
    "fan-retro-86":       { "body": "#1a1a2e", "accent": "#ffd700", "pattern": "vstripes", "name": "Retro '86",           "group": "Fan",      "variant": "fan"  },
    "fan-retro-90s":      { "body": "#00b4d8", "accent": "#ff6b6b", "pattern": "halves",   "name": "Retro 90s",           "group": "Fan",      "variant": "fan"  },
    "fan-neon":           { "body": "#39ff14", "accent": "#ff073a", "pattern": "vstripes", "name": "Neon Fever",          "group": "Fan",      "variant": "fan"  },
    "fan-carnival":       { "body": "#ffd700", "accent": "#ff073a", "pattern": "vstripes", "name": "Karneval",            "group": "Fan",      "variant": "fan"  },
    "fan-military":       { "body": "#4b5320", "accent": "#2d2d1e", "pattern": "solid",    "name": "Military",            "group": "Fan",      "variant": "fan"  },
    "fan-gold":           { "body": "#ffd700", "accent": "#0a0f0c", "pattern": "solid",    "name": "Gold Edition",        "group": "Fan",      "variant": "fan"  },
    "fan-midnight":       { "body": "#0a0f0c", "accent": "#1d4ed8", "pattern": "vstripes", "name": "Mitternacht",         "group": "Fan",      "variant": "fan"  },
    "fan-sunset":         { "body": "#ff6b35", "accent": "#ffc300", "pattern": "sash",     "name": "Sonnenuntergang",     "group": "Fan",      "variant": "fan"  },
    "fan-arctic":         { "body": "#a8dadc", "accent": "#1d3557", "pattern": "hoops",    "name": "Arktis",              "group": "Fan",      "variant": "fan"  },
    "fan-flame":          { "body": "#ff073a", "accent": "#ff6b35", "pattern": "hoops",    "name": "Flamme",              "group": "Fan",      "variant": "fan"  },
    "fan-forest":         { "body": "#1b4332", "accent": "#40916c", "pattern": "vstripes", "name": "Wald",                "group": "Fan",      "variant": "fan"  },
    "fan-royal":          { "body": "#4a0e8f", "accent": "#ffd700", "pattern": "halves",   "name": "Royal",               "group": "Fan",      "variant": "fan"  },
    "fan-pirate":         { "body": "#0a0f0c", "accent": "#dc2626", "pattern": "sash",     "name": "Pirat",               "group": "Fan",      "variant": "fan"  },
    "fan-galaxy":         { "body": "#0a0f0c", "accent": "#74ff8c", "pattern": "check",    "name": "Galaxie",             "group": "Fan",      "variant": "fan"  },
    "fan-camo":           { "body": "#556b2f", "accent": "#8b7355", "pattern": "check",    "name": "Camo",                "group": "Fan",      "variant": "fan"  }
  }
}"##;

pub fn load() -> Arc<HashMap<String, JerseyPreset>> {
    let parsed: JerseysFile =
        serde_json::from_str(JERSEYS_JSON).expect("jerseys JSON must be valid");
    Arc::new(parsed.presets)
}

pub fn get<'a>(
    presets: &'a HashMap<String, JerseyPreset>,
    key: &str,
) -> &'a JerseyPreset {
    presets
        .get(key)
        .or_else(|| presets.get("classic"))
        .expect("'classic' jersey preset must exist as fallback")
}
