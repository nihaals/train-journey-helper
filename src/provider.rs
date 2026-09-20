use std::collections::HashMap;

use anyhow::Result;
use jiff::Timestamp;

use crate::{
    config::{Config, Stations},
    custom_types::TrainService,
    station::Station,
};

pub trait TrainProvider {
    fn new(config: &Config, client: reqwest::Client) -> Self;

    async fn departures_between(
        &self,
        from: Station,
        to: Station,
        not_before: Timestamp,
    ) -> Result<TrainServices>;

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
    pub fn new(vec: Vec<TrainService>) -> Self {
        let mut services = Self {
            by_arrival: vec.clone(),
            by_departure: vec,
        };
        services.sort();
        services
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

    /// This should only be used in tests and tracing.
    pub fn len(&self) -> usize {
        assert_eq!(self.by_arrival.len(), self.by_departure.len());
        self.by_arrival.len()
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
            .sort_by_key(|train| (train.to.estimated_arrival, train.from.estimated_departure));
        self.by_departure
            .sort_by_key(|train| (train.from.estimated_departure, train.to.estimated_arrival));
    }

    /// Removes duplicate services based on `service_id`, preferring services departing from the
    /// return-preferred interchange and otherwise keeping the first occurrence by arrival.
    pub fn dedup_by_service_id(&mut self, stations: &Stations) {
        let mut service_indices: HashMap<String, usize> = HashMap::new();
        let mut deduplicated: Vec<TrainService> = Vec::with_capacity(self.by_arrival.len());
        for service in self.by_arrival.drain(..) {
            if let Some(&index) = service_indices.get(&service.service_id) {
                if service.from.station == stations.line_one_interchange_return_preferred
                    && deduplicated[index].from.station
                        != stations.line_one_interchange_return_preferred
                {
                    deduplicated[index] = service;
                }
            } else {
                service_indices.insert(service.service_id.clone(), deduplicated.len());
                deduplicated.push(service);
            }
        }
        *self = Self::new(deduplicated);
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

    /// Returns the latest-arriving service which arrives at or before `deadline`.
    pub fn last_arriving_before(&self, deadline: Timestamp) -> Option<&TrainService> {
        self.by_arrival
            .iter()
            .rev()
            .find(|train| train.to.estimated_arrival <= deadline)
    }

    pub fn into_iter_by_arrival(self) -> std::vec::IntoIter<TrainService> {
        self.by_arrival.into_iter()
    }

    pub fn into_iter_by_departure(self) -> std::vec::IntoIter<TrainService> {
        self.by_departure.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::custom_types::{NumberOfCarriages, TrainServiceStation};

    fn station(code: &str) -> Station {
        Station::from_str(code).unwrap()
    }

    fn stations() -> Stations {
        Stations {
            home: station("HOM"),
            line_one_interchange_primary: station("PRI"),
            line_one_interchange_return_preferred: station("PRE"),
            destination_line_interchange: station("INT"),
            destination: station("DST"),
        }
    }

    fn service(service_id: &str, from: &str, departure: i64, arrival: i64) -> TrainService {
        let departure = Timestamp::from_second(departure).unwrap();
        let arrival = Timestamp::from_second(arrival).unwrap();
        TrainService {
            service_id: service_id.to_owned(),
            from: TrainServiceStation {
                station: station(from),
                scheduled_arrival: departure,
                estimated_arrival: departure,
                scheduled_departure: departure,
                estimated_departure: departure,
                platform: None,
            },
            to: TrainServiceStation {
                station: station("HOM"),
                scheduled_arrival: arrival,
                estimated_arrival: arrival,
                scheduled_departure: arrival,
                estimated_departure: arrival,
                platform: None,
            },
            company: "Company".to_owned(),
            route_destination: "Home".to_owned(),
            number_of_carriages: NumberOfCarriages::SameThroughout(4),
        }
    }

    #[test]
    fn dedup_by_service_id_prefers_return_preferred_interchange() {
        let mut services = TrainServices::new(vec![
            service("duplicate", "PRI", 10, 20),
            service("duplicate", "PRE", 15, 20),
        ]);

        {
            let services = services.first_n_by_arrival(2);
            assert_eq!(services.len(), 2);
            assert_eq!(services[0].from.station, station("PRI"));
            assert_eq!(services[1].from.station, station("PRE"));
        }

        services.dedup_by_service_id(&stations());

        let services = services.first_n_by_arrival(2);
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].from.station, station("PRE"));
    }

    #[test]
    fn dedup_by_service_id_keeps_first_by_arrival_without_preferred_interchange() {
        let mut services = TrainServices::new(vec![
            service("duplicate", "INT", 15, 20),
            service("duplicate", "PRI", 10, 20),
        ]);

        services.dedup_by_service_id(&stations());

        let services = services.first_n_by_arrival(2);
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].from.station, station("PRI"));
    }

    #[test]
    fn dedup_by_service_id_orders_results_by_arrival() {
        let mut services = TrainServices::new(vec![
            service("duplicate", "PRI", 10, 20),
            service("other", "INT", 20, 22),
            service("duplicate", "PRE", 15, 20),
        ]);

        services.dedup_by_service_id(&stations());

        let services = services.first_n_by_arrival(3);
        assert_eq!(services.len(), 2);
        assert_eq!(services[0].service_id, "duplicate");
        assert_eq!(services[0].from.station, station("PRE"));
        assert_eq!(services[1].service_id, "other");
    }
}
