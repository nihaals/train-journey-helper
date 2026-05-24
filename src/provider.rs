use crate::custom_types::TrainService;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, FixedOffset};

#[async_trait]
pub trait TrainProvider: Send + Sync + 'static {
    async fn departures_between(
        &self,
        from: &str,
        to: &str,
        not_before: DateTime<FixedOffset>,
    ) -> Result<Vec<TrainService>>;
}
