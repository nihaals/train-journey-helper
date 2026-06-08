use std::{fs, net::SocketAddr, path::Path};

use anyhow::{Context, Result};
use chrono::{NaiveTime, Weekday};
use serde::Deserialize;
use validator::{Validate, ValidationError};

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct Config {
    #[validate(nested)]
    pub stations: Stations,
    pub walk: WalkTimes,
    pub destination_arrival_time: NaiveTime,
    pub travel_day: Weekday,
    pub listen_addr: SocketAddr,
    pub healthcheck_url: Option<String>,
    pub home_assistant: HomeAssistantConfig,
    pub rtt: RttConfig,
}

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct Stations {
    #[validate(custom(function = "validate_station"))]
    pub home: String,
    #[validate(custom(function = "validate_station"))]
    pub line_one_interchange_primary: String,
    #[validate(custom(function = "validate_station"))]
    pub line_one_interchange_return_preferred: String,
    #[validate(custom(function = "validate_station"))]
    pub destination_line_interchange: String,
    #[validate(custom(function = "validate_station"))]
    pub destination: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WalkTimes {
    // TODO: Use u8
    pub home_to_station_1_minutes: i64,
    pub station_2_to_4_minutes: i64,
    pub station_4_to_3_minutes: i64,
    pub station_5_to_final_destination_minutes: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HomeAssistantConfig {
    pub base_url: String,
    pub token: String,
    pub notify_service: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RttConfig {
    pub base_url: String,
    pub username: String,
    pub password: String,
}

fn validate_station(value: &str) -> std::result::Result<(), ValidationError> {
    if value.len() == 3
        && value
            .chars()
            .all(|c| c.is_ascii_alphabetic() && c.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(ValidationError::new("station"))
    }
}

impl Config {
    pub fn from_json(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config from {}", path.display()))?;
        let config: Self = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse JSON config from {}", path.display()))?;
        config.validate().context("Invalid config")?;
        Ok(config)
    }
}
