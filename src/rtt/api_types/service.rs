use std::str::FromStr;

use serde::{Deserialize, Deserializer};

use crate::timezone::DateTimeExt;

fn deserialize_optional_timestamp<'de, D>(
    deserializer: D,
) -> Result<Option<jiff::Timestamp>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    value
        .map(|value| {
            jiff::civil::DateTime::from_str(&value)
                .map_err(serde::de::Error::custom)?
                .to_london_zoned()
                .map_err(serde::de::Error::custom)
                .map(|zoned| zoned.timestamp())
        })
        .transpose()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Root {
    pub service: Service,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    pub schedule_metadata: ScheduleMetadata,
    pub locations: Vec<ServiceLocation>,
    pub destination: Vec<Destination>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceLocation {
    pub temporal_data: TemporalData,
    pub location_metadata: LocationMetadata,
    pub location: Location,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemporalData {
    pub arrival: Option<IndividualTemporalData>,
    pub departure: Option<IndividualTemporalData>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndividualTemporalData {
    #[serde(default, deserialize_with = "deserialize_optional_timestamp")]
    pub schedule_advertised: Option<jiff::Timestamp>,
    #[serde(default, deserialize_with = "deserialize_optional_timestamp")]
    pub realtime_forecast: Option<jiff::Timestamp>,
    #[serde(default, deserialize_with = "deserialize_optional_timestamp")]
    pub realtime_estimate: Option<jiff::Timestamp>,
    #[serde(default, deserialize_with = "deserialize_optional_timestamp")]
    pub realtime_actual: Option<jiff::Timestamp>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocationMetadata {
    pub platform: Option<Platform>,
    // TODO: Is this actually optional?
    pub number_of_vehicles: Option<u8>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Platform {
    pub planned: Option<String>,
    pub forecast: Option<String>,
    pub actual: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub short_codes: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Destination {
    pub location: Location,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleMetadata {
    pub operator: Operator,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Operator {
    pub name: String,
}
