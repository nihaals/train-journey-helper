use std::sync::Arc;

use anyhow::{Context, Result};
use jiff::{Timestamp, civil::Date};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::{
    config::Config,
    custom_types::TrainService,
    notifier::{JourneyNotifier, Notifier},
    provider::{TrainProvider, TrainServices},
    station::Station,
    timezone::{DateTimeExt, TimestampExt},
};

// TODO: Move to config?
const POLL_SECONDS: u64 = 60;
const MAX_EARLY_DESTINATION_MINUTES: i64 = 30;
/// How far before the destination arrival time we start making provider calls.
const MONITORING_LOOKAHEAD_SECONDS: i64 = 12 * 60 * 60;
/// How often the cached monitoring start is recomputed while waiting until we are close to the
/// journey.
const MONITORING_START_CACHE_TTL_SECONDS: i64 = 30 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JourneyState {
    Waiting,
    WaitingForNextJourney { resume_at: Timestamp },
    OnTrainHomeToPrimaryInterchange { selected: TrainService },
    OnTrainInterchangeToDestination { selected: TrainService },
    AtDestination,
    OnTrainDestinationToInterchange { selected: TrainService },
    OnTrainReturnPreferredInterchangeToHome { selected: TrainService },
    OnTrainPrimaryInterchangeToHome { selected: TrainService },
    // TODO: Replace with `WaitingForNextJourney`?
    Complete,
}

#[derive(Debug, Clone, Copy)]
pub enum JourneyLeg {
    HomeToPrimaryInterchange,
    InterchangeToDestination,
    DestinationToInterchange,
    ReturnPreferredInterchangeToHome,
    PrimaryInterchangeToHome,
}

#[derive(Debug, Clone, Serialize)]
pub struct StationConfigResponse {
    pub home: Station,
    pub line_one_interchange_primary: Station,
    pub line_one_interchange_return_preferred: Station,
    pub destination_line_interchange: Station,
    pub destination: Station,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrainResponse {
    pub service_id: String,
    pub from: Station,
    pub to: Station,
    pub company: String,
    pub route_destination: String,
    #[serde(with = "jiff::fmt::serde::timestamp::second::required")]
    pub scheduled_departure: Timestamp,
    #[serde(with = "jiff::fmt::serde::timestamp::second::required")]
    pub estimated_departure: Timestamp,
    #[serde(with = "jiff::fmt::serde::timestamp::second::required")]
    pub scheduled_arrival: Timestamp,
    #[serde(with = "jiff::fmt::serde::timestamp::second::required")]
    pub estimated_arrival: Timestamp,
    pub from_platform: Option<String>,
}

#[derive(Clone, Copy)]
struct CachedMonitoringStart {
    /// The time to start monitoring.
    start: Timestamp,
    /// The target destination arrival time for which this cached value was computed.
    target_arrival: Timestamp,
    /// When this cached value was computed.
    computed_at: Timestamp,
}

pub struct App<P, N> {
    pub config: Config,
    http: reqwest::Client,
    provider: P,
    notifier: JourneyNotifier<N>,
    state: Arc<Mutex<JourneyState>>,
    monitoring_start_cache: Mutex<Option<CachedMonitoringStart>>,
}

impl<P: TrainProvider, N: Notifier> App<P, N> {
    pub fn new(config: Config, client: reqwest::Client) -> Self {
        Self {
            provider: P::new(&config, client.clone()),
            notifier: JourneyNotifier::new(N::new(&config, client.clone()), &config),
            http: client,
            config,
            state: Arc::new(Mutex::new(JourneyState::Waiting)),
            monitoring_start_cache: Mutex::new(None),
        }
    }

    pub async fn set_state(&self, state: JourneyState) {
        *self.state.lock().await = state;
    }

