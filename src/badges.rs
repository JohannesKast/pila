//! Hero-Panel Badge-System.
//!
//! Badges aggregieren on-the-fly über vorhandene Predictions — sie ändern keine Punkte
//! und werden niemals persistiert. Ein einmal pro Request gebauter `BadgeContext` wird
//! an jeden registrierten Badge durchgereicht, der daraus pure einen `BadgeView` ableitet.
//!
//! Architekturüberblick: siehe CLAUDE.md, Abschnitt „Badges".

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Europe::Berlin;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::scoring;
use crate::stage::Stage;

const UNDERDOG_THRESHOLD: f32 = 0.30;

// ─── Display-Typen ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum BadgeAccent {
    Default,
    Yellow,
    Green,
    Red,
    Blue,
}

impl BadgeAccent {
    /// CSS-Variable für die Card-Akzentfarbe. `Default` mappt auf `--pl-fg`.
    pub fn css_var(&self) -> &'static str {
        match self {
            BadgeAccent::Default => "var(--pl-fg)",
            BadgeAccent::Yellow => "var(--pl-yellow)",
            BadgeAccent::Green => "var(--pl-green)",
            BadgeAccent::Red => "var(--pl-red)",
            BadgeAccent::Blue => "var(--pl-blue)",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BadgeValue {
    Count(i32),
    Fraction { num: i32, denom: i32 },
    Percent(Option<i32>),
    Streak(i32),
    Delta(Option<i32>),
    Champion { team: String, flag_url: String },
    Empty,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BadgeDisplay {
    Metric(BadgeValue),
    Achievement { times_earned: i32 },
}

impl BadgeDisplay {
    // Askama-Helfer (Pattern-Matching in Templates ist umständlich, deswegen Booleans).
    pub fn is_achievement(&self) -> bool {
        matches!(self, BadgeDisplay::Achievement { .. })
    }
    pub fn is_metric(&self) -> bool {
        matches!(self, BadgeDisplay::Metric(_))
    }
    pub fn times_earned(&self) -> i32 {
        match self {
            BadgeDisplay::Achievement { times_earned } => *times_earned,
            _ => 0,
        }
    }
    pub fn metric_kind(&self) -> &'static str {
        match self {
            BadgeDisplay::Metric(BadgeValue::Count(_)) => "count",
            BadgeDisplay::Metric(BadgeValue::Fraction { .. }) => "fraction",
            BadgeDisplay::Metric(BadgeValue::Percent(_)) => "percent",
            BadgeDisplay::Metric(BadgeValue::Streak(_)) => "streak",
            BadgeDisplay::Metric(BadgeValue::Delta(_)) => "delta",
            BadgeDisplay::Metric(BadgeValue::Champion { .. }) => "champion",
            BadgeDisplay::Metric(BadgeValue::Empty) => "empty",
            BadgeDisplay::Achievement { .. } => "achievement",
        }
    }
    pub fn count(&self) -> i32 {
        match self {
            BadgeDisplay::Metric(BadgeValue::Count(n)) => *n,
            _ => 0,
        }
    }
    pub fn fraction_num(&self) -> i32 {
        match self {
            BadgeDisplay::Metric(BadgeValue::Fraction { num, .. }) => *num,
            _ => 0,
        }
    }
    pub fn fraction_denom(&self) -> i32 {
        match self {
            BadgeDisplay::Metric(BadgeValue::Fraction { denom, .. }) => *denom,
            _ => 0,
        }
    }
    pub fn percent(&self) -> Option<i32> {
        match self {
            BadgeDisplay::Metric(BadgeValue::Percent(p)) => *p,
            _ => None,
        }
    }
    pub fn streak(&self) -> i32 {
        match self {
            BadgeDisplay::Metric(BadgeValue::Streak(n)) => *n,
            _ => 0,
        }
    }
    pub fn delta(&self) -> Option<i32> {
        match self {
            BadgeDisplay::Metric(BadgeValue::Delta(d)) => *d,
            _ => None,
        }
    }
    pub fn champion_team(&self) -> &str {
        match self {
            BadgeDisplay::Metric(BadgeValue::Champion { team, .. }) => team.as_str(),
            _ => "",
        }
    }
    pub fn champion_flag(&self) -> &str {
        match self {
            BadgeDisplay::Metric(BadgeValue::Champion { flag_url, .. }) => flag_url.as_str(),
            _ => "",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BadgeView {
    pub key: &'static str,
    pub icon: &'static str,
    pub title: &'static str,
    pub how_to_earn: &'static str,
    pub display: BadgeDisplay,
    pub accent: BadgeAccent,
}

impl BadgeView {
    // Askama-Helper (Enum-Matching im Template ist umständlich).
    pub fn accent_css(&self) -> &'static str {
        self.accent.css_var()
    }
}

// ─── Context ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PredictionRow {
    pub user_id: Uuid,
    pub match_id: i32,
    pub stage: Stage,
    pub kickoff: DateTime<Utc>,
    pub score_h: i32,
    pub score_a: i32,
    pub pred_h: i32,
    pub pred_a: i32,
}

impl PredictionRow {
    pub fn berlin_date(&self) -> NaiveDate {
        self.kickoff.with_timezone(&Berlin).date_naive()
    }
    pub fn base_points(&self) -> i32 {
        scoring::calculate_match_points(
            self.stage,
            self.score_h,
            self.score_a,
            self.pred_h,
            self.pred_a,
        )
    }
    pub fn is_exact(&self) -> bool {
        self.pred_h == self.score_h && self.pred_a == self.score_a
    }
}

#[derive(Debug, Clone)]
pub struct ChampionView {
    pub team_name: String,
    pub flag_url: String,
}

pub struct BadgeContext<'a> {
    pub user_id: Uuid,
    pub now: DateTime<Utc>,
    pub berlin_today: NaiveDate,
    pub finished_predictions: &'a [PredictionRow],
    pub all_user_ids: &'a [Uuid],
    pub started_matches_total: i32,
    pub user_started_tips: i32,
    pub actual_champion_id: Option<i32>,
    pub all_special_picks: &'a [(Uuid, i32)],
    pub user_champion: Option<&'a ChampionView>,
}

/// Owner-Variante mit eigenen Vecs — der HTTP-Handler hält dies, Badges nutzen `as_ctx()`.
#[derive(Debug, Clone)]
pub struct BadgeContextOwned {
    pub user_id: Uuid,
    pub now: DateTime<Utc>,
    pub berlin_today: NaiveDate,
    pub finished_predictions: Vec<PredictionRow>,
    pub all_user_ids: Vec<Uuid>,
    pub started_matches_total: i32,
    pub user_started_tips: i32,
    pub actual_champion_id: Option<i32>,
    pub all_special_picks: Vec<(Uuid, i32)>,
    pub user_champion: Option<ChampionView>,
}

impl BadgeContextOwned {
    pub fn as_ctx(&self) -> BadgeContext<'_> {
        BadgeContext {
            user_id: self.user_id,
            now: self.now,
            berlin_today: self.berlin_today,
            finished_predictions: &self.finished_predictions,
            all_user_ids: &self.all_user_ids,
            started_matches_total: self.started_matches_total,
            user_started_tips: self.user_started_tips,
            actual_champion_id: self.actual_champion_id,
            all_special_picks: &self.all_special_picks,
            user_champion: self.user_champion.as_ref(),
        }
    }
}

// ─── Badge-Trait ──────────────────────────────────────────────────────────────

pub trait Badge: Send + Sync {
    fn key(&self) -> &'static str;
    fn icon(&self) -> &'static str;
    fn title(&self) -> &'static str;
    fn how_to_earn(&self) -> &'static str;
    fn accent(&self) -> BadgeAccent {
        BadgeAccent::Default
    }
    fn compute(&self, ctx: &BadgeContext<'_>) -> BadgeDisplay;
}

pub fn registry() -> Vec<Box<dyn Badge>> {
    vec![
        Box::new(MatchdayWinsBadge),
        Box::new(ExactCountBadge),
        Box::new(UnderdogBadge),
        Box::new(SoloHitBadge),
        Box::new(TendencyPctBadge),
        Box::new(KnockoutPointsBadge),
        Box::new(DisciplinePctBadge),
        Box::new(CurrentStreakBadge),
        Box::new(LongestStreakBadge),
        Box::new(RankDeltaBadge),
        Box::new(ChampionPickBadge),
    ]
}

pub fn compute_all(ctx: &BadgeContext<'_>) -> Vec<BadgeView> {
    registry()
        .iter()
        .map(|b| BadgeView {
            key: b.key(),
            icon: b.icon(),
            title: b.title(),
            how_to_earn: b.how_to_earn(),
            display: b.compute(ctx),
            accent: b.accent(),
        })
        .collect()
}

// ─── Badge-Impls ──────────────────────────────────────────────────────────────

pub struct MatchdayWinsBadge;
impl Badge for MatchdayWinsBadge {
    fn key(&self) -> &'static str {
        "matchday_wins"
    }
    fn icon(&self) -> &'static str {
        "🥇"
    }
    fn title(&self) -> &'static str {
        "Tagessieg"
    }
    fn how_to_earn(&self) -> &'static str {
        "Höchste Tagespunktzahl an einem Spieltag (Berlin-Zeit). Bei Gleichstand teilen sich die Sieger den Tag."
    }
    fn accent(&self) -> BadgeAccent {
        BadgeAccent::Yellow
    }
    fn compute(&self, ctx: &BadgeContext<'_>) -> BadgeDisplay {
        // (date, user_id) → Punkte
        let mut by_day_user: HashMap<(NaiveDate, Uuid), i32> = HashMap::new();
        for r in ctx.finished_predictions {
            *by_day_user.entry((r.berlin_date(), r.user_id)).or_insert(0) += r.base_points();
        }
        // Pro Tag Max bestimmen, dann zählen, wie oft `user_id` Max erreicht (>0).
        let mut day_max: HashMap<NaiveDate, i32> = HashMap::new();
        for (&(day, _), &pts) in &by_day_user {
            day_max.entry(day).and_modify(|m| *m = (*m).max(pts)).or_insert(pts);
        }
        let mut wins = 0;
        for ((day, uid), pts) in &by_day_user {
            if *uid == ctx.user_id && *pts > 0 && *pts == *day_max.get(day).unwrap_or(&0) {
                wins += 1;
            }
        }
        BadgeDisplay::Achievement { times_earned: wins }
    }
}

