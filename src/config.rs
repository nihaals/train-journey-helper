use std::{fs, net::SocketAddr, path::Path};

use anyhow::{Context, Result};
use jiff::civil::{Time, Weekday};
use serde::{Deserialize, Deserializer, de::Error};

use crate::station::Station;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub stations: Stations,
    pub walk: WalkTimes,
    pub destination_arrival_time: Time,
    #[serde(deserialize_with = "deserialize_weekday")]
    pub travel_day: Weekday,
    pub listen_addr: SocketAddr,
    pub healthcheck_url: Option<String>,
    pub home_assistant: HomeAssistantConfig,
    pub rtt: RttConfig,
}

fn deserialize_weekday<'de, D>(deserializer: D) -> Result<Weekday, D::Error>
where
    D: Deserializer<'de>,
{
    match String::deserialize(deserializer)?.as_str() {
        "monday" => Ok(Weekday::Monday),
        "tuesday" => Ok(Weekday::Tuesday),
        "wednesday" => Ok(Weekday::Wednesday),
        "thursday" => Ok(Weekday::Thursday),
        "friday" => Ok(Weekday::Friday),
        "saturday" => Ok(Weekday::Saturday),
        "sunday" => Ok(Weekday::Sunday),
        other => Err(D::Error::custom(format!("invalid weekday {other}"))),
    }
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
    pub home_to_station_1_minutes: u8,
    pub station_2_to_4_minutes: u8,
    pub station_4_to_3_minutes: u8,
    pub station_5_to_final_destination_minutes: u8,
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

#[cfg(test)]
mod tests {
    use jiff::civil::{Time, Weekday};

    use super::Config;

    #[test]
    fn deserializes_time_and_weekday() {
        let config: Config = serde_json::from_str(
            r#"
            {
              "stations": {
                "home": "AAA",
                "line_one_interchange_primary": "BBB",
                "line_one_interchange_return_preferred": "CCC",
                "destination_line_interchange": "DDD",
                "destination": "EEE"
              },
              "walk": {
                "home_to_station_1_minutes": 1,
                "station_2_to_4_minutes": 2,
                "station_4_to_3_minutes": 3,
                "station_5_to_final_destination_minutes": 4
              },
              "destination_arrival_time": "09:30:00.0",
              "travel_day": "monday",
              "listen_addr": "127.0.0.1:3000",
              "healthcheck_url": null,
              "home_assistant": {
                "base_url": "https://example.com",
                "token": "token",
                "notify_service": "notify.mobile_app"
              },
              "rtt": {
                "base_url": "https://example.com/rtt",
                "username": "user",
                "password": "pass"
              }
            }
            "#,
        )
        .unwrap();

        assert_eq!(
            config.destination_arrival_time,
            Time::new(9, 30, 0, 0).unwrap()
        );
        assert_eq!(config.travel_day, Weekday::Monday);
    }
}