    pub async fn set_on_train(&self, leg: JourneyLeg, service_id: &str) -> Result<()> {
        let (from, to) = self.leg_stations(leg);
        let selected = self.provider.get_service(service_id, from, to).await?;
        self.set_state(match leg {
            JourneyLeg::HomeToPrimaryInterchange => {
                JourneyState::OnTrainHomeToPrimaryInterchange { selected }
            }
            JourneyLeg::InterchangeToDestination => {
                JourneyState::OnTrainInterchangeToDestination { selected }
            }
            JourneyLeg::DestinationToInterchange => {
                JourneyState::OnTrainDestinationToInterchange { selected }
            }
            JourneyLeg::ReturnPreferredInterchangeToHome => {
                JourneyState::OnTrainReturnPreferredInterchangeToHome { selected }
            }
            JourneyLeg::PrimaryInterchangeToHome => {
                JourneyState::OnTrainPrimaryInterchangeToHome { selected }
            }
        })
        .await;
        Ok(())
    }

    pub fn config_response(&self) -> StationConfigResponse {
        StationConfigResponse {
            home: self.config.stations.home,
            line_one_interchange_primary: self.config.stations.line_one_interchange_primary,
            line_one_interchange_return_preferred: self
                .config
                .stations
                .line_one_interchange_return_preferred,
            destination_line_interchange: self.config.stations.destination_line_interchange,
            destination: self.config.stations.destination,
        }
    }

    pub async fn trains_for_leg(
        &self,
        leg: JourneyLeg,
        not_before: Timestamp,
    ) -> Result<TrainServices> {
        let (from, to) = self.leg_stations(leg);
        Ok(TrainServices::new(
            self.provider
                .departures_between(from, to, not_before)
                .await?,
        ))
    }

    pub async fn send_last_notification(&self) -> Result<bool> {
        self.notifier.resend_last_notification().await
    }

    /// Sends request to healthcheck URL if configured.
    pub async fn send_healthcheck(&self) -> Result<()> {
        if let Some(url) = &self.config.healthcheck_url {
            self.http
                .get(url)
                .send()
                .await
                .context("Failed to send healthcheck")?;
        }
        Ok(())
    }

    pub async fn run_scheduler(self: Arc<Self>) -> Result<()> {
        // TODO: Add span?
        let mut initial_report_sent = false;
        loop {
            self.provider.purge_cache().await;

            let now = Timestamp::now();
            let mut state = self.state.lock().await.clone();
            match state {
                JourneyState::Complete => {
                    state = JourneyState::WaitingForNextJourney {
                        resume_at: self.next_journey_start(now)?,
                    };
                    self.set_state(state.clone()).await;
                }
                JourneyState::WaitingForNextJourney { resume_at } if now >= resume_at => {
                    state = JourneyState::Waiting;
                    self.set_state(state.clone()).await;
                    initial_report_sent = false;
                }
                _ => {}
            }

            if self.should_poll(now, &state).await? {
                if !initial_report_sent {
                    self.send_initial_report(now).await?;
                    initial_report_sent = true;
                } else {
                    self.notify_for_state(&state, now).await?;
                }
            }

            self.send_healthcheck().await?;
            tokio::time::sleep(std::time::Duration::from_secs(POLL_SECONDS)).await;
        }
    }

    async fn should_poll(&self, now: Timestamp, state: &JourneyState) -> Result<bool> {
        Ok(match state {
            JourneyState::Waiting => {
                if !self.within_monitoring_lookahead(now)? {
                    false
                } else {
                    now >= self.monitoring_start_time(now).await?
                }
            }
            // Checking if we should roll over is handled elsewhere
            JourneyState::WaitingForNextJourney { .. } => false,
            _ => true,
        })
    }

    /// Checks if we are within a generous window of the target arrival time without making
    /// any provider calls.
    fn within_monitoring_lookahead(&self, now: Timestamp) -> Result<bool> {
        let target = self.destination_arrival_timestamp(now)?;
        let until_arrival = now.duration_until(target);
        Ok(until_arrival.as_secs() <= MONITORING_LOOKAHEAD_SECONDS)
    }

    async fn monitoring_start_time(&self, now: Timestamp) -> Result<Timestamp> {
        let target = self.destination_arrival_timestamp(now)?;
        {
            let cache = self.monitoring_start_cache.lock().await;
            if let Some(cached) = cache.as_ref()
                && cached.target_arrival == target
            {
                let age = cached.computed_at.duration_until(now);
                if age.as_secs() < MONITORING_START_CACHE_TTL_SECONDS {
                    return Ok(cached.start);
                }
            }
        }
        let start = self.compute_monitoring_start_time(now, target).await?;
        *self.monitoring_start_cache.lock().await = Some(CachedMonitoringStart {
            target_arrival: target,
            start,
            computed_at: now,
        });
        Ok(start)
    }

