use serde::Deserialize;

use crate::rtt::api_types::deserializers::deserialize_optional_timestamp;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Root {
    pub services: Vec<Service>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    pub temporal_data: ServiceTemporalData,
    pub schedule_metadata: ScheduleMetadata,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceTemporalData {
    pub departure: TemporalData,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemporalData {
    #[serde(default, deserialize_with = "deserialize_optional_timestamp")]
    pub schedule_advertised: Option<jiff::Timestamp>,
    #[serde(default, deserialize_with = "deserialize_optional_timestamp")]
    pub realtime_forecast: Option<jiff::Timestamp>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleMetadata {
    pub unique_identity: String,
    pub mode_type: String,
    pub in_passenger_service: bool,
}