pub struct ExactCountBadge;
impl Badge for ExactCountBadge {
    fn key(&self) -> &'static str {
        "exact_count"
    }
    fn icon(&self) -> &'static str {
        "🎯"
    }
    fn title(&self) -> &'static str {
        "Exakter Tipp"
    }
    fn how_to_earn(&self) -> &'static str {
        "Tipp 100% korrekt: gleicher Heim- und Auswärtsscore wie das Endergebnis."
    }
    fn accent(&self) -> BadgeAccent {
        BadgeAccent::Yellow
    }
    fn compute(&self, ctx: &BadgeContext<'_>) -> BadgeDisplay {
        let n = ctx
            .finished_predictions
            .iter()
            .filter(|r| r.user_id == ctx.user_id && r.is_exact())
            .count() as i32;
        BadgeDisplay::Achievement { times_earned: n }
    }
}

pub struct UnderdogBadge;
impl Badge for UnderdogBadge {
    fn key(&self) -> &'static str {
        "underdog"
    }
    fn icon(&self) -> &'static str {
        "🐺"
    }
    fn title(&self) -> &'static str {
        "Underdog"
    }
    fn how_to_earn(&self) -> &'static str {
        "Exakter Tipp auf ein Match, das weniger als 30% aller Tipper exakt trafen."
    }
    fn accent(&self) -> BadgeAccent {
        BadgeAccent::Blue
    }
    fn compute(&self, ctx: &BadgeContext<'_>) -> BadgeDisplay {
        // Pro Match: (Anzahl exakter Tipper, hat User exakt)
        let mut by_match: HashMap<i32, (i32, i32, bool)> = HashMap::new();
        for r in ctx.finished_predictions {
            let entry = by_match.entry(r.match_id).or_insert((0, 0, false));
            entry.1 += 1;
            if r.is_exact() {
                entry.0 += 1;
                if r.user_id == ctx.user_id {
                    entry.2 = true;
                }
            }
        }
        let mut hits = 0;
        for (_, (exact_count, total_tippers, user_exact)) in by_match {
            if !user_exact || total_tippers == 0 {
                continue;
            }
            let ratio = exact_count as f32 / total_tippers as f32;
            if ratio < UNDERDOG_THRESHOLD {
                hits += 1;
            }
        }
        BadgeDisplay::Achievement { times_earned: hits }
    }
}

