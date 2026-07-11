use std::collections::HashSet;

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
        // TODO: Consider switching to iterator or `TrainServices`
    ) -> Result<Vec<TrainService>>;

    async fn get_service(
        &self,
        service_id: &str,
        from: Station,
        to: Station,
    ) -> Result<TrainService>;

    async fn purge_cache(&self);
}

/// A wrapper around [`Vec<TrainService>`] which requires specifying the sort order to access the
/// data.
pub struct TrainServices {
    by_arrival: Vec<TrainService>,
    by_departure: Vec<TrainService>,
}

impl TrainServices {
    pub fn new(mut vec: Vec<TrainService>) -> Self {
        vec.sort_by_key(|train| train.to.estimated_arrival);
        let mut by_departure = vec.clone();
        by_departure.sort_by_key(|train| train.from.estimated_departure);
        Self {
            by_arrival: vec,
            by_departure,
        }
    }

    pub fn new_empty() -> Self {
        Self {
            by_arrival: Vec::new(),
            by_departure: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        assert_eq!(self.by_arrival.len(), self.by_departure.len());
        self.by_arrival.is_empty()
    }

    pub fn push(&mut self, service: TrainService) {
        self.by_arrival.push(service.clone());
        self.by_departure.push(service);
        self.sort();
    }

    pub fn extend(&mut self, other: TrainServices) {
        self.by_arrival.extend(other.by_arrival);
        self.by_departure.extend(other.by_departure);
        self.sort();
    }

    fn sort(&mut self) {
        self.by_arrival
            .sort_by_key(|train| train.to.estimated_arrival);
        self.by_departure
            .sort_by_key(|train| train.from.estimated_departure);
    }

    /// Removes duplicate services based on `service_id`, keeping the first occurrence when sorting
    /// by arrival.
    pub fn dedup_by_service_id(&mut self) {
        let mut service_ids = HashSet::new();
        self.by_arrival
            .retain(|service| service_ids.insert(service.service_id.clone()));
        self.by_departure = self.by_arrival.clone();
        self.by_departure
            .sort_by_key(|train| train.from.estimated_departure);
    }

    pub fn first_by_departure(&self) -> Option<&TrainService> {
        self.by_departure.first()
    }

    pub fn first_n_by_arrival(&self, n: usize) -> &[TrainService] {
        &self.by_arrival[..n.min(self.by_arrival.len())]
    }

    pub fn first_n_by_departure(&self, n: usize) -> &[TrainService] {
        &self.by_departure[..n.min(self.by_departure.len())]
    }

    pub fn last_by_departure(&self) -> Option<&TrainService> {
        self.by_departure.last()
    }

    pub fn into_iter_by_arrival(self) -> std::vec::IntoIter<TrainService> {
        self.by_arrival.into_iter()
    }

    pub fn into_iter_by_departure(self) -> std::vec::IntoIter<TrainService> {
        self.by_departure.into_iter()
    }
}
