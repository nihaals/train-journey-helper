use anyhow::{Context, Result, ensure};
use chrono::{NaiveTime, Weekday};
use std::{env, net::SocketAddr};

#[derive(Debug, Clone)]
pub struct Config {
    pub stations: Stations,
    pub walk: WalkTimes,
    pub destination_arrival_time: NaiveTime,
    pub travel_day: Weekday,
    pub listen_addr: SocketAddr,
    pub home_assistant: HomeAssistantConfig,
    pub rtt: RttConfig,
}

#[derive(Debug, Clone)]
pub struct Stations {
    pub home: String,
    pub line_one_interchange_primary: String,
    pub line_one_interchange_return_preferred: String,
    pub destination_line_interchange: String,
    pub destination: String,
}

#[derive(Debug, Clone)]
pub struct WalkTimes {
    pub home_to_station_1_minutes: i64,
    pub station_2_to_4_minutes: i64,
    pub station_4_to_3_minutes: i64,
    pub station_5_to_final_destination_minutes: i64,
}

#[derive(Debug, Clone)]
pub struct HomeAssistantConfig {
    pub base_url: String,
    pub token: String,
    pub notify_service: String,
}

#[derive(Debug, Clone)]
pub struct RttConfig {
    pub base_url: String,
    pub username: String,
    pub password: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let stations = Stations {
            home: station("TRAIN_STATION_1")?,
            line_one_interchange_primary: station("TRAIN_STATION_2")?,
            line_one_interchange_return_preferred: station("TRAIN_STATION_3")?,
            destination_line_interchange: station("TRAIN_STATION_4")?,
            destination: station("TRAIN_STATION_5")?,
        };

        Ok(Self {
            stations,
            walk: WalkTimes {
                home_to_station_1_minutes: parse_env("WALK_HOME_TO_STATION_1_MINUTES")?,
                station_2_to_4_minutes: parse_env("WALK_STATION_2_TO_4_MINUTES")?,
                station_4_to_3_minutes: parse_env("WALK_STATION_4_TO_3_MINUTES")?,
                station_5_to_final_destination_minutes: parse_env(
                    "WALK_STATION_5_TO_FINAL_DESTINATION_MINUTES",
                )?,
            },
            destination_arrival_time: env::var("DESTINATION_ARRIVAL_TIME")
                .context("DESTINATION_ARRIVAL_TIME is required, e.g. 18:00")?
                .parse()
                .context("DESTINATION_ARRIVAL_TIME must be HH:MM[:SS]")?,
            travel_day: parse_weekday(&env::var("TRAVEL_DAY").context("TRAVEL_DAY is required")?)?,
            listen_addr: env::var("LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:3000".to_string())
                .parse()
                .context("LISTEN_ADDR must be host:port")?,
            home_assistant: HomeAssistantConfig {
                base_url: env::var("HOME_ASSISTANT_BASE_URL")
                    .context("HOME_ASSISTANT_BASE_URL is required")?,
                token: env::var("HOME_ASSISTANT_TOKEN")
                    .context("HOME_ASSISTANT_TOKEN is required")?,
                notify_service: env::var("HOME_ASSISTANT_NOTIFY_SERVICE")
                    .unwrap_or_else(|_| "mobile_app_phone".to_string()),
            },
            rtt: RttConfig {
                base_url: env::var("RTT_BASE_URL")
                    .unwrap_or_else(|_| "https://api.rtt.io/api/v1/json".to_string()),
                username: env::var("RTT_USERNAME").context("RTT_USERNAME is required")?,
                password: env::var("RTT_PASSWORD").context("RTT_PASSWORD is required")?,
            },
        })
    }
}

fn station(key: &str) -> Result<String> {
    let value = env::var(key).with_context(|| format!("{key} is required"))?;
    ensure!(value.len() == 3, "{key} must be a 3-letter CRS code");
    ensure!(
        value.chars().all(|c| c.is_ascii_alphabetic()),
        "{key} must only contain letters"
    );
    Ok(value.to_ascii_uppercase())
}

fn parse_env<T>(key: &str) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    env::var(key)
        .with_context(|| format!("{key} is required"))?
        .parse()
        .with_context(|| format!("{key} is invalid"))
}

fn parse_weekday(value: &str) -> Result<Weekday> {
    match value.trim().to_ascii_lowercase().as_str() {
        "mon" | "monday" => Ok(Weekday::Mon),
        "tue" | "tues" | "tuesday" => Ok(Weekday::Tue),
        "wed" | "wednesday" => Ok(Weekday::Wed),
        "thu" | "thur" | "thurs" | "thursday" => Ok(Weekday::Thu),
        "fri" | "friday" => Ok(Weekday::Fri),
        "sat" | "saturday" => Ok(Weekday::Sat),
        "sun" | "sunday" => Ok(Weekday::Sun),
        _ => anyhow::bail!("TRAVEL_DAY must be a weekday name"),
    }
}
