use anyhow::{Context, Result};
use serde::Serialize;

use crate::{
    config::{Config, HomeAssistantConfig},
    notifier::Notifier,
};

#[derive(Clone)]
pub struct HomeAssistantNotifier {
    http: reqwest::Client,
    config: HomeAssistantConfig,
}

#[derive(Debug, Serialize)]
struct NotifyRequest<'a> {
    title: &'a str,
    message: &'a str,
    data: NotifyData<'a>,
}

#[derive(Debug, Serialize)]
struct NotifyData<'a> {
    // TODO: Tag should be the same for all notifications related to the same journey/day
    tag: &'a str,
    // TODO: Static
    group: Option<&'a str>,
}

impl HomeAssistantNotifier {
    async fn send(&self, request: &NotifyRequest<'_>) -> Result<()> {
        let url = format!(
            "{}services/notify/{}",
            self.config.base_url, self.config.notify_service,
        );
        self.http
            .post(url)
            .bearer_auth(&self.config.token)
            .json(request)
            .send()
            .await
            .context("Failed to send Home Assistant notification")?
            .error_for_status()
            .context("Failed to trigger Home Assistant notification")?;
        Ok(())
    }
}

impl Notifier for HomeAssistantNotifier {
    fn new(config: &Config, client: reqwest::Client) -> Self {
        Self {
            config: config.home_assistant.clone(),
            http: client,
        }
    }

    async fn send_notification(
        &self,
        title: &str,
        message: &str,
        tag: &str,
        group: &str,
    ) -> Result<()> {
        self.send(&NotifyRequest {
            title,
            message,
            data: NotifyData {
                tag,
                group: Some(group),
            },
        })
        .await
    }

    async fn clear_notification(&self, tag: &str) -> Result<()> {
        self.send(&NotifyRequest {
            title: "",
            message: "clear_notification",
            data: NotifyData { tag, group: None },
        })
        .await
        .context("Failed to clear Home Assistant notification")
    }
}