    async fn compute_monitoring_start_time(
        &self,
        now: Timestamp,
        target: Timestamp,
    ) -> Result<Timestamp> {
        let latest_destination_train =
            target.checked_sub(self.config.walk.station_5_to_final_destination)?;
        let second = self
            .provider
            .departures_between(
                self.config.stations.destination_line_interchange,
                self.config.stations.destination,
                // TODO: Document assumptions on `now`, maybe use something better if needed?
                now,
            )
            .await?
            .into_iter()
            .rev()
            .find(|train| train.to.estimated_arrival <= latest_destination_train)
            // TODO: What if we have to restart during a journey? Applies less to this one
            .context("No destination-leg train can reach the destination in time")?;
        let latest_first_arrival = second
            .from
            .estimated_departure
            .checked_sub(self.config.walk.station_2_to_4)?;
        let first = self
            .provider
            .departures_between(
                self.config.stations.home,
                self.config.stations.line_one_interchange_primary,
                now,
            )
            .await?
            .into_iter()
            .rev()
            .find(|train| train.to.estimated_arrival <= latest_first_arrival)
            .context("No first-leg train can make the destination train")?;
        // TODO: We're just returning the latest we can leave home, not when to start monitoring
        first
            .from
            .estimated_departure
            .checked_sub(self.config.walk.home_to_station_1)
            .context("Failed to calculate monitoring start")
    }

    async fn send_initial_report(&self, now: Timestamp) -> Result<()> {
        let outbound_first = self
            .trains_for_leg(
                JourneyLeg::HomeToPrimaryInterchange,
                now.checked_add(self.config.walk.home_to_station_1)?,
            )
            .await?;
        let outbound_second = self.best_second_leg_options(&outbound_first, now).await?;
        let return_start = self.return_start_time(now)?;
        let return_first = self
            .trains_for_leg(JourneyLeg::DestinationToInterchange, return_start)
            .await?;
        let return_second = self.best_return_second_leg_options(&return_first).await?;
        self.notifier
            .send_status_report(
                &outbound_first,
                &outbound_second,
                &return_first,
                &return_second,
            )
            .await
    }

    async fn notify_for_state(&self, state: &JourneyState, now: Timestamp) -> Result<()> {
        match state {
            JourneyState::Waiting => {
                // TODO: We should leave the status report up instead of replacing it immediately on
                // the next poll as we don't include return status here
                let first = self
                    .trains_for_leg(
                        JourneyLeg::HomeToPrimaryInterchange,
                        now.checked_add(self.config.walk.home_to_station_1)?,
                    )
                    .await?;
                let second = self.best_second_leg_options(&first, now).await?;
                self.notifier.send_outbound_update(&first, &second).await
            }
            JourneyState::OnTrainHomeToPrimaryInterchange { selected } => {
                let start = selected
                    .to
                    .estimated_arrival
                    .checked_add(self.config.walk.station_2_to_4)?;
                let second = self
                    .trains_for_leg(JourneyLeg::InterchangeToDestination, start)
                    .await?;
                self.notifier
                    .send_leg_update("Next outbound leg", &second)
                    .await
            }
            JourneyState::OnTrainInterchangeToDestination { selected } => {
                self.notifier
                    .send_selected_train_update("On final outbound leg", selected)
                    .await
            }
            JourneyState::AtDestination => {
                let first = self
                    .trains_for_leg(JourneyLeg::DestinationToInterchange, now)
                    .await?;
                let second = self.best_return_second_leg_options(&first).await?;
                self.notifier.send_return_update(&first, &second).await
            }
            JourneyState::OnTrainDestinationToInterchange { selected } => {
                let start_3 = selected
                    .to
                    .estimated_arrival
                    .checked_add(self.config.walk.station_4_to_3)?;
                let start_2 = selected
                    .to
                    .estimated_arrival
                    .checked_add(self.config.walk.station_2_to_4)?;
                let mut trains = self
                    .trains_for_leg(JourneyLeg::ReturnPreferredInterchangeToHome, start_3)
                    .await?;
                trains.extend(
                    self.trains_for_leg(JourneyLeg::PrimaryInterchangeToHome, start_2)
                        .await?,
                );
                self.notifier
                    .send_leg_update("Next return leg", &trains)
                    .await
            }
            JourneyState::OnTrainReturnPreferredInterchangeToHome { selected }
            | JourneyState::OnTrainPrimaryInterchangeToHome { selected } => {
                self.notifier
                    .send_selected_train_update("On final return leg", selected)
                    .await
            }
            JourneyState::WaitingForNextJourney { .. } | JourneyState::Complete => Ok(()),
        }
    }

