pub mod types;

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset};

use crate::{config::RttConfig, custom_types::TrainService, provider::TrainProvider};

#[derive(Clone)]
pub struct RttClient {
    http: reqwest::Client,
    config: RttConfig,
}

impl RttClient {
    pub fn new(config: RttConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            config,
        }
    }
}

impl TrainProvider for RttClient {
    async fn departures_between(
        &self,
        from: &str,
        to: &str,
        not_before: DateTime<FixedOffset>,
    ) -> Result<Vec<TrainService>> {
        let date = not_before.format("%Y/%m/%d");
        let url = format!(
            "{}/search/{from}/to/{to}/{date}",
            self.config.base_url.trim_end_matches('/')
        );

        let response: types::SearchResponse = self
            .http
            .get(url)
            .basic_auth(&self.config.username, Some(&self.config.password))
            .send()
            .await
            .context("requesting RTT departures")?
            .error_for_status()
            .context("RTT returned an error status")?
            .json()
            .await
            .context("decoding RTT search response")?;

        response
            .services
            .into_iter()
            .map(TrainService::try_from)
            .filter(|service| {
                service
                    .as_ref()
                    .map_or(true, |service| service.estimated_departure >= not_before)
            })
            .collect()
    }
}
