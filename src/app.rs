use std::sync::Arc;

use anyhow::{Context, Result};
use jiff::{Timestamp, civil::Date};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::{
    config::Config,
    custom_types::TrainService,
    notifier::{JourneyNotifier, Notifier},
    provider::TrainProvider,
    station::Station,
    timezone::{DateTimeExt, TimestampExt},
};

// TODO: Move to config?
const POLL_SECONDS: u64 = 60;
const LEG_OPTIONS: usize = 6;
const MAX_EARLY_DESTINATION_MINUTES: i64 = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JourneyState {
    Waiting,
    OnTrainHomeToPrimaryInterchange { selected: TrainService },
    OnTrainInterchangeToDestination { selected: TrainService },
    AtDestination,
    OnTrainDestinationToInterchange { selected: TrainService },
    OnTrainReturnPreferredInterchangeToHome { selected: TrainService },
    OnTrainPrimaryInterchangeToHome { selected: TrainService },
    Complete,
    // TODO: Delete?
    SkippedDay,
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

pub struct App<P, N> {
    pub config: Config,
    http: reqwest::Client,
    provider: P,
    notifier: JourneyNotifier<N>,
    state: Arc<Mutex<JourneyState>>,
}

impl<P: TrainProvider, N: Notifier> App<P, N> {
    pub fn new(config: Config, client: reqwest::Client) -> Self {
        Self {
            provider: P::new(&config, client.clone()),
            notifier: JourneyNotifier::new(N::new(&config, client.clone()), &config),
            http: client,
            config,
            state: Arc::new(Mutex::new(JourneyState::Waiting)),
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
    ) -> Result<Vec<TrainService>> {
        let (from, to) = self.leg_stations(leg);
        Ok(self
            .provider
            .departures_between(from, to, not_before)
            .await?
            .into_iter()
            .take(LEG_OPTIONS)
            .collect())
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
            let state = self.state.lock().await.clone();
            if matches!(state, JourneyState::SkippedDay | JourneyState::Complete) {
                self.send_healthcheck().await?;
                // TODO: We need to send a healthcheck forever
                // TODO: We need to handle going from complete to waiting for next week
                break;
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
        Ok(())
    }

    async fn should_poll(&self, now: Timestamp, state: &JourneyState) -> Result<bool> {
        Ok(match state {
            JourneyState::Waiting => now >= self.monitoring_start_time().await?,
            // TODO: Still check for complete
            _ => true,
        })
    }

    async fn monitoring_start_time(&self) -> Result<Timestamp> {
        // TODO: We shouldn't be doing RTT API calls every minute while in `JourneyState::Waiting`,
        // cache or something
        let now = Timestamp::now();
        let target = self.destination_arrival_timestamp(now)?;
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
        let outbound_second = self.best_second_leg_options(&outbound_first).await?;
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
                let second = self.best_second_leg_options(&first).await?;
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
                trains.sort_by_key(|train| train.to.estimated_arrival);
                trains.truncate(LEG_OPTIONS);
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
            JourneyState::SkippedDay | JourneyState::Complete => Ok(()),
        }
    }

    async fn best_second_leg_options(
        &self,
        first_legs: &[TrainService],
    ) -> Result<Vec<TrainService>> {
        let mut second = Vec::new();
        // TODO: Not `now` as an argument?
        let target = self.destination_arrival_timestamp(Timestamp::now())?;
        // TODO: Make `Vec` wrapper which handles sorting
        for first in first_legs.iter().take(3) {
            let start = first
                .to
                .estimated_arrival
                .checked_add(self.config.walk.station_2_to_4)?;
            for train in self
                .trains_for_leg(JourneyLeg::InterchangeToDestination, start)
                .await?
            {
                let after_walk = train
                    .to
                    .estimated_arrival
                    .checked_add(self.config.walk.station_5_to_final_destination)?;
                let too_early =
                    after_walk.duration_until(target).as_mins() > MAX_EARLY_DESTINATION_MINUTES;
                if after_walk <= target
                    && !too_early
                    && !second
                        .iter()
                        .any(|seen: &TrainService| seen.service_id == train.service_id)
                {
                    second.push(train);
                }
            }
        }
        second.sort_by_key(|train| train.to.estimated_arrival);
        second.truncate(LEG_OPTIONS);
        Ok(second)
    }

    async fn best_return_second_leg_options(
        &self,
        first_legs: &[TrainService],
    ) -> Result<Vec<TrainService>> {
        let mut second = Vec::new();
        for first in first_legs.iter().take(3) {
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
        second.sort_by_key(|train| train.to.estimated_arrival);
        // TODO: Prefer return preferred which have the same service ID, could sort by `from`
        second.dedup_by(|a, b| a.service_id == b.service_id);
        second.truncate(LEG_OPTIONS);
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
    // TODO: Audit usages for handling being in `Waiting` after journey
    while date.weekday() != weekday {
        date = date.tomorrow().expect("next day should be in range");
    }
    date
}