    async fn best_second_leg_options(
        &self,
        first_legs: &TrainServices,
        now: Timestamp,
    ) -> Result<TrainServices> {
        let mut second = TrainServices::new_empty();
        let target = self.destination_arrival_timestamp(now)?;
        for first in first_legs.first_n_by_arrival(3) {
            let start = first
                .to
                .estimated_arrival
                .checked_add(self.config.walk.station_2_to_4)?;
            for train in self
                .trains_for_leg(JourneyLeg::InterchangeToDestination, start)
                .await?
                .into_iter_by_arrival()
            {
                let after_walk = train
                    .to
                    .estimated_arrival
                    .checked_add(self.config.walk.station_5_to_final_destination)?;
                let too_early =
                    after_walk.duration_until(target).as_mins() > MAX_EARLY_DESTINATION_MINUTES;
                if after_walk <= target && !too_early {
                    second.push(train);
                }
            }
        }
        second.dedup_by_service_id(&self.config.stations);
        Ok(second)
    }

    async fn best_return_second_leg_options(
        &self,
        first_legs: &TrainServices,
    ) -> Result<TrainServices> {
        let mut second = TrainServices::new_empty();
        for first in first_legs.first_n_by_arrival(3) {
            let start_3 = first
                .to
                .estimated_arrival
                .checked_add(self.config.walk.station_4_to_3)?;
            let start_2 = first
                .to
                .estimated_arrival
                .checked_add(self.config.walk.station_2_to_4)?;
            second.extend(
                self.trains_for_leg(JourneyLeg::ReturnPreferredInterchangeToHome, start_3)
                    .await?,
            );
            second.extend(
                self.trains_for_leg(JourneyLeg::PrimaryInterchangeToHome, start_2)
                    .await?,
            );
        }
        second.dedup_by_service_id(&self.config.stations);
        Ok(second)
    }

    fn destination_arrival_timestamp(&self, now: Timestamp) -> Result<Timestamp> {
        let date = next_weekday(now.to_london_zoned().date(), self.config.travel_day);
        Ok(date
            .to_datetime(self.config.destination_arrival_time)
            .to_london_zoned()?
            .timestamp())
    }

    fn return_start_time(&self, now: Timestamp) -> Result<Timestamp> {
        let destination_arrival = self.destination_arrival_timestamp(now)?;
        Ok(destination_arrival
            .checked_add(self.config.destination_stay_estimate)?
            .max(now))
    }

    fn next_journey_start(&self, now: Timestamp) -> Result<Timestamp> {
        let date = next_weekday(
            now.to_london_zoned().date().tomorrow()?,
            self.config.travel_day,
        );
        Ok(date.at(0, 0, 0, 0).to_london_zoned()?.timestamp())
    }

