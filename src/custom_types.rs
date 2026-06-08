use jiff::Timestamp;

use crate::station::Station;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainService {
    pub from: Station,
    pub to: Station,
    pub planned_departure: Timestamp,
    pub estimated_departure: Timestamp,
    pub planned_arrival: Timestamp,
    pub estimated_arrival: Timestamp,
    pub company: TrainCompany,
    pub route_destination: String,
}

impl TrainService {
    pub fn departure_delay_minutes(&self) -> i64 {
        self.planned_departure
            .duration_until(self.estimated_departure)
            .as_mins()
    }

    pub fn arrival_delay_minutes(&self) -> i64 {
        self.planned_arrival
            .duration_until(self.estimated_arrival)
            .as_mins()
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
    pub interchange_walk_minutes: u8,
}

impl JourneyOption {
    pub fn arrives_at_destination(&self) -> Timestamp {
        self.outbound_second_leg.estimated_arrival
    }

    pub fn is_delayed(&self) -> bool {
        self.outbound_first_leg.departure_delay_minutes() > 0
            || self.outbound_first_leg.arrival_delay_minutes() > 0
            || self.outbound_second_leg.departure_delay_minutes() > 0
            || self.outbound_second_leg.arrival_delay_minutes() > 0
    }
}
