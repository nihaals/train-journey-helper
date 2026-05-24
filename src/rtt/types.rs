use anyhow::{Context, Result, ensure};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use serde::Deserialize;

use crate::custom_types::{TrainCompany, TrainService};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub services: Vec<Service>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    pub location_detail: LocationDetail,
    pub service_uid: String,
    pub run_date: String,
    pub atoc_name: String,
    pub location_detail_destination: Option<Vec<ServiceLocation>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocationDetail {
    pub crs: String,
    pub gbtt_booked_departure: Option<String>,
    pub realtime_departure: Option<String>,
    pub destination: Vec<ServiceLocation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceLocation {
    pub crs: String,
    pub description: Option<String>,
    pub gbtt_booked_arrival: Option<String>,
    pub realtime_arrival: Option<String>,
}

impl TryFrom<Service> for TrainService {
    type Error = anyhow::Error;

    fn try_from(value: Service) -> Result<Self> {
        ensure!(!value.service_uid.is_empty(), "RTT service UID is empty");
        ensure!(
            value.location_detail.destination.len() == 1,
            "expected exactly one route destination for {}",
            value.service_uid
        );
        let route_destination = value
            .location_detail
            .destination
            .first()
            .and_then(|location| location.description.clone())
            .context("route destination description is missing")?;

        let to_locations = value
            .location_detail_destination
            .context("RTT response did not include destination timing details")?;
        ensure!(
            to_locations.len() == 1,
            "expected exactly one requested destination timing for {}",
            value.service_uid
        );
        let to = to_locations
            .into_iter()
            .next()
            .context("destination timing unexpectedly missing")?;
        ensure!(
            value.location_detail.crs != to.crs,
            "origin and destination CRS are both {} for {}",
            to.crs,
            value.service_uid
        );

        let run_date = NaiveDate::parse_from_str(&value.run_date, "%Y-%m-%d")
            .with_context(|| format!("invalid RTT runDate {}", value.run_date))?;

        Ok(Self {
            from: value.location_detail.crs,
            to: to.crs,
            planned_departure: parse_datetime(
                run_date,
                value
                    .location_detail
                    .gbtt_booked_departure
                    .as_deref()
                    .context("booked departure is missing")?,
            )?,
            estimated_departure: parse_datetime(
                run_date,
                value
                    .location_detail
                    .realtime_departure
                    .as_deref()
                    .or(value.location_detail.gbtt_booked_departure.as_deref())
                    .context("realtime departure is missing")?,
            )?,
            planned_arrival: parse_datetime(
                run_date,
                to.gbtt_booked_arrival
                    .as_deref()
                    .context("booked arrival is missing")?,
            )?,
            estimated_arrival: parse_datetime(
                run_date,
                to.realtime_arrival
                    .as_deref()
                    .or(to.gbtt_booked_arrival.as_deref())
                    .context("realtime arrival is missing")?,
            )?,
            company: TrainCompany::from(value.atoc_name.as_str()),
            route_destination,
        })
    }
}

fn parse_datetime(date: NaiveDate, hhmm: &str) -> Result<DateTime<FixedOffset>> {
    ensure!(hhmm.len() == 4, "expected HHMM time, got {hhmm}");
    let time = NaiveTime::parse_from_str(hhmm, "%H%M")
        .with_context(|| format!("invalid RTT time {hhmm}"))?;
    let offset = FixedOffset::east_opt(0).context("UTC offset is invalid")?;
    NaiveDateTime::new(date, time)
        .and_local_timezone(offset)
        .single()
        .context("could not construct datetime")
}