    fn leg_stations(&self, leg: JourneyLeg) -> (Station, Station) {
        match leg {
            JourneyLeg::HomeToPrimaryInterchange => (
                self.config.stations.home,
                self.config.stations.line_one_interchange_primary,
            ),
            JourneyLeg::InterchangeToDestination => (
                self.config.stations.destination_line_interchange,
                self.config.stations.destination,
            ),
            JourneyLeg::DestinationToInterchange => (
                self.config.stations.destination,
                self.config.stations.destination_line_interchange,
            ),
            JourneyLeg::ReturnPreferredInterchangeToHome => (
                self.config.stations.line_one_interchange_return_preferred,
                self.config.stations.home,
            ),
            JourneyLeg::PrimaryInterchangeToHome => (
                self.config.stations.line_one_interchange_primary,
                self.config.stations.home,
            ),
        }
    }
}

impl From<TrainService> for TrainResponse {
    fn from(value: TrainService) -> Self {
        Self {
            service_id: value.service_id,
            from: value.from.station,
            to: value.to.station,
            company: value.company,
            route_destination: value.route_destination,
            scheduled_departure: value.from.scheduled_departure,
            estimated_departure: value.from.estimated_departure,
            scheduled_arrival: value.to.scheduled_arrival,
            estimated_arrival: value.to.estimated_arrival,
            from_platform: value.from.platform,
        }
    }
}

fn next_weekday(mut date: Date, weekday: jiff::civil::Weekday) -> Date {
    while date.weekday() != weekday {
        date = date.tomorrow().expect("next day should be in range");
    }
    date
}

#[cfg(test)]
mod tests {
    use std::{
        str::FromStr,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use jiff::{Span, Timestamp};

    use super::*;
    use crate::{
        config::Config,
        custom_types::{NumberOfCarriages, TrainServiceStation},
    };

    struct MockProvider {
        calls: Arc<AtomicUsize>,
    }

    struct MockNotifier;

    fn mock_train(
        service_id: String,
        from: Station,
        to: Station,
        departure: Timestamp,
        arrival: Timestamp,
    ) -> TrainService {
        TrainService {
            service_id,
            from: TrainServiceStation {
                station: from,
                scheduled_arrival: departure,
                estimated_arrival: departure,
                scheduled_departure: departure,
                estimated_departure: departure,
                platform: None,
            },
            to: TrainServiceStation {
                station: to,
                scheduled_arrival: arrival,
                estimated_arrival: arrival,
                scheduled_departure: arrival,
                estimated_departure: arrival,
                platform: None,
            },
            company: "Test".to_owned(),
            route_destination: "Test".to_owned(),
            number_of_carriages: NumberOfCarriages::SameThroughout(2),
        }
    }

    impl TrainProvider for MockProvider {
        fn new(_config: &Config, _client: reqwest::Client) -> Self {
            Self {
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        async fn departures_between(
            &self,
            from: Station,
            to: Station,
            not_before: Timestamp,
        ) -> Result<Vec<TrainService>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let date = not_before.to_london_zoned().date();
            let mut trains = Vec::new();
            for hour in [5, 6, 7, 8, 9, 10] {
                let departure = date
                    .to_datetime(jiff::civil::Time::new(hour, 0, 0, 0).unwrap())
                    .to_london_zoned()
                    .unwrap()
                    .timestamp();
                if departure < not_before {
                    continue;
                }
                let arrival = departure.checked_add(Span::new().minutes(20)).unwrap();
                trains.push(mock_train(
                    format!("svc-{hour}"),
                    from,
                    to,
                    departure,
                    arrival,
                ));
            }
            Ok(trains)
        }

        async fn get_service(
            &self,
            _service_id: &str,
            _from: Station,
            _to: Station,
        ) -> Result<TrainService> {
            unimplemented!()
        }

        async fn purge_cache(&self) {}
    }

    impl Notifier for MockNotifier {
        fn new(_config: &Config, _client: reqwest::Client) -> Self {
            Self
        }

        async fn send_notification(
            &self,
            _title: &str,
            _message: &str,
            _tag: &str,
            _group: &str,
        ) -> Result<()> {
            Ok(())
        }

        async fn clear_notification(&self, _tag: &str) -> Result<()> {
            Ok(())
        }
    }

    fn test_config() -> Config {
        Config {
            stations: crate::config::Stations {
                home: Station::from_str("AAA").unwrap(),
                line_one_interchange_primary: Station::from_str("BBB").unwrap(),
                line_one_interchange_return_preferred: Station::from_str("CCC").unwrap(),
                destination_line_interchange: Station::from_str("DDD").unwrap(),
                destination: Station::from_str("EEE").unwrap(),
            },
            walk: crate::config::WalkTimes {
                home_to_station_1: Span::new().minutes(10),
                station_2_to_4: Span::new().minutes(12),
                station_4_to_3: Span::new().minutes(8),
                station_5_to_final_destination: Span::new().minutes(15),
            },
            destination_arrival_time: jiff::civil::Time::new(9, 30, 0, 0).unwrap(),
            destination_stay_estimate: Span::new().minutes(120),
            travel_day: jiff::civil::Weekday::Monday,
            listen_addr: "127.0.0.1:3000".parse().unwrap(),
            healthcheck_url: None,
            home_assistant: crate::config::HomeAssistantConfig {
                base_url: "https://example.com".to_owned(),
                token: "token".to_owned(),
                notify_service: "notify.mobile_app".to_owned(),
            },
            rtt: crate::config::RttConfig {
                token: "token".to_owned(),
            },
        }
    }

    fn test_app() -> (App<MockProvider, MockNotifier>, Arc<AtomicUsize>) {
        let config = test_config();
        let client = reqwest::Client::new();
        let app = App::<MockProvider, MockNotifier>::new(config, client);
        let calls = Arc::clone(&app.provider.calls);
        (app, calls)
    }

    fn call_count(calls: &Arc<AtomicUsize>) -> usize {
        calls.load(Ordering::SeqCst)
    }

    fn london_timestamp(year: i16, month: i8, day: i8, hour: i8, minute: i8) -> Timestamp {
        crate::timezone::init().unwrap();
        let date = jiff::civil::Date::new(year, month, day).unwrap();
        let time = jiff::civil::Time::new(hour, minute, 0, 0).unwrap();
        date.to_datetime(time)
            .to_london_zoned()
            .unwrap()
            .timestamp()
    }

    const SHOULD_POLL_PROVIDER_CALL_COUNT: usize = 2;

    #[tokio::test]
    async fn far_from_journey_skips_provider_calls() {
        // Friday 09:30, three days before Monday 09:30 arrival
        let now = london_timestamp(2000, 1, 7, 9, 30);
        let (app, calls) = test_app();
        let should_poll = app.should_poll(now, &JourneyState::Waiting).await.unwrap();
        assert!(!should_poll);
        assert_eq!(call_count(&calls), 0);
    }

    #[tokio::test]
    async fn monitoring_start_is_cached() {
        // Monday 06:00, within the lookahead of the 09:30 arrival
        let now = london_timestamp(2000, 1, 3, 6, 0);
        let (app, calls) = test_app();

        let first = app.monitoring_start_time(now).await.unwrap();
        assert_eq!(call_count(&calls), SHOULD_POLL_PROVIDER_CALL_COUNT);
        let second = app.monitoring_start_time(now).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(call_count(&calls), SHOULD_POLL_PROVIDER_CALL_COUNT);

        {
            let soon = now.checked_add(Span::new().minutes(10)).unwrap();
            let cached = app.monitoring_start_time(soon).await.unwrap();
            assert_eq!(cached, first);
            assert_eq!(call_count(&calls), SHOULD_POLL_PROVIDER_CALL_COUNT);
        }

        {
            let soon = now.checked_add(Span::new().hours(1)).unwrap();
            let not_cached = app.monitoring_start_time(soon).await.unwrap();
            assert_eq!(not_cached, first);
            assert_eq!(call_count(&calls), SHOULD_POLL_PROVIDER_CALL_COUNT * 2);
        }
    }

    #[tokio::test]
    async fn should_poll_uses_cache_before_start() {
        let now = london_timestamp(2000, 1, 3, 6, 0);
        let (app, calls) = test_app();

        let start = app.monitoring_start_time(now).await.unwrap();
        assert!(start > now);
        assert_eq!(call_count(&calls), SHOULD_POLL_PROVIDER_CALL_COUNT);

        let should_poll = app.should_poll(now, &JourneyState::Waiting).await.unwrap();
        assert!(!should_poll);
        assert_eq!(call_count(&calls), SHOULD_POLL_PROVIDER_CALL_COUNT);
    }

    #[tokio::test]
    async fn should_poll_starts_after_cached_start() {
        let now = london_timestamp(2000, 1, 3, 6, 0);
        let (app, calls) = test_app();

        let start = app.monitoring_start_time(now).await.unwrap();
        let after_start = start.checked_add(Span::new().minutes(5)).unwrap();
        let should_poll = app
            .should_poll(after_start, &JourneyState::Waiting)
            .await
            .unwrap();
        assert!(should_poll);
        assert_eq!(call_count(&calls), SHOULD_POLL_PROVIDER_CALL_COUNT * 2);
    }
}
