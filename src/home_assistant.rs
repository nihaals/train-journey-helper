use crate::{config::HomeAssistantConfig, custom_types::JourneyOption};
use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Clone)]
pub struct HomeAssistantNotifier {
    http: reqwest::Client,
    config: HomeAssistantConfig,
}

#[derive(Debug, Serialize)]
struct NotifyRequest<'a> {
    title: &'a str,
    message: String,
    data: NotifyData<'a>,
}

#[derive(Debug, Serialize)]
struct NotifyData<'a> {
    tag: &'a str,
    group: &'a str,
    sticky: bool,
}

impl HomeAssistantNotifier {
    pub fn new(config: HomeAssistantConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            config,
        }
    }

    pub async fn send_status_report(&self, options: &[JourneyOption]) -> Result<()> {
        let mut message = String::new();
        for option in options {
            message.push_str(&format_journey(option));
            message.push('\n');
        }
        self.send("Train options", "train-journey-status", message)
            .await
    }

    pub async fn send_train_delayed_before_train_1(&self, best: &JourneyOption) -> Result<()> {
        self.send(
            "Train delayed",
            "train-journey-status",
            format!(
                "A train is delayed. Best current option:\n{}",
                format_journey(best)
            ),
        )
        .await
    }

    pub async fn send_return_details(&self, options: &[JourneyOption]) -> Result<()> {
        let mut message = String::new();
        for option in options {
            message.push_str(&format_journey(option));
            message.push('\n');
        }
        self.send("Return train options", "train-journey-return", message)
            .await
    }

    pub async fn send_healthcheck(&self) -> Result<()> {
        self.send(
            "Train journey helper",
            "train-journey-healthcheck",
            "Service is running".to_string(),
        )
        .await
    }

    async fn send(&self, title: &'static str, tag: &'static str, message: String) -> Result<()> {
        let url = format!(
            "{}/api/services/notify/{}",
            self.config.base_url.trim_end_matches('/'),
            self.config.notify_service
        );
        self.http
            .post(url)
            .bearer_auth(&self.config.token)
            .json(&NotifyRequest {
                title,
                message,
                data: NotifyData {
                    tag,
                    group: "train-journey-helper",
                    sticky: true,
                },
            })
            .send()
            .await
            .context("sending Home Assistant notification")?
            .error_for_status()
            .context("Home Assistant returned an error status")?;
        Ok(())
    }
}

fn format_journey(option: &JourneyOption) -> String {
    format!(
        "{} {}→{} {} (arr {}) then {}→{} {} (arr {}) via {}; walk {}m",
        option
            .outbound_first_leg
            .estimated_departure
            .format("%H:%M"),
        option.outbound_first_leg.from,
        option.outbound_first_leg.to,
        option.outbound_first_leg.company,
        option.outbound_first_leg.estimated_arrival.format("%H:%M"),
        option.outbound_second_leg.from,
        option.outbound_second_leg.to,
        option.outbound_second_leg.company,
        option.outbound_second_leg.estimated_arrival.format("%H:%M"),
        option.outbound_second_leg.route_destination,
        option.interchange_walk_minutes,
    )
}
