use chrono::{DateTime, FixedOffset};

use crate::station::Station;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainService {
    pub from: Station,
    pub to: Station,
    pub planned_departure: DateTime<FixedOffset>,
    pub estimated_departure: DateTime<FixedOffset>,
    pub planned_arrival: DateTime<FixedOffset>,
    pub estimated_arrival: DateTime<FixedOffset>,
    pub company: TrainCompany,
    pub route_destination: String,
}

impl TrainService {
    pub fn departure_delay_minutes(&self) -> i64 {
        (self.estimated_departure - self.planned_departure).num_minutes()
    }

    pub fn arrival_delay_minutes(&self) -> i64 {
        (self.estimated_arrival - self.planned_arrival).num_minutes()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrainCompany {
    ChilternRailways,
    CrossCountry,
    LondonNorthwesternRailway,
    WestMidlandsRailway,
    Other(String),
}

impl From<&str> for TrainCompany {
    fn from(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "chiltern railways" | "chiltern" => Self::ChilternRailways,
            "crosscountry" | "cross country" => Self::CrossCountry,
            "london northwestern railway" | "london northwestern" => {
                Self::LondonNorthwesternRailway
            }
            "west midlands railway" => Self::WestMidlandsRailway,
            other => Self::Other(other.to_string()),
        }
    }
}

impl std::fmt::Display for TrainCompany {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChilternRailways => write!(f, "Chiltern Railways"),
            Self::CrossCountry => write!(f, "CrossCountry"),
            Self::LondonNorthwesternRailway => write!(f, "London Northwestern Railway"),
            Self::WestMidlandsRailway => write!(f, "West Midlands Railway"),
            Self::Other(value) => write!(f, "{value}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JourneyOption {
    pub outbound_first_leg: TrainService,
    pub outbound_second_leg: TrainService,
    pub interchange_walk_minutes: i64,
}

impl JourneyOption {
    pub fn arrives_at_destination(&self) -> DateTime<FixedOffset> {
        self.outbound_second_leg.estimated_arrival
    }

    pub fn is_delayed(&self) -> bool {
        self.outbound_first_leg.departure_delay_minutes() > 0
            || self.outbound_first_leg.arrival_delay_minutes() > 0
            || self.outbound_second_leg.departure_delay_minutes() > 0
            || self.outbound_second_leg.arrival_delay_minutes() > 0
    }
}
