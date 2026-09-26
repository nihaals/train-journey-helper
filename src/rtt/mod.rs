mod api_types;

use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result, bail, ensure};
use jiff::Timestamp;
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::{Span, debug, debug_span, info, warn};

use crate::{
    config::{Config, RttConfig},
    custom_types::{NumberOfCarriages, TrainService, TrainServiceStation},
    provider::{TrainProvider, TrainServices},
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
    let cutoff = Timestamp::now().checked_add(jiff::Span::new().minutes(1))?;
    Ok(payload.expiry()? <= cutoff)
}

/// RTT returns `X-RateLimit-Limit-<Minute|Hour|Day|Week>`,
/// `X-RateLimit-Remaining-<Minute|Hour|Day|Week>`, and `Retry-After` on 429s.
fn format_rate_limit_headers(headers: &reqwest::header::HeaderMap) -> String {
    let relevant = headers
        .iter()
        .filter(|(name, _)| {
            let name = name.as_str();
            name.starts_with("x-ratelimit-") || name == "retry-after"
        })
        .map(|(name, value)| {
            format!(
                "{name}: {}",
                value.to_str().unwrap_or("<non-utf-8 header value>"),
            )
        })
        .collect::<Vec<_>>();
    if relevant.is_empty() {
        "<no rate-limit headers>".to_owned()
    } else {
        relevant.join(", ")
    }
}

/// Extension to send an RTT request, check the response status, and deserialize the JSON body.
/// `operation` is used for logging and error context, e.g. "RTT service request for
/// {unique_identity}".
trait RttRequestBuilder {
    async fn fetch_rtt_json<T>(self, operation: &str) -> Result<T>
    where
        T: serde::de::DeserializeOwned;
}

impl RttRequestBuilder for reqwest::RequestBuilder {
    async fn fetch_rtt_json<T>(self, operation: &str) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let response = self
            .send()
            .await
            .with_context(|| format!("Failed to send {operation}"))?;
        let status = response.status();
        let url = response.url().clone();
        let rate_limit_headers = format_rate_limit_headers(response.headers());
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read body>".to_owned());
            warn!(
                %status,
                %url,
                rate_limit_headers,
                body,
                operation,
                "RTT request failed",
            );
            bail!("failed {operation}: got {status}");
        }
        debug!(%status, %url, rate_limit_headers, operation, "RTT request succeeded");
        response
            .json()
            .await
            .with_context(|| format!("Failed to deserialize {operation} response"))
    }
}

pub struct RttClient {
    http: reqwest::Client,
    config: RttConfig,
    access_token: Mutex<Option<String>>,
    service_cache: Mutex<HashMap<String, Arc<api_types::service::Root>>>,
    location_cache: Mutex<HashMap<LocationCacheKey, Arc<api_types::location::Root>>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct LocationCacheKey {
    from: Station,
    to: Station,
    not_before: Timestamp,
}

impl RttClient {
    #[tracing::instrument(skip(self))]
    async fn get_access_token(&self) -> Result<String> {
        let response: api_types::get_access_token::Root = self
            .http
            .get("https://data.rtt.io/api/get_access_token")
            .bearer_auth(&self.config.token)
            .fetch_rtt_json("RTT access token request")
            .await?;
        debug!("received new RTT access token");
        Ok(response.token)
    }

    #[tracing::instrument(skip(self))]
    async fn access_token(&self) -> Result<String> {
        let mut access_token = self.access_token.lock().await;
        if let Some(token) = access_token.as_deref() {
            if !access_token_needs_refresh(token)? {
                debug!("reusing cached RTT access token");
                return Ok(token.to_owned());
            }
            info!("refreshing expiring RTT access token");
        } else {
            info!("fetching initial RTT access token");
        }
        let token = self.get_access_token().await?;
        *access_token = Some(token.clone());
        Ok(token)
    }

    #[tracing::instrument(skip(self, access_token))]
    async fn service(
        &self,
        access_token: &str,
        unique_identity: &str,
    ) -> Result<Arc<api_types::service::Root>> {
        let unique_identity = unique_identity
            .strip_prefix("gb-nr:")
            .unwrap_or(unique_identity);
        if let Some(service) = self
            .service_cache
            .lock()
            .await
            .get(unique_identity)
            .cloned()
        {
            debug!("RTT service cache hit");
            return Ok(service);
        }

        let response: Arc<api_types::service::Root> = Arc::new(
            self.http
                .get("https://data.rtt.io/gb-nr/service")
                .bearer_auth(access_token)
                .query(&[("uniqueIdentity", unique_identity)])
                .fetch_rtt_json(&format!("RTT service request for {unique_identity}"))
                .await?,
        );
        self.service_cache
            .lock()
            .await
            .insert(unique_identity.to_owned(), Arc::clone(&response));
        Ok(response)
    }

