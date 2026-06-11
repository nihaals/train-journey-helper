use serde::Deserialize;

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
    // TODO: Is it actually optional?
    pub departure: Option<TemporalData>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemporalData {
    pub schedule_advertised: Option<String>,
    pub realtime_forecast: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleMetadata {
    pub unique_identity: String,
    pub mode_type: String,
    pub in_passenger_service: bool,
}
