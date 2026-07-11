use jiff::Timestamp;

use crate::station::Station;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainServiceStation {
    pub station: Station,
    pub scheduled_arrival: Timestamp,
    pub estimated_arrival: Timestamp,
    pub scheduled_departure: Timestamp,
    pub estimated_departure: Timestamp,
    pub platform: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberOfCarriages {
    SameThroughout(u8),
    Varies { minimum: u8, at_from: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainService {
    pub service_id: String,
    pub from: TrainServiceStation,
    pub to: TrainServiceStation,
    pub company: String,
    /// The full station name.
    pub route_destination: String,
    pub number_of_carriages: NumberOfCarriages,
}

impl TrainService {
    pub fn departure_delay_minutes(&self) -> i64 {
        self.from
            .scheduled_departure
            .duration_until(self.from.estimated_departure)
            .as_mins()
    }

    pub fn arrival_delay_minutes(&self) -> i64 {
        self.to
            .scheduled_arrival
            .duration_until(self.to.estimated_arrival)
            .as_mins()
    }
}