pub struct SoloHitBadge;
impl Badge for SoloHitBadge {
    fn key(&self) -> &'static str {
        "solo_hit"
    }
    fn icon(&self) -> &'static str {
        "💎"
    }
    fn title(&self) -> &'static str {
        "Solo-Treffer"
    }
    fn how_to_earn(&self) -> &'static str {
        "Du warst als einziger Tipper exakt — pro Match einmal vergeben."
    }
    fn accent(&self) -> BadgeAccent {
        BadgeAccent::Green
    }
    fn compute(&self, ctx: &BadgeContext<'_>) -> BadgeDisplay {
        let mut by_match: HashMap<i32, (i32, bool)> = HashMap::new();
        for r in ctx.finished_predictions {
            let entry = by_match.entry(r.match_id).or_insert((0, false));
            if r.is_exact() {
                entry.0 += 1;
                if r.user_id == ctx.user_id {
                    entry.1 = true;
                }
            }
        }
        let solo = by_match
            .values()
            .filter(|(exact_count, user_exact)| *user_exact && *exact_count == 1)
            .count() as i32;
        BadgeDisplay::Achievement { times_earned: solo }
    }
}

pub struct TendencyPctBadge;
impl Badge for TendencyPctBadge {
    fn key(&self) -> &'static str {
        "tendency_pct"
    }
    fn icon(&self) -> &'static str {
        "📈"
    }
    fn title(&self) -> &'static str {
        "Tendenz-Quote"
    }
    fn how_to_earn(&self) -> &'static str {
        "Anteil deiner getippten fertigen Matches mit mindestens einem Punkt."
    }
    fn compute(&self, ctx: &BadgeContext<'_>) -> BadgeDisplay {
        let user_rows: Vec<_> = ctx
            .finished_predictions
            .iter()
            .filter(|r| r.user_id == ctx.user_id)
            .collect();
        if user_rows.is_empty() {
            return BadgeDisplay::Metric(BadgeValue::Percent(None));
        }
        let with_points = user_rows.iter().filter(|r| r.base_points() > 0).count() as i32;
        let pct = (100 * with_points) / user_rows.len() as i32;
        BadgeDisplay::Metric(BadgeValue::Percent(Some(pct)))
    }
}

pub struct KnockoutPointsBadge;
impl Badge for KnockoutPointsBadge {
    fn key(&self) -> &'static str {
        "knockout_points"
    }
    fn icon(&self) -> &'static str {
        "🏆"
    }
    fn title(&self) -> &'static str {
        "K.O.-Punkte"
    }
    fn how_to_earn(&self) -> &'static str {
        "Summe deiner Punkte aus der K.O.-Phase (ab Sechzehntelfinale)."
    }
    fn accent(&self) -> BadgeAccent {
        BadgeAccent::Yellow
    }
    fn compute(&self, ctx: &BadgeContext<'_>) -> BadgeDisplay {
        let any_ko = ctx
            .finished_predictions
            .iter()
            .any(|r| r.stage.is_knockout());
        if !any_ko {
            return BadgeDisplay::Metric(BadgeValue::Empty);
        }
        let pts: i32 = ctx
            .finished_predictions
            .iter()
            .filter(|r| r.user_id == ctx.user_id && r.stage.is_knockout())
            .map(|r| r.base_points())
            .sum();
        BadgeDisplay::Metric(BadgeValue::Count(pts))
    }
}

pub struct DisciplinePctBadge;
impl Badge for DisciplinePctBadge {
    fn key(&self) -> &'static str {
        "discipline_pct"
    }
    fn icon(&self) -> &'static str {
        "📋"
    }
    fn title(&self) -> &'static str {
        "Tippmoral"
    }
    fn how_to_earn(&self) -> &'static str {
        "Anteil bereits gestarteter Matches, für die du einen Tipp abgegeben hast."
    }
    fn compute(&self, ctx: &BadgeContext<'_>) -> BadgeDisplay {
        if ctx.started_matches_total <= 0 {
            return BadgeDisplay::Metric(BadgeValue::Percent(None));
        }
        let pct = (100 * ctx.user_started_tips) / ctx.started_matches_total;
        BadgeDisplay::Metric(BadgeValue::Percent(Some(pct)))
    }
}

pub struct CurrentStreakBadge;
impl Badge for CurrentStreakBadge {
    fn key(&self) -> &'static str {
        "current_streak"
    }
    fn icon(&self) -> &'static str {
        "🔥"
    }
    fn title(&self) -> &'static str {
        "Aktuelle Serie"
    }
    fn how_to_earn(&self) -> &'static str {
        "Folge fertiger Matches mit ≥1 Punkt — bricht beim ersten 0-Punkte-Match ab."
    }
    fn accent(&self) -> BadgeAccent {
        BadgeAccent::Green
    }
    fn compute(&self, ctx: &BadgeContext<'_>) -> BadgeDisplay {
        let mut user_rows: Vec<&PredictionRow> = ctx
            .finished_predictions
            .iter()
            .filter(|r| r.user_id == ctx.user_id)
            .collect();
        user_rows.sort_by_key(|b| std::cmp::Reverse(b.kickoff)); // neueste zuerst
        let mut streak = 0;
        for r in user_rows {
            if r.base_points() > 0 {
                streak += 1;
            } else {
                break;
            }
        }
        BadgeDisplay::Metric(BadgeValue::Streak(streak))
    }
}