    #[tracing::instrument(
        skip(self, access_token),
        fields(service_count = tracing::field::Empty),
    )]
    async fn location(
        &self,
        access_token: &str,
        from: Station,
        to: Station,
        not_before: Timestamp,
    ) -> Result<Arc<api_types::location::Root>> {
        let key = LocationCacheKey {
            from,
            to,
            not_before,
        };
        if let Some(location) = self.location_cache.lock().await.get(&key).cloned() {
            debug!("RTT location cache hit");
            return Ok(location);
        }

        let response: Arc<api_types::location::Root> = Arc::new(
            self.http
                .get("https://data.rtt.io/gb-nr/location")
                .bearer_auth(access_token)
                .query(&[
                    ("code", from.as_str()),
                    ("filterTo", to.as_str()),
                    ("timeFrom", &not_before.to_string()),
                ])
                .fetch_rtt_json(&format!(
                    "RTT location request for {from}->{to} from {not_before}"
                ))
                .await?,
        );
        Span::current().record("service_count", response.services.len());
        self.location_cache
            .lock()
            .await
            .insert(key, Arc::clone(&response));
        Ok(response)
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
    if locations
        .iter()
        .any(|location| location.location_metadata.number_of_vehicles.is_none())
    {
        return Ok(NumberOfCarriages::Unknown);
    }

    let at_from = locations
        .first()
        .expect("from should not be cancelled")
        .location_metadata
        .number_of_vehicles
        .unwrap();

    if locations
        .iter()
        .all(|location| location.location_metadata.number_of_vehicles.unwrap() == at_from)
    {
        return Ok(NumberOfCarriages::SameThroughout(at_from));
    }

    let minimum = locations
        .iter()
        .map(|location| location.location_metadata.number_of_vehicles.unwrap())
        .min()
        .context("RTT service did not include any locations between journey stops")?;

    Ok(NumberOfCarriages::Varies { minimum, at_from })
}

