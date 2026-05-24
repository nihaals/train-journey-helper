use crate::custom_types::TrainService;
use anyhow::Result;
use chrono::{DateTime, FixedOffset};

pub trait TrainProvider: Send + Sync + 'static {
    async fn departures_between(
        &self,
        from: &str,
        to: &str,
        not_before: DateTime<FixedOffset>,
    ) -> Result<Vec<TrainService>>;
}
