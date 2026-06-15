mod api_types;

use anyhow::{Context, Result, bail, ensure};
use jiff::{Span, Timestamp};
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::{
    config::{Config, RttConfig},
    custom_types::{NumberOfCarriages, TrainService, TrainServiceStation},
    provider::TrainProvider,
    station::Station,
};

fn base64_decode_url_safe_no_pad(input: &str) -> Result<Vec<u8>, base64::DecodeError> {
    base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, input)
}

fn parse_jwt_payload<T>(token: &str) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let parts: Vec<&str> = token.split('.').collect();
    ensure!(parts.len() == 3, "invalid JWT format");
    let payload =
        base64_decode_url_safe_no_pad(parts[1]).context("Failed to base64 decode JWT payload")?;
    serde_json::from_slice(&payload).context("Failed to deserialize JWT payload")
}

#[derive(Deserialize)]
struct AccessTokenPayload {
    #[serde(rename = "exp")]
    expiry: i64,
}

impl AccessTokenPayload {
    fn expiry(&self) -> Result<Timestamp, jiff::Error> {
        Timestamp::from_second(self.expiry)
    }
}

/// Returns true if the access token will expire within the next minute.
fn access_token_needs_refresh(token: &str) -> Result<bool> {
    let payload: AccessTokenPayload = parse_jwt_payload(token)?;
    let cutoff = Timestamp::now().checked_add(Span::new().minutes(1))?;
    Ok(payload.expiry()? <= cutoff)
}

pub struct RttClient {
    http: reqwest::Client,
    config: RttConfig,
    access_token: Mutex<Option<String>>,
}

impl RttClient {
    async fn get_access_token(&self) -> Result<String> {
        let response: api_types::get_access_token::Root = self
            .http
            .get("https://data.rtt.io/api/get_access_token")
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("Failed to send RTT access token request")?
            .error_for_status()
            .context("Failed to get RTT access token")?
            .json()
            .await
            .context("Failed to deserialize RTT auth response")?;
        Ok(response.token)
    }

    async fn access_token(&self) -> Result<String> {
        let mut access_token = self.access_token.lock().await;
        match access_token.as_deref() {
            Some(token) if !access_token_needs_refresh(token)? => Ok(token.to_owned()),
            _ => {
                let token = self.get_access_token().await?;
                *access_token = Some(token.clone());
                Ok(token)
            }
        }
    }

    async fn service(
        &self,
        access_token: &str,
        unique_identity: &str,
    ) -> Result<api_types::service::Root> {
        // TODO: Cache for less than poll time
        let unique_identity = unique_identity
            .strip_prefix("gb-nr:")
            .unwrap_or(unique_identity);
        self.http
            .get("https://data.rtt.io/gb-nr/service")
            .bearer_auth(access_token)
            .query(&[("uniqueIdentity", unique_identity)])
            .send()
            .await
            .context("Failed to send RTT service request")?
            .error_for_status()
            .context("Failed to get RTT service data")?
            .json()
            .await
            .context("Failed to deserialize RTT service response")
    }
}

fn best_time(data: &api_types::service::IndividualTemporalData) -> Option<Timestamp> {
    data.realtime_actual
        .or(data.realtime_forecast)
        .or(data.schedule_advertised)
}

// TODO: Why do we need a function for this
fn advertised_time(data: &api_types::service::IndividualTemporalData) -> Option<Timestamp> {
    data.schedule_advertised
}

fn location_matches_station(
    location: &api_types::service::ServiceLocation,
    station: Station,
) -> bool {
    location
        .location
        .short_codes
        .iter()
        .any(|code| code == station.as_str())
}

fn service_location_index(
    service: &api_types::service::Service,
    station: Station,
) -> Result<usize> {
    service
        .locations
        .iter()
        .position(|location| location_matches_station(location, station))
        .with_context(|| format!("RTT service did not include stop {station}"))
}

fn service_location(
    service: &api_types::service::Service,
    station: Station,
) -> Result<&api_types::service::ServiceLocation> {
    Ok(&service.locations[service_location_index(service, station)?])
}

fn service_location_is_cancelled(location: &api_types::service::ServiceLocation) -> Result<bool> {
    match (
        location.temporal_data.arrival.as_ref(),
        location.temporal_data.departure.as_ref(),
    ) {
        (Some(arrival), Some(departure)) => {
            ensure!(
                arrival.is_cancelled == departure.is_cancelled,
                "RTT service stop has inconsistent arrival/departure cancellation status",
            );
            Ok(arrival.is_cancelled)
        }
        (Some(data), None) | (None, Some(data)) => Ok(data.is_cancelled),
        (None, None) => bail!("RTT service stop did not include arrival/departure data"),
    }
}

