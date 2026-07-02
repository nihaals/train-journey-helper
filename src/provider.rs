use anyhow::Result;
use jiff::Timestamp;

use crate::{config::Config, custom_types::TrainService, station::Station};

pub trait TrainProvider {
    fn new(config: &Config, client: reqwest::Client) -> Self;

    async fn departures_between(
        &self,
        from: Station,
        to: Station,
        not_before: Timestamp,
    ) -> Result<Vec<TrainService>>;

    async fn get_service(
        &self,
        service_id: &str,
        from: Station,
        to: Station,
    ) -> Result<TrainService>;

    async fn purge_cache(&self);
}