fn train_service_from_rtt(
    service: &api_types::service::Service,
    from: Station,
    to: Station,
) -> Result<Option<TrainService>> {
    let Some(from_station) = service_station(service, from)? else {
        return Ok(None);
    };
    let Some(to_station) = service_station(service, to)? else {
        return Ok(None);
    };
    let destination = service
        .destination
        .first()
        .context("RTT service did not include destination")?;
    let number_of_carriages = number_of_carriages_for_journey(service, from, to)?;

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
            service_cache: Mutex::new(HashMap::new()),
            location_cache: Mutex::new(HashMap::new()),
        }
    }

    #[tracing::instrument(
        skip(self),
        fields(
            total_candidates = tracing::field::Empty,
            matched = tracing::field::Empty,
        ),
    )]
    async fn departures_between(
        &self,
        from: Station,
        to: Station,
        not_before: Timestamp,
    ) -> Result<TrainServices> {
        let access_token = self.access_token().await?;
        let response = self.location(&access_token, from, to, not_before).await?;

        let total_candidates = response.services.len();
        let span = Span::current();
        span.record("total_candidates", total_candidates);
        let mut trains = Vec::new();
        for (index, service) in response.services.iter().enumerate() {
            let unique_identity = &service.schedule_metadata.unique_identity;
            {
                let span = debug_span!(
                    "considering RTT candidate service",
                    index,
                    unique_identity,
                    mode_type = service.schedule_metadata.mode_type,
                );
                let _enter = span.enter();
                if service.schedule_metadata.mode_type == "BUS" {
                    debug!("skipping BUS service");
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

                if service.temporal_data.arrival.is_cancelled
                    || service.temporal_data.departure.is_cancelled
                {
                    debug!("skipping cancelled service");
                    continue;
                }

                let departure_data = &service.temporal_data.departure;
                let Some(departure) = departure_data
                    .realtime_forecast
                    .or(departure_data.schedule_advertised)
                else {
                    debug!("skipping service with no departure time");
                    continue;
                };
                if departure < not_before {
                    debug!(
                        %departure,
                        "skipping service departing before not_before",
                    );
                    continue;
                }
            }
            // TODO: Concurrency
            let service = self.service(&access_token, unique_identity).await?;
            if let Some(service) = train_service_from_rtt(&service.service, from, to)? {
                trains.push(service);
            }
        }

        span.record("matched", trains.len());
        Ok(TrainServices::new(trains))
    }

    #[tracing::instrument(skip(self))]
    async fn get_service(
        &self,
        service_id: &str,
        from: Station,
        to: Station,
    ) -> Result<TrainService> {
        let access_token = self.access_token().await?;
        let service = self.service(&access_token, service_id).await?;
        train_service_from_rtt(&service.service, from, to)?
            .context("Failed to get train service from RTT")
    }

    async fn purge_cache(&self) {
        self.service_cache.lock().await.clear();
        self.location_cache.lock().await.clear();
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
        number_of_vehicles: Option<u8>,
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

    fn service_with_carriages(stops: &[(&str, Option<u8>)]) -> api_types::service::Service {
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
        let service = service_with_carriages(&[
            ("AAA", Some(8)),
            ("BBB", Some(4)),
            ("CCC", Some(12)),
            ("DDD", Some(2)),
        ]);
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
        let service =
            service_with_carriages(&[("AAA", Some(8)), ("BBB", Some(4)), ("CCC", Some(8))]);
        let carriages =
            number_of_carriages_for_journey(&service, station("BBB"), station("CCC")).unwrap();
        assert_eq!(carriages, NumberOfCarriages::SameThroughout(4));
    }

    #[test]
    fn number_of_carriages_excludes_drop_at_to() {
        let service =
            service_with_carriages(&[("AAA", Some(8)), ("BBB", Some(8)), ("CCC", Some(4))]);
        let carriages =
            number_of_carriages_for_journey(&service, station("AAA"), station("CCC")).unwrap();
        assert_eq!(carriages, NumberOfCarriages::SameThroughout(8));
    }

    #[test]
    fn number_of_carriages_ignores_cancelled_intermediate_stops() {
        let service = service_with_locations(vec![
            service_location("AAA", Some(8), false),
            service_location("BBB", Some(4), true),
            service_location("CCC", Some(8), false),
            service_location("DDD", Some(8), false),
        ]);
        let carriages =
            number_of_carriages_for_journey(&service, station("AAA"), station("DDD")).unwrap();
        assert_eq!(carriages, NumberOfCarriages::SameThroughout(8));
    }

    #[test]
    fn number_of_carriages_is_unknown_when_any_stop_is_unknown() {
        let service = service_with_carriages(&[
            ("AAA", Some(8)),
            ("BBB", None),
            ("CCC", Some(8)),
            ("DDD", Some(8)),
        ]);
        let carriages =
            number_of_carriages_for_journey(&service, station("AAA"), station("DDD")).unwrap();
        assert_eq!(carriages, NumberOfCarriages::Unknown);
    }

    #[test]
    fn number_of_carriages_ignores_unknown_before_from() {
        let service = service_with_carriages(&[
            ("AAA", None),
            ("BBB", Some(8)),
            ("CCC", Some(8)),
            ("DDD", Some(8)),
        ]);
        let carriages =
            number_of_carriages_for_journey(&service, station("BBB"), station("DDD")).unwrap();
        assert_eq!(carriages, NumberOfCarriages::SameThroughout(8));
    }

    #[test]
    fn number_of_carriages_ignores_unknown_at_to() {
        let service = service_with_carriages(&[("AAA", Some(8)), ("BBB", Some(8)), ("CCC", None)]);
        let carriages =
            number_of_carriages_for_journey(&service, station("AAA"), station("CCC")).unwrap();
        assert_eq!(carriages, NumberOfCarriages::SameThroughout(8));
    }

    #[test]
    fn train_service_does_not_exist_when_from_is_cancelled() {
        let service = service_with_locations(vec![
            service_location("AAA", Some(8), true),
            service_location("BBB", Some(8), false),
        ]);
        let service = train_service_from_rtt(&service, station("AAA"), station("BBB")).unwrap();
        assert!(service.is_none());
    }

    #[test]
    fn train_service_does_not_exist_when_to_is_cancelled() {
        let service = service_with_locations(vec![
            service_location("AAA", Some(8), false),
            service_location("BBB", Some(8), true),
        ]);
        let service = train_service_from_rtt(&service, station("AAA"), station("BBB")).unwrap();
        assert!(service.is_none());
    }

    #[test]
    fn cancellation_status_must_match_between_arrival_and_departure() {
        let mut location = service_location("AAA", Some(8), false);
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
