use anyhow::Result;
use chrono::{DateTime, FixedOffset};

use crate::{config::Config, custom_types::TrainService, station::Station};

pub trait TrainProvider {
    fn new(config: &Config, client: reqwest::Client) -> Self;

    async fn departures_between(
        &self,
        from: Station,
        to: Station,
        // TODO: Use UTC internally, convert to timezone in notifications
        not_before: DateTime<FixedOffset>,
    ) -> Result<Vec<TrainService>>;
}