fn service_station(
    service: &api_types::service::Service,
    station: Station,
) -> Result<Option<TrainServiceStation>> {
    let location = service_location(service, station)?;
    if service_location_is_cancelled(location)? {
        return Ok(None);
    }

    let arrival = location
        .temporal_data
        .arrival
        .as_ref()
        .or(location.temporal_data.departure.as_ref())
        .with_context(|| format!("RTT service stop {station} did not include arrival/departure"))?;
    let departure = location
        .temporal_data
        .departure
        .as_ref()
        .or(location.temporal_data.arrival.as_ref())
        .with_context(|| format!("RTT service stop {station} did not include departure/arrival"))?;

    let platform = location
        .location_metadata
        .platform
        .as_ref()
        .and_then(|platform| {
            platform
                .actual
                .as_ref()
                .filter(|actual| !actual.is_empty())
                .or_else(|| {
                    platform
                        .forecast
                        .as_ref()
                        .filter(|planned| !planned.is_empty())
                        .or_else(|| {
                            platform
                                .planned
                                .as_ref()
                                .filter(|forecast| !forecast.is_empty())
                        })
                })
        })
        .cloned();

    Ok(Some(TrainServiceStation {
        station,
        scheduled_arrival: advertised_time(arrival).with_context(|| {
            format!("RTT service stop {station} did not include scheduled arrival")
        })?,
        estimated_arrival: best_time(arrival).with_context(|| {
            format!("RTT service stop {station} did not include estimated arrival")
        })?,
        scheduled_departure: advertised_time(departure).with_context(|| {
            format!("RTT service stop {station} did not include scheduled departure")
        })?,
        estimated_departure: best_time(departure).with_context(|| {
            format!("RTT service stop {station} did not include estimated departure")
        })?,
        platform,
    }))
}

