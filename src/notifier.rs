use std::sync::Arc;

use chrono::{DateTime, Timelike, Utc};
use chrono_tz::Europe::Berlin;
use reqwest::Client;
use serde::Serialize;

use crate::stage::Stage;

pub type NotifierError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Clone)]
pub enum NotificationEvent {
    MatchClosingSoon {
        match_id: i32,
        home: String,
        away: String,
        stage: Stage,
        group_letter: Option<String>,
        lock_at: DateTime<Utc>,
        missing_names: Vec<String>,
    },
    SpecialPredictionsLock {
        lock_at: DateTime<Utc>,
        missing_names: Vec<String>,
    },
    KnockoutBracketReady {
        stage: Stage,
        match_count: i32,
    },
}

#[async_trait::async_trait]
pub trait Notifier: Send + Sync {
    async fn notify(&self, event: NotificationEvent) -> Result<(), NotifierError>;
}

pub fn in_quiet_hours_now() -> bool {
    let h = Utc::now().with_timezone(&Berlin).hour();
    !(8..22).contains(&h)
}

pub fn from_env() -> Arc<dyn Notifier> {
    let api = std::env::var("SIGNAL_API_URL").ok();
    let from = std::env::var("SIGNAL_FROM_NUMBER").ok();
    let group = std::env::var("SIGNAL_GROUP_ID").ok();
    let base_url = std::env::var("BASE_URL")
        .unwrap_or_else(|_| "https://pila.example.com".to_string());

    match (api, from, group) {
        (Some(api), Some(from), Some(group))
            if !api.is_empty() && !from.is_empty() && !group.is_empty() =>
        {
            tracing::info!("Signal notifier configured (group {})", group);
            Arc::new(SignalNotifier {
                client: Client::new(),
                api_url: api,
                from_number: from,
                group_id: group,
                base_url,
            })
        }
        _ => {
            tracing::info!("Signal env vars not set — notifications disabled");
            Arc::new(NoopNotifier)
        }
    }
}

pub struct NoopNotifier;

#[async_trait::async_trait]
impl Notifier for NoopNotifier {
    async fn notify(&self, event: NotificationEvent) -> Result<(), NotifierError> {
        tracing::debug!("NoopNotifier dropping event: {:?}", event);
        Ok(())
    }
}

pub struct SignalNotifier {
    client: Client,
    api_url: String,
    from_number: String,
    group_id: String,
    base_url: String,
}

impl SignalNotifier {
    pub fn new(api_url: &str, from_number: &str, group_id: &str, base_url: &str) -> Self {
        Self {
            client: Client::new(),
            api_url: api_url.to_string(),
            from_number: from_number.to_string(),
            group_id: group_id.to_string(),
            base_url: base_url.to_string(),
        }
    }
}

#[derive(Serialize)]
struct SignalSendBody<'a> {
    message: &'a str,
    number: &'a str,
    recipients: Vec<&'a str>,
}

#[async_trait::async_trait]
impl Notifier for SignalNotifier {
    async fn notify(&self, event: NotificationEvent) -> Result<(), NotifierError> {
        let message = render_message(&event, &self.base_url);
        send_signal_message(
            &self.client,
            &self.api_url,
            &self.from_number,
            &self.group_id,
            &message,
        )
        .await
    }
}

pub async fn send_signal_message(
    client: &Client,
    api_url: &str,
    from: &str,
    recipient: &str,
    message: &str,
) -> Result<(), NotifierError> {
    let body = SignalSendBody {
        message,
        number: from,
        recipients: vec![recipient],
    };
    let url = format!("{}/v2/send", api_url.trim_end_matches('/'));
    let resp = client.post(&url).json(&body).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Signal API {url} returned {status}: {text}").into());
    }
    Ok(())
}

pub fn signal_configured() -> bool {
    let api = std::env::var("SIGNAL_API_URL").ok();
    let from = std::env::var("SIGNAL_FROM_NUMBER").ok();
    matches!((api, from), (Some(a), Some(f)) if !a.is_empty() && !f.is_empty())
}

