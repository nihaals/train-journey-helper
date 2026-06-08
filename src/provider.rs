use anyhow::Result;
use chrono::{DateTime, FixedOffset};

use crate::{config::Config, custom_types::TrainService};

pub trait TrainProvider: Send + Sync + 'static {
    fn new(config: &Config, client: reqwest::Client) -> Self;

    async fn departures_between(
        &self,
        // TODO: Use custom type using [char; 3]
        from: &str,
        to: &str,
        // TODO: Use UTC internally, convert to timezone in notifications
        not_before: DateTime<FixedOffset>,
    ) -> Result<Vec<TrainService>>;
}
