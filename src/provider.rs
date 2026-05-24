use anyhow::Result;
use chrono::{DateTime, FixedOffset};

use crate::custom_types::TrainService;

pub trait TrainProvider: Send + Sync + 'static {
    async fn departures_between(
        &self,
        from: &str,
        to: &str,
        not_before: DateTime<FixedOffset>,
    ) -> Result<Vec<TrainService>>;
}