fn number_of_carriages_for_journey(
    service: &api_types::service::Service,
    from: Station,
    to: Station,
) -> Result<NumberOfCarriages> {
    let from_index = service_location_index(service, from)?;
    let to_index = service_location_index(service, to)?;

    ensure!(
        from_index < to_index,
        "RTT service stop {from} was not before stop {to}",
    );

    let locations = service.locations[from_index..to_index]
        .iter()
        .filter_map(|location| match service_location_is_cancelled(location) {
            Ok(false) => Some(Ok(location)),
            Ok(true) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>>>()?;
    let at_from = locations
        .first()
        .expect("from should not be cancelled")
        .location_metadata
        .number_of_vehicles;

    if locations
        .iter()
        .all(|location| location.location_metadata.number_of_vehicles == at_from)
    {
        return Ok(NumberOfCarriages::SameThroughout(at_from));
    }

    let minimum = locations
        .iter()
        .map(|location| location.location_metadata.number_of_vehicles)
        .min()
        .context("RTT service did not include any locations between journey stops")?;

    Ok(NumberOfCarriages::Varies { minimum, at_from })
}

fn train_service_from_rtt(
    service: api_types::service::Service,
    from: Station,
    to: Station,
) -> Result<Option<TrainService>> {
    let Some(from_station) = service_station(&service, from)? else {
        return Ok(None);
    };
    let Some(to_station) = service_station(&service, to)? else {
        return Ok(None);
    };
    let destination = service
        .destination
        .first()
        .context("RTT service did not include destination")?;
    let number_of_carriages = number_of_carriages_for_journey(&service, from, to)?;

    Ok(Some(TrainService {
        service_id: service.schedule_metadata.unique_identity.clone(),
        from: from_station,
        to: to_station,
        company: service.schedule_metadata.operator.name.clone(),
        route_destination: destination.location.description.clone(),
        number_of_carriages,
    }))
}

impl TrainProvider for RttClient {
    fn new(config: &Config, client: reqwest::Client) -> Self {
        Self {
            http: client,
            config: config.rtt.clone(),
            access_token: Mutex::new(None),
        }
    }

    async fn departures_between(
        &self,
        from: Station,
        to: Station,
        not_before: Timestamp,
    ) -> Result<Vec<TrainService>> {
        // TODO: Cache for less than poll time
        let access_token = self.access_token().await?;
        let response: api_types::location::Root = self
            .http
            .get("https://data.rtt.io/gb-nr/location")
            .bearer_auth(&access_token)
            .query(&[
                ("code", from.as_str()),
                ("filterTo", to.as_str()),
                ("timeFrom", &not_before.to_string()),
            ])
            .send()
            .await
            .context("Failed to send RTT request")?
            .error_for_status()
            .context("Failed to get RTT data")?
            .json()
            .await
            .context("Failed to deserialize RTT response")?;

        let mut trains = Vec::new();
        for service in response.services {
            if service.schedule_metadata.mode_type == "BUS" {
                continue;
            }

            ensure!(
                service.schedule_metadata.in_passenger_service,
                "RTT service is not in passenger service"
            );
            ensure!(
                service.schedule_metadata.mode_type == "TRAIN",
                "RTT service mode type is not TRAIN"
            );

            let departure_data = service.temporal_data.departure;
            let Some(departure) = departure_data
                .realtime_forecast
                .or(departure_data.schedule_advertised)
            else {
                continue;
            };
            if departure < not_before {
                continue;
            }

            // TODO: Concurrency
            let service = self
                .service(&access_token, &service.schedule_metadata.unique_identity)
                .await?;
            if let Some(service) = train_service_from_rtt(service.service, from, to)? {
                trains.push(service);
            }
        }

        Ok(trains)
    }

    async fn get_service(
        &self,
        service_id: &str,
        from: Station,
        to: Station,
    ) -> Result<TrainService> {
        let access_token = self.access_token().await?;
        let service = self.service(&access_token, service_id).await?;
        train_service_from_rtt(service.service, from, to)?
            .context("Failed to get train service from RTT")
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn station(code: &str) -> Station {
        Station::from_str(code).unwrap()
    }

    fn temporal_data(is_cancelled: bool) -> api_types::service::IndividualTemporalData {
        api_types::service::IndividualTemporalData {
            is_cancelled,
            schedule_advertised: Some(Timestamp::from_second(0).unwrap()),
            realtime_forecast: None,
            realtime_actual: None,
        }
    }

    fn service_location(
        code: &str,
        number_of_vehicles: u8,
        is_cancelled: bool,
    ) -> api_types::service::ServiceLocation {
        api_types::service::ServiceLocation {
            temporal_data: api_types::service::TemporalData {
                arrival: Some(temporal_data(is_cancelled)),
                departure: Some(temporal_data(is_cancelled)),
            },
            location_metadata: api_types::service::LocationMetadata {
                platform: None,
                number_of_vehicles,
            },
            location: api_types::service::Location {
                description: code.to_owned(),
                short_codes: vec![code.to_owned()],
            },
        }
    }

    fn service_with_locations(
        locations: Vec<api_types::service::ServiceLocation>,
    ) -> api_types::service::Service {
        api_types::service::Service {
            schedule_metadata: api_types::service::ScheduleMetadata {
                unique_identity: "service-id".to_owned(),
                operator: api_types::service::Operator {
                    name: "Operator".to_owned(),
                },
            },
            locations,
            destination: vec![api_types::service::Destination {
                location: api_types::service::Location {
                    description: "Destination".to_owned(),
                    short_codes: vec!["DST".to_owned()],
                },
            }],
        }
    }

    fn service_with_carriages(stops: &[(&str, u8)]) -> api_types::service::Service {
        service_with_locations(
            stops
                .iter()
                .map(|(code, number_of_vehicles)| {
                    service_location(code, *number_of_vehicles, false)
                })
                .collect(),
        )
    }

    #[test]
    fn number_of_carriages_uses_minimum_from_from_until_before_to() {
        let service = service_with_carriages(&[("AAA", 8), ("BBB", 4), ("CCC", 12), ("DDD", 2)]);
        let carriages =
            number_of_carriages_for_journey(&service, station("AAA"), station("DDD")).unwrap();
        assert_eq!(
            carriages,
            NumberOfCarriages::Varies {
                minimum: 4,
                at_from: 8
            }
        );
    }

    #[test]
    fn number_of_carriages_includes_drop_at_from() {
        let service = service_with_carriages(&[("AAA", 8), ("BBB", 4), ("CCC", 8)]);
        let carriages =
            number_of_carriages_for_journey(&service, station("BBB"), station("CCC")).unwrap();
        assert_eq!(carriages, NumberOfCarriages::SameThroughout(4));
    }

    #[test]
    fn number_of_carriages_excludes_drop_at_to() {
        let service = service_with_carriages(&[("AAA", 8), ("BBB", 8), ("CCC", 4)]);
        let carriages =
            number_of_carriages_for_journey(&service, station("AAA"), station("CCC")).unwrap();
        assert_eq!(carriages, NumberOfCarriages::SameThroughout(8));
    }

    #[test]
    fn number_of_carriages_ignores_cancelled_intermediate_stops() {
        let service = service_with_locations(vec![
            service_location("AAA", 8, false),
            service_location("BBB", 4, true),
            service_location("CCC", 8, false),
            service_location("DDD", 8, false),
        ]);
        let carriages =
            number_of_carriages_for_journey(&service, station("AAA"), station("DDD")).unwrap();
        assert_eq!(carriages, NumberOfCarriages::SameThroughout(8));
    }

    #[test]
    fn train_service_does_not_exist_when_from_is_cancelled() {
        let service = service_with_locations(vec![
            service_location("AAA", 8, true),
            service_location("BBB", 8, false),
        ]);
        let service = train_service_from_rtt(service, station("AAA"), station("BBB")).unwrap();
        assert!(service.is_none());
    }

    #[test]
    fn train_service_does_not_exist_when_to_is_cancelled() {
        let service = service_with_locations(vec![
            service_location("AAA", 8, false),
            service_location("BBB", 8, true),
        ]);
        let service = train_service_from_rtt(service, station("AAA"), station("BBB")).unwrap();
        assert!(service.is_none());
    }

    #[test]
    fn cancellation_status_must_match_between_arrival_and_departure() {
        let mut location = service_location("AAA", 8, false);
        location
            .temporal_data
            .arrival
            .as_mut()
            .unwrap()
            .is_cancelled = true;
        let error = service_location_is_cancelled(&location).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("inconsistent arrival/departure cancellation status")
        );
    }
}
