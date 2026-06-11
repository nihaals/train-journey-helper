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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainService {
    pub from: TrainServiceStation,
    pub to: TrainServiceStation,
    pub company: String,
    /// The full station name.
    pub route_destination: String,
    pub number_of_carriages: u8,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JourneyOption {
    pub outbound_first_leg: TrainService,
    pub outbound_second_leg: TrainService,
    pub interchange_walk_minutes: u8,
}

impl JourneyOption {
    pub fn arrives_at_destination(&self) -> Timestamp {
        self.outbound_second_leg.to.estimated_arrival
    }

    pub fn is_delayed(&self) -> bool {
        self.outbound_first_leg.departure_delay_minutes() > 0
            || self.outbound_first_leg.arrival_delay_minutes() > 0
            || self.outbound_second_leg.departure_delay_minutes() > 0
            || self.outbound_second_leg.arrival_delay_minutes() > 0
    }
}