pub struct LongestStreakBadge;
impl Badge for LongestStreakBadge {
    fn key(&self) -> &'static str {
        "longest_streak"
    }
    fn icon(&self) -> &'static str {
        "🚀"
    }
    fn title(&self) -> &'static str {
        "Längste Serie"
    }
    fn how_to_earn(&self) -> &'static str {
        "Längste je erreichte Folge fertiger Matches mit ≥1 Punkt."
    }
    fn compute(&self, ctx: &BadgeContext<'_>) -> BadgeDisplay {
        let mut user_rows: Vec<&PredictionRow> = ctx
            .finished_predictions
            .iter()
            .filter(|r| r.user_id == ctx.user_id)
            .collect();
        user_rows.sort_by_key(|b| b.kickoff); // chronologisch
        let mut best = 0;
        let mut current = 0;
        for r in user_rows {
            if r.base_points() > 0 {
                current += 1;
                if current > best {
                    best = current;
                }
            } else {
                current = 0;
            }
        }
        BadgeDisplay::Metric(BadgeValue::Streak(best))
    }
}

pub struct RankDeltaBadge;
impl Badge for RankDeltaBadge {
    fn key(&self) -> &'static str {
        "rank_delta"
    }
    fn icon(&self) -> &'static str {
        "📊"
    }
    fn title(&self) -> &'static str {
        "Rang-Bewegung"
    }
    fn how_to_earn(&self) -> &'static str {
        "Veränderung deines Rangs seit dem letzten Spieltag (positiv = aufgestiegen)."
    }
    fn compute(&self, ctx: &BadgeContext<'_>) -> BadgeDisplay {
        // prev = nur Matches mit Berlin-Datum vor heute. Ohne solche Matches: kein Delta.
        let prev_rows: Vec<&PredictionRow> = ctx
            .finished_predictions
            .iter()
            .filter(|r| r.berlin_date() < ctx.berlin_today)
            .collect();
        if prev_rows.is_empty() {
            return BadgeDisplay::Metric(BadgeValue::Delta(None));
        }
        // Punkte je User für prev (nur Matches mit Datum < heute).
        let prev_final_decided = prev_rows
            .iter()
            .any(|r| matches!(r.stage, Stage::Final));
        let prev_totals = totals_with_champion(
            &prev_rows,
            ctx.all_user_ids,
            ctx.all_special_picks,
            if prev_final_decided {
                ctx.actual_champion_id
            } else {
                None
            },
        );
        // Gesamttotals (alle finished + Champion-Bonus falls schon entschieden).
        let cur_rows: Vec<&PredictionRow> = ctx.finished_predictions.iter().collect();
        let cur_totals = totals_with_champion(
            &cur_rows,
            ctx.all_user_ids,
            ctx.all_special_picks,
            ctx.actual_champion_id,
        );
        let prev_rank = rank_of(ctx.user_id, &prev_totals);
        let cur_rank = rank_of(ctx.user_id, &cur_totals);
        match (prev_rank, cur_rank) {
            (Some(p), Some(c)) => BadgeDisplay::Metric(BadgeValue::Delta(Some(p - c))),
            _ => BadgeDisplay::Metric(BadgeValue::Delta(None)),
        }
    }
}

pub struct ChampionPickBadge;
impl Badge for ChampionPickBadge {
    fn key(&self) -> &'static str {
        "champion_pick"
    }
    fn icon(&self) -> &'static str {
        "👑"
    }
    fn title(&self) -> &'static str {
        "Weltmeister-Tipp"
    }
    fn how_to_earn(&self) -> &'static str {
        "Dein Champion-Pick. 10 Punkte, wenn dein Team das Turnier gewinnt."
    }
    fn accent(&self) -> BadgeAccent {
        BadgeAccent::Yellow
    }
    fn compute(&self, ctx: &BadgeContext<'_>) -> BadgeDisplay {
        match ctx.user_champion {
            Some(c) => BadgeDisplay::Metric(BadgeValue::Champion {
                team: c.team_name.clone(),
                flag_url: c.flag_url.clone(),
            }),
            None => BadgeDisplay::Metric(BadgeValue::Empty),
        }
    }
}

// ─── Helfer ───────────────────────────────────────────────────────────────────

fn totals_with_champion(
    rows: &[&PredictionRow],
    all_user_ids: &[Uuid],
    special_picks: &[(Uuid, i32)],
    actual_champion: Option<i32>,
) -> HashMap<Uuid, i32> {
    let mut totals: HashMap<Uuid, i32> = all_user_ids.iter().map(|u| (*u, 0)).collect();
    for r in rows {
        *totals.entry(r.user_id).or_insert(0) += r.base_points();
    }
    if let Some(actual) = actual_champion {
        for (uid, pick) in special_picks {
            if *pick == actual {
                *totals.entry(*uid).or_insert(0) += scoring::champion_points(Some(*pick), Some(actual));
            }
        }
    }
    totals
}