pub async fn send_invite_via_signal(
    phone: &str,
    name: &str,
    magic_link: &str,
) -> Result<(), NotifierError> {
    let api = std::env::var("SIGNAL_API_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or("SIGNAL_API_URL not set")?;
    let from = std::env::var("SIGNAL_FROM_NUMBER")
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or("SIGNAL_FROM_NUMBER not set")?;

    let message = format!(
        "Hallo {name}! Du bist beim Pila WM-Tippspiel dabei. Dein Login-Link: {magic_link}"
    );
    let client = Client::new();
    send_signal_message(&client, &api, &from, phone, &message).await
}

fn names_or_count(names: &[String]) -> String {
    if names.len() > 5 {
        format!("{} Mitspieler", names.len())
    } else {
        names.join(", ")
    }
}

fn render_message(event: &NotificationEvent, base_url: &str) -> String {
    match event {
        NotificationEvent::MatchClosingSoon {
            home,
            away,
            stage,
            group_letter,
            missing_names,
            ..
        } => {
            let where_label = match (stage, group_letter) {
                (Stage::Group, Some(letter)) => format!("Gruppe {letter}"),
                (s, _) => s.label_de().to_string(),
            };
            format!(
                "⚽ Tipp-Erinnerung: {home} – {away} ({where_label}) wird in <24h angepfiffen. Es fehlen noch: {who}. Jetzt tippen: {base_url}",
                who = names_or_count(missing_names)
            )
        }
        NotificationEvent::SpecialPredictionsLock { missing_names, .. } => format!(
            "⏰ Weltmeister-Tipp wird in <24h gesperrt (Anpfiff Eröffnungsspiel). Es fehlen: {}. {base_url}",
            names_or_count(missing_names)
        ),
        NotificationEvent::KnockoutBracketReady {
            stage,
            match_count,
        } => format!(
            "🏆 {stage_label}: {match_count} Paarungen stehen fest, Tipps jetzt möglich. {base_url}",
            stage_label = stage.label_de()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_or_count_short_list_joins() {
        let names = vec!["Anna".into(), "Ben".into()];
        assert_eq!(names_or_count(&names), "Anna, Ben");
    }

    #[test]
    fn names_or_count_long_list_falls_back_to_count() {
        let names: Vec<String> = (0..6).map(|i| format!("U{i}")).collect();
        assert_eq!(names_or_count(&names), "6 Mitspieler");
    }

    #[test]
    fn render_match_closing_includes_teams_and_group() {
        let ev = NotificationEvent::MatchClosingSoon {
            match_id: 1,
            home: "Deutschland".into(),
            away: "Schottland".into(),
            stage: Stage::Group,
            group_letter: Some("A".into()),
            lock_at: Utc::now(),
            missing_names: vec!["Anna".into(), "Ben".into()],
        };
        let msg = render_message(&ev, "https://x.example");
        assert!(msg.contains("Deutschland"));
        assert!(msg.contains("Schottland"));
        assert!(msg.contains("Gruppe A"));
        assert!(msg.contains("Anna, Ben"));
        assert!(msg.contains("https://x.example"));
    }

    #[test]
    fn render_match_closing_knockout_uses_stage_label() {
        let ev = NotificationEvent::MatchClosingSoon {
            match_id: 1,
            home: "FRA".into(),
            away: "BRA".into(),
            stage: Stage::QuarterFinal,
            group_letter: None,
            lock_at: Utc::now(),
            missing_names: vec![],
        };
        let msg = render_message(&ev, "u");
        assert!(msg.contains("Viertelfinale"));
    }

    #[test]
    fn render_special_lock_mentions_weltmeister() {
        let ev = NotificationEvent::SpecialPredictionsLock {
            lock_at: Utc::now(),
            missing_names: vec!["Anna".into()],
        };
        let msg = render_message(&ev, "u");
        assert!(msg.contains("Weltmeister"));
        assert!(msg.contains("Anna"));
    }

    #[test]
    fn render_knockout_ready() {
        let ev = NotificationEvent::KnockoutBracketReady {
            stage: Stage::RoundOf16,
            match_count: 8,
        };
        let msg = render_message(&ev, "u");
        assert!(msg.contains("Achtelfinale"));
        assert!(msg.contains("8"));
    }
}
