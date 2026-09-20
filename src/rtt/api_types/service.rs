use serde::Deserialize;

use crate::rtt::api_types::deserializers::deserialize_optional_timestamp;

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
    pub is_cancelled: bool,
    #[serde(default, deserialize_with = "deserialize_optional_timestamp")]
    pub schedule_advertised: Option<jiff::Timestamp>,
    #[serde(default, deserialize_with = "deserialize_optional_timestamp")]
    pub realtime_forecast: Option<jiff::Timestamp>,
    #[serde(default, deserialize_with = "deserialize_optional_timestamp")]
    pub realtime_actual: Option<jiff::Timestamp>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocationMetadata {
    pub platform: Option<Platform>,
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
    pub unique_identity: String,
    pub operator: Operator,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Operator {
    pub name: String,
}
