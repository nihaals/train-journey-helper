use std::{fs, net::SocketAddr, path::Path};

use anyhow::{Context, Result};
use chrono::{NaiveTime, Weekday};
use serde::Deserialize;

use crate::station::Station;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub stations: Stations,
    pub walk: WalkTimes,
    pub destination_arrival_time: NaiveTime,
    pub travel_day: Weekday,
    pub listen_addr: SocketAddr,
    pub healthcheck_url: Option<String>,
    pub home_assistant: HomeAssistantConfig,
    pub rtt: RttConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Stations {
    pub home: Station,
    pub line_one_interchange_primary: Station,
    pub line_one_interchange_return_preferred: Station,
    pub destination_line_interchange: Station,
    pub destination: Station,
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

impl Config {
    pub fn from_json(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config from {}", path.display()))?;
        let config: Self = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse JSON config from {}", path.display()))?;
        Ok(config)
    }
}