/// Liefert den Rang (1-basiert) des Users in `totals`. Bei Punktegleichstand teilen
/// sich User denselben Rang (dense ranking: "1, 2, 2, 3").
fn rank_of(user: Uuid, totals: &HashMap<Uuid, i32>) -> Option<i32> {
    let user_pts = *totals.get(&user)?;
    let distinct: HashSet<i32> = totals.values().copied().collect();
    let higher = distinct.iter().filter(|&&p| p > user_pts).count() as i32;
    Some(higher + 1)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn uid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    fn ko(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, day, hour, 0, 0).unwrap()
    }

    fn pred(
        user: Uuid,
        match_id: i32,
        kickoff: DateTime<Utc>,
        score: (i32, i32),
        prediction: (i32, i32),
        stage: Stage,
    ) -> PredictionRow {
        PredictionRow {
            user_id: user,
            match_id,
            stage,
            kickoff,
            score_h: score.0,
            score_a: score.1,
            pred_h: prediction.0,
            pred_a: prediction.1,
        }
    }

    fn run<B: Badge>(badge: B, owned: &BadgeContextOwned) -> BadgeDisplay {
        badge.compute(&owned.as_ctx())
    }

    fn base_owned(user: Uuid, rows: Vec<PredictionRow>, all_users: Vec<Uuid>) -> BadgeContextOwned {
        BadgeContextOwned {
            user_id: user,
            now: ko(15, 12),
            berlin_today: NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
            finished_predictions: rows,
            all_user_ids: all_users,
            started_matches_total: 0,
            user_started_tips: 0,
            actual_champion_id: None,
            all_special_picks: vec![],
            user_champion: None,
        }
    }

    // ─── ExactCount ─────────────────────────────────────────────────────────────

    #[test]
    fn exact_count_zero_when_empty() {
        let owned = base_owned(uid(1), vec![], vec![uid(1)]);
        assert_eq!(
            run(ExactCountBadge, &owned),
            BadgeDisplay::Achievement { times_earned: 0 }
        );
    }

    #[test]
    fn exact_count_counts_only_exacts() {
        let me = uid(1);
        let owned = base_owned(
            me,
            vec![
                pred(me, 1, ko(10, 18), (2, 1), (2, 1), Stage::Group), // exact
                pred(me, 2, ko(11, 18), (2, 1), (3, 0), Stage::Group), // tendency only
                pred(me, 3, ko(12, 18), (0, 0), (0, 0), Stage::Group), // exact
            ],
            vec![me],
        );
        assert_eq!(
            run(ExactCountBadge, &owned),
            BadgeDisplay::Achievement { times_earned: 2 }
        );
    }

    #[test]
    fn exact_count_ignores_other_users() {
        let me = uid(1);
        let other = uid(2);
        let owned = base_owned(
            me,
            vec![
                pred(other, 1, ko(10, 18), (2, 1), (2, 1), Stage::Group),
                pred(me, 1, ko(10, 18), (2, 1), (1, 0), Stage::Group),
            ],
            vec![me, other],
        );
        assert_eq!(
            run(ExactCountBadge, &owned),
            BadgeDisplay::Achievement { times_earned: 0 }
        );
    }

    // ─── MatchdayWins ───────────────────────────────────────────────────────────

    #[test]
    fn matchday_wins_solo_and_tie() {
        let a = uid(1);
        let b = uid(2);
        let c = uid(3);
        // Tag 10: A=4 (exact), B=1 (tendency), C=0 → A solo
        // Tag 11: A=4 (exact), B=4 (exact), C=2 → A & B tie
        let rows = vec![
            pred(a, 1, ko(10, 18), (1, 0), (1, 0), Stage::Group), // 4
            pred(b, 1, ko(10, 18), (1, 0), (2, 0), Stage::Group), // 1 (tendency)
            pred(c, 1, ko(10, 18), (1, 0), (0, 1), Stage::Group), // 0
            pred(a, 2, ko(11, 18), (2, 2), (2, 2), Stage::Group), // 4
            pred(b, 2, ko(11, 18), (2, 2), (2, 2), Stage::Group), // 4
            pred(c, 2, ko(11, 18), (2, 2), (1, 1), Stage::Group), // 2 (diff)
        ];
        let owned_a = base_owned(a, rows.clone(), vec![a, b, c]);
        let owned_b = base_owned(b, rows.clone(), vec![a, b, c]);
        let owned_c = base_owned(c, rows.clone(), vec![a, b, c]);
        assert_eq!(
            run(MatchdayWinsBadge, &owned_a),
            BadgeDisplay::Achievement { times_earned: 2 }
        );
        assert_eq!(
            run(MatchdayWinsBadge, &owned_b),
            BadgeDisplay::Achievement { times_earned: 1 }
        );
        assert_eq!(
            run(MatchdayWinsBadge, &owned_c),
            BadgeDisplay::Achievement { times_earned: 0 }
        );
    }

    #[test]
    fn matchday_wins_zero_points_does_not_count() {
        let a = uid(1);
        // User-allein an einem Tag, aber 0 Punkte → kein Sieg.
        let rows = vec![pred(a, 1, ko(10, 18), (1, 0), (0, 1), Stage::Group)];
        let owned = base_owned(a, rows, vec![a]);
        assert_eq!(
            run(MatchdayWinsBadge, &owned),
            BadgeDisplay::Achievement { times_earned: 0 }
        );
    }

    #[test]
    fn matchday_wins_empty() {
        let owned = base_owned(uid(1), vec![], vec![uid(1)]);
        assert_eq!(
            run(MatchdayWinsBadge, &owned),
            BadgeDisplay::Achievement { times_earned: 0 }
        );
    }

    // ─── Underdog ───────────────────────────────────────────────────────────────

    #[test]
    fn underdog_all_exact_no_hit() {
        // 4 User, alle exakt → Anteil 100% > 30% → 0
        let users: Vec<Uuid> = (1..=4).map(uid).collect();
        let rows: Vec<_> = users
            .iter()
            .map(|u| pred(*u, 1, ko(10, 18), (1, 0), (1, 0), Stage::Group))
            .collect();
        let owned = base_owned(users[0], rows, users.clone());
        assert_eq!(
            run(UnderdogBadge, &owned),
            BadgeDisplay::Achievement { times_earned: 0 }
        );
    }

    #[test]
    fn underdog_one_exact_among_four_is_hit() {
        // 4 User, nur User 1 exakt → Anteil 25% < 30% → 1
        let users: Vec<Uuid> = (1..=4).map(uid).collect();
        let rows = vec![
            pred(users[0], 1, ko(10, 18), (1, 0), (1, 0), Stage::Group), // exact
            pred(users[1], 1, ko(10, 18), (1, 0), (2, 0), Stage::Group),
            pred(users[2], 1, ko(10, 18), (1, 0), (3, 0), Stage::Group),
            pred(users[3], 1, ko(10, 18), (1, 0), (0, 0), Stage::Group),
        ];
        let owned = base_owned(users[0], rows, users.clone());
        assert_eq!(
            run(UnderdogBadge, &owned),
            BadgeDisplay::Achievement { times_earned: 1 }
        );
    }

    #[test]
    fn underdog_user_not_exact_no_hit() {
        let users: Vec<Uuid> = (1..=4).map(uid).collect();
        let rows = vec![
            pred(users[0], 1, ko(10, 18), (1, 0), (2, 0), Stage::Group), // not exact
            pred(users[1], 1, ko(10, 18), (1, 0), (1, 0), Stage::Group),
        ];
        let owned = base_owned(users[0], rows, users.clone());
        assert_eq!(
            run(UnderdogBadge, &owned),
            BadgeDisplay::Achievement { times_earned: 0 }
        );
    }

    // ─── SoloHit ────────────────────────────────────────────────────────────────

    #[test]
    fn solo_hit_only_user_exact() {
        let users: Vec<Uuid> = (1..=3).map(uid).collect();
        let rows = vec![
            pred(users[0], 1, ko(10, 18), (1, 0), (1, 0), Stage::Group), // exact
            pred(users[1], 1, ko(10, 18), (1, 0), (2, 0), Stage::Group),
            pred(users[2], 1, ko(10, 18), (1, 0), (0, 0), Stage::Group),
        ];
        let owned = base_owned(users[0], rows, users.clone());
        assert_eq!(
            run(SoloHitBadge, &owned),
            BadgeDisplay::Achievement { times_earned: 1 }
        );
    }

    #[test]
    fn solo_hit_two_exact_no_hit() {
        let users: Vec<Uuid> = (1..=3).map(uid).collect();
        let rows = vec![
            pred(users[0], 1, ko(10, 18), (1, 0), (1, 0), Stage::Group),
            pred(users[1], 1, ko(10, 18), (1, 0), (1, 0), Stage::Group),
        ];
        let owned = base_owned(users[0], rows, users.clone());
        assert_eq!(
            run(SoloHitBadge, &owned),
            BadgeDisplay::Achievement { times_earned: 0 }
        );
    }

    #[test]
    fn solo_hit_empty() {
        let owned = base_owned(uid(1), vec![], vec![uid(1)]);
        assert_eq!(
            run(SoloHitBadge, &owned),
            BadgeDisplay::Achievement { times_earned: 0 }
        );
    }

    // ─── TendencyPct ────────────────────────────────────────────────────────────

    #[test]
    fn tendency_pct_none_without_tips() {
        let owned = base_owned(uid(1), vec![], vec![uid(1)]);
        assert_eq!(
            run(TendencyPctBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Percent(None))
        );
    }

    #[test]
    fn tendency_pct_basic() {
        let me = uid(1);
        let rows = vec![
            pred(me, 1, ko(10, 18), (1, 0), (1, 0), Stage::Group), // 4
            pred(me, 2, ko(11, 18), (1, 0), (2, 0), Stage::Group), // 1
            pred(me, 3, ko(12, 18), (1, 0), (0, 1), Stage::Group), // 0
            pred(me, 4, ko(13, 18), (1, 0), (0, 2), Stage::Group), // 0
        ];
        let owned = base_owned(me, rows, vec![me]);
        // 2/4 → 50%
        assert_eq!(
            run(TendencyPctBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Percent(Some(50)))
        );
    }

    #[test]
    fn tendency_pct_only_user_rows() {
        let me = uid(1);
        let other = uid(2);
        let rows = vec![
            pred(me, 1, ko(10, 18), (1, 0), (1, 0), Stage::Group),
            pred(other, 1, ko(10, 18), (1, 0), (0, 1), Stage::Group), // anderer User, ignoriert
        ];
        let owned = base_owned(me, rows, vec![me, other]);
        assert_eq!(
            run(TendencyPctBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Percent(Some(100)))
        );
    }

    // ─── KnockoutPoints ─────────────────────────────────────────────────────────

    #[test]
    fn ko_points_empty_before_first_ko() {
        let me = uid(1);
        let rows = vec![pred(me, 1, ko(10, 18), (1, 0), (1, 0), Stage::Group)];
        let owned = base_owned(me, rows, vec![me]);
        assert_eq!(
            run(KnockoutPointsBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Empty)
        );
    }

    #[test]
    fn ko_points_sums_only_ko() {
        let me = uid(1);
        let rows = vec![
            pred(me, 1, ko(10, 18), (1, 0), (1, 0), Stage::Group), // 4 (group, ignored)
            pred(me, 2, ko(20, 18), (2, 1), (2, 1), Stage::RoundOf32), // 4 * 2 = 8
            pred(me, 3, ko(25, 18), (1, 0), (1, 0), Stage::QuarterFinal), // 4 * 4 = 16
        ];
        let owned = base_owned(me, rows, vec![me]);
        assert_eq!(
            run(KnockoutPointsBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Count(24))
        );
    }

    #[test]
    fn ko_points_zero_when_no_ko_predictions_but_others_exist() {
        let me = uid(1);
        let other = uid(2);
        let rows = vec![pred(other, 1, ko(20, 18), (1, 0), (1, 0), Stage::RoundOf32)];
        let owned = base_owned(me, rows, vec![me, other]);
        // Es gibt K.O.-Matches, aber User selbst hat keine Punkte → 0
        assert_eq!(
            run(KnockoutPointsBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Count(0))
        );
    }

    // ─── DisciplinePct ──────────────────────────────────────────────────────────

    #[test]
    fn discipline_pct_none_when_no_started() {
        let owned = base_owned(uid(1), vec![], vec![uid(1)]);
        assert_eq!(
            run(DisciplinePctBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Percent(None))
        );
    }

    #[test]
    fn discipline_pct_basic() {
        let mut owned = base_owned(uid(1), vec![], vec![uid(1)]);
        owned.started_matches_total = 10;
        owned.user_started_tips = 7;
        assert_eq!(
            run(DisciplinePctBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Percent(Some(70)))
        );
    }

    #[test]
    fn discipline_pct_full() {
        let mut owned = base_owned(uid(1), vec![], vec![uid(1)]);
        owned.started_matches_total = 5;
        owned.user_started_tips = 5;
        assert_eq!(
            run(DisciplinePctBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Percent(Some(100)))
        );
    }

    // ─── CurrentStreak ──────────────────────────────────────────────────────────

    #[test]
    fn current_streak_breaks_on_zero_at_top() {
        let me = uid(1);
        // chronologisch: 4, 2, 1 (alle ≥1) — aber neuestes ist 0
        let rows = vec![
            pred(me, 1, ko(10, 18), (1, 0), (1, 0), Stage::Group), // 4
            pred(me, 2, ko(11, 18), (1, 0), (2, 1), Stage::Group), // 2 (diff)
            pred(me, 3, ko(12, 18), (1, 0), (3, 0), Stage::Group), // 1 (tendency)
            pred(me, 4, ko(13, 18), (1, 0), (0, 1), Stage::Group), // 0 ← neuestes
        ];
        let owned = base_owned(me, rows, vec![me]);
        assert_eq!(
            run(CurrentStreakBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Streak(0))
        );
    }

    #[test]
    fn current_streak_counts_consecutive_recent() {
        let me = uid(1);
        // chronologisch: 0, 0, 4, 2, 1 → Streak 3 (1, 2, 4 zurückwärts)
        let rows = vec![
            pred(me, 1, ko(10, 18), (1, 0), (0, 1), Stage::Group), // 0
            pred(me, 2, ko(11, 18), (1, 0), (0, 2), Stage::Group), // 0
            pred(me, 3, ko(12, 18), (1, 0), (1, 0), Stage::Group), // 4
            pred(me, 4, ko(13, 18), (2, 1), (3, 2), Stage::Group), // 2 (diff)
            pred(me, 5, ko(14, 18), (1, 0), (3, 0), Stage::Group), // 1 (tendency)
        ];
        let owned = base_owned(me, rows, vec![me]);
        assert_eq!(
            run(CurrentStreakBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Streak(3))
        );
    }

    #[test]
    fn current_streak_zero_when_empty() {
        let owned = base_owned(uid(1), vec![], vec![uid(1)]);
        assert_eq!(
            run(CurrentStreakBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Streak(0))
        );
    }

    // ─── LongestStreak ──────────────────────────────────────────────────────────

    #[test]
    fn longest_streak_tracks_best_run() {
        let me = uid(1);
        // chronologisch: 1, 0, 2, 4, 4, 0, 1 → längste 3
        let rows = vec![
            pred(me, 1, ko(10, 18), (1, 0), (3, 0), Stage::Group), // 1
            pred(me, 2, ko(11, 18), (1, 0), (0, 1), Stage::Group), // 0
            pred(me, 3, ko(12, 18), (2, 1), (3, 2), Stage::Group), // 2
            pred(me, 4, ko(13, 18), (1, 0), (1, 0), Stage::Group), // 4
            pred(me, 5, ko(14, 18), (0, 0), (0, 0), Stage::Group), // 4
            pred(me, 6, ko(15, 18), (1, 0), (0, 1), Stage::Group), // 0
            pred(me, 7, ko(16, 18), (1, 0), (3, 0), Stage::Group), // 1
        ];
        let owned = base_owned(me, rows, vec![me]);
        assert_eq!(
            run(LongestStreakBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Streak(3))
        );
    }

    #[test]
    fn longest_streak_zero_when_all_misses() {
        let me = uid(1);
        let rows = vec![pred(me, 1, ko(10, 18), (1, 0), (0, 1), Stage::Group)];
        let owned = base_owned(me, rows, vec![me]);
        assert_eq!(
            run(LongestStreakBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Streak(0))
        );
    }

    #[test]
    fn longest_streak_zero_when_empty() {
        let owned = base_owned(uid(1), vec![], vec![uid(1)]);
        assert_eq!(
            run(LongestStreakBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Streak(0))
        );
    }

    // ─── RankDelta ──────────────────────────────────────────────────────────────

    #[test]
    fn rank_delta_none_when_no_prior_day() {
        let me = uid(1);
        // Heute (15.6.) ist auch der einzige Tag mit Matches.
        let rows = vec![pred(me, 1, ko(15, 18), (1, 0), (1, 0), Stage::Group)];
        let owned = base_owned(me, rows, vec![me]);
        assert_eq!(
            run(RankDeltaBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Delta(None))
        );
    }

    #[test]
    fn rank_delta_climb() {
        let a = uid(1); // me
        let b = uid(2);
        let c = uid(3);
        // Vor heute: B=4, C=4, A=0 → A auf Rang 2 (A=0, B&C=4 tied)
        // Heute: A=4, B=0, C=0 → A=4, B=4, C=4 alle tied → Rang 1
        let rows = vec![
            // Vortag (10.6.)
            pred(a, 1, ko(10, 18), (1, 0), (0, 1), Stage::Group),
            pred(b, 1, ko(10, 18), (1, 0), (1, 0), Stage::Group),
            pred(c, 1, ko(10, 18), (1, 0), (1, 0), Stage::Group),
            // Heute (15.6.)
            pred(a, 2, ko(15, 18), (1, 0), (1, 0), Stage::Group),
            pred(b, 2, ko(15, 18), (1, 0), (0, 1), Stage::Group),
            pred(c, 2, ko(15, 18), (1, 0), (0, 1), Stage::Group),
        ];
        let owned = base_owned(a, rows, vec![a, b, c]);
        // prev_rank: A=0, B=4, C=4 → 2 verschiedene Punktwerte > 0 = 1 → A Rang 2.
        // cur_rank: A=4, B=4, C=4 → alle gleich → A Rang 1.
        // Delta: 2 - 1 = 1
        assert_eq!(
            run(RankDeltaBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Delta(Some(1)))
        );
    }

    #[test]
    fn rank_delta_drop() {
        let a = uid(1);
        let b = uid(2);
        // Vortag: A=4, B=0 → A Rang 1. Heute: A=0, B=4 → A=4, B=4 tied → A Rang 1.
        // Delta = 0 (kein Drop, weil tie). Eigentlicher Drop-Test: A=4, B=0 → A=4, B=8.
        let rows = vec![
            pred(a, 1, ko(10, 18), (1, 0), (1, 0), Stage::Group), // A=4
            pred(b, 1, ko(10, 18), (1, 0), (0, 1), Stage::Group), // B=0
            pred(a, 2, ko(15, 18), (1, 0), (0, 1), Stage::Group), // A=0
            pred(b, 2, ko(15, 18), (1, 0), (1, 0), Stage::Group), // B=4
            pred(b, 3, ko(15, 19), (2, 1), (2, 1), Stage::Group), // B=4 → B total 8
        ];
        let owned = base_owned(a, rows, vec![a, b]);
        // prev: A=4, B=0 → A Rang 1. cur: A=4, B=8 → A Rang 2. Delta 1-2 = -1.
        assert_eq!(
            run(RankDeltaBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Delta(Some(-1)))
        );
    }

    // ─── Champion ───────────────────────────────────────────────────────────────

    #[test]
    fn champion_empty_without_pick() {
        let owned = base_owned(uid(1), vec![], vec![uid(1)]);
        assert_eq!(
            run(ChampionPickBadge, &owned),
            BadgeDisplay::Metric(BadgeValue::Empty)
        );
    }

    #[test]
    fn champion_with_pick() {
        let mut owned = base_owned(uid(1), vec![], vec![uid(1)]);
        owned.user_champion = Some(ChampionView {
            team_name: "Deutschland".into(),
            flag_url: "https://flagcdn.com/w40/de.png".into(),
        });
        let r = run(ChampionPickBadge, &owned);
        match r {
            BadgeDisplay::Metric(BadgeValue::Champion { team, flag_url }) => {
                assert_eq!(team, "Deutschland");
                assert!(flag_url.ends_with("de.png"));
            }
            _ => panic!("expected Champion variant, got {:?}", r),
        }
    }

    // ─── Sanity-Schleife ────────────────────────────────────────────────────────

    #[test]
    fn sanity_invariants_over_random_inputs() {
        // Deterministische Pseudo-Sequenz statt rand-Crate.
        let me = uid(1);
        let other = uid(2);
        for seed in 0u32..50 {
            let mut rows = vec![];
            for i in 0..10 {
                let day = 10 + (i + seed) % 6;
                let actual = ((seed.wrapping_mul(31) + i) % 4) as i32;
                let pred_h = ((seed.wrapping_mul(17) + i * 3) % 4) as i32;
                let pred_a = ((seed.wrapping_mul(23) + i * 5) % 4) as i32;
                let user = if (seed + i) % 2 == 0 { me } else { other };
                let stage = if i % 5 == 0 {
                    Stage::RoundOf16
                } else {
                    Stage::Group
                };
                rows.push(pred(
                    user,
                    i as i32,
                    ko(day, 18),
                    (actual, (actual + 1) % 4),
                    (pred_h, pred_a),
                    stage,
                ));
            }
            let owned = base_owned(me, rows, vec![me, other]);

            // ExactCount ≤ Anzahl getippter fertiger Matches.
            let user_finished = owned
                .finished_predictions
                .iter()
                .filter(|r| r.user_id == me)
                .count() as i32;
            if let BadgeDisplay::Achievement { times_earned } = run(ExactCountBadge, &owned) {
                assert!(
                    times_earned <= user_finished,
                    "exact_count {} > user_finished {}",
                    times_earned,
                    user_finished
                );
            }

            // TendencyPct ∈ [0,100] oder None.
            if let BadgeDisplay::Metric(BadgeValue::Percent(p)) = run(TendencyPctBadge, &owned) {
                if let Some(v) = p {
                    assert!((0..=100).contains(&v), "pct out of range: {v}");
                }
            }

            // CurrentStreak ≤ LongestStreak.
            let cur = match run(CurrentStreakBadge, &owned) {
                BadgeDisplay::Metric(BadgeValue::Streak(n)) => n,
                _ => 0,
            };
            let lng = match run(LongestStreakBadge, &owned) {
                BadgeDisplay::Metric(BadgeValue::Streak(n)) => n,
                _ => 0,
            };
            assert!(cur <= lng, "current {cur} > longest {lng}");
        }
    }

    // ─── compute_all/registry ───────────────────────────────────────────────────

    #[test]
    fn registry_has_all_badges_in_stable_order() {
        let owned = base_owned(uid(1), vec![], vec![uid(1)]);
        let views = compute_all(&owned.as_ctx());
        let keys: Vec<&str> = views.iter().map(|v| v.key).collect();
        assert_eq!(
            keys,
            vec![
                "matchday_wins",
                "exact_count",
                "underdog",
                "solo_hit",
                "tendency_pct",
                "knockout_points",
                "discipline_pct",
                "current_streak",
                "longest_streak",
                "rank_delta",
                "champion_pick",
            ]
        );
        // Jeder Badge hat einen nicht-leeren how_to_earn-Text.
        for v in &views {
            assert!(
                !v.how_to_earn.is_empty(),
                "{} hat keinen how_to_earn-Text",
                v.key
            );
            assert!(!v.title.is_empty());
            assert!(!v.icon.is_empty());
        }
    }

}
