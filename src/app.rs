use std::sync::Arc;

use anyhow::{Context, Result};
use jiff::{Timestamp, ToSpan, civil::Date};
use num_traits::ToPrimitive;
use tokio::sync::Mutex;

use crate::{
    config::Config,
    custom_types::JourneyOption,
    notifier::{JourneyNotifier, Notifier},
    provider::TrainProvider,
    station::Station,
    timezone::{DateTimeExt, TimestampExt},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JourneyState {
    Waiting,
    SkippedUntilDestination,
    OnTrainHomeToInterchange,
    OnTrainInterchangeToDestination,
    AtDestination,
    OnTrainDestinationToInterchange,
    OnTrainInterchangeToHome,
    Complete,
    SkippedDay,
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
            notifier: JourneyNotifier::new(N::new(&config, client.clone())),
            http: client,
            config,
            state: Arc::new(Mutex::new(JourneyState::Waiting)),
        }
    }

    pub fn state_handle(&self) -> Arc<Mutex<JourneyState>> {
        Arc::clone(&self.state)
    }

    /// Sends request to healthcheck URL if configured
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
        wait_until(self.monitoring_start_time()?).await;
        let options = self.outbound_options().await?;
        self.notifier.send_status_report(&options).await?;

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(120));
        loop {
            interval.tick().await;
            match *self.state.lock().await {
                JourneyState::SkippedDay | JourneyState::Complete => {
                    self.notifier.clear_notifications().await?;
                    break;
                }
                JourneyState::SkippedUntilDestination | JourneyState::AtDestination => {
                    let options = self.return_options().await?;
                    self.notifier.send_return_details(&options).await?;
                }
                JourneyState::Waiting
                | JourneyState::OnTrainHomeToInterchange
                | JourneyState::OnTrainInterchangeToDestination => {
                    let options = self.outbound_options().await?;
                    let best = options.first().context("no outbound options found")?;
                    if best.is_delayed() {
                        self.notifier
                            .send_train_delayed_before_train_1(best)
                            .await?;
                    }
                }
                JourneyState::OnTrainDestinationToInterchange
                | JourneyState::OnTrainInterchangeToHome => {
                    let options = self.return_options().await?;
                    self.notifier.send_return_details(&options).await?;
                }
            }
        }
        Ok(())
    }

    pub async fn outbound_options(&self) -> Result<Vec<JourneyOption>> {
        let start = now_fixed()
            .checked_add(i64::from(self.config.walk.home_to_station_1_minutes).minutes())?;
        self.options_via(
            self.config.stations.home,
            self.config.stations.line_one_interchange_primary,
            self.config.stations.destination_line_interchange,
            self.config.stations.destination,
            self.config.walk.station_2_to_4_minutes,
            start,
        )
        .await
    }

    pub async fn return_options(&self) -> Result<Vec<JourneyOption>> {
        let start = now_fixed();
        let mut via_3 = self
            .options_via(
                self.config.stations.destination,
                self.config.stations.destination_line_interchange,
                self.config.stations.line_one_interchange_return_preferred,
                self.config.stations.home,
                self.config.walk.station_4_to_3_minutes,
                start,
            )
            .await?;
        let mut via_2 = self
            .options_via(
                self.config.stations.destination,
                self.config.stations.destination_line_interchange,
                self.config.stations.line_one_interchange_primary,
                self.config.stations.home,
                self.config.walk.station_2_to_4_minutes,
                start,
            )
            .await?;
        via_3.append(&mut via_2);
        via_3.sort_by_key(JourneyOption::arrives_at_destination);
        via_3.truncate(4);
        Ok(via_3)
    }

    fn monitoring_start_time(&self) -> Result<Timestamp> {
        let today = Timestamp::now().to_london_zoned().date();
        let date = next_weekday(today, self.config.travel_day);
        let arrival = date
            .to_datetime(self.config.destination_arrival_time)
            .to_london_zoned()?
            .timestamp();
        let rough_journey = self.config.walk.home_to_station_1_minutes
            + self.config.walk.station_2_to_4_minutes
            + self.config.walk.station_5_to_final_destination_minutes
            + 90;
        Ok(arrival.checked_sub(
            (f64::from(rough_journey) * 1.75)
                .round()
                .to_i64()
                .context("Failed to convert rough journey duration to i64")?
                .minutes(),
        )?)
    }

    async fn options_via(
        &self,
        leg1_from: Station,
        leg1_to: Station,
        leg2_from: Station,
        leg2_to: Station,
        walk_minutes: u8,
        not_before: Timestamp,
    ) -> Result<Vec<JourneyOption>> {
        let first_legs = self
            .provider
            .departures_between(leg1_from, leg1_to, not_before)
            .await?;
        let mut options = Vec::new();
        for first in first_legs.into_iter().take(6) {
            let second_not_before = first
                .to
                .estimated_arrival
                .checked_add(i64::from(walk_minutes).minutes())?;
            let second = self
                .provider
                .departures_between(leg2_from, leg2_to, second_not_before)
                .await?
                .into_iter()
                .next()
                .with_context(|| format!("no connection found from {leg2_from} to {leg2_to}"))?;
            options.push(JourneyOption {
                outbound_first_leg: first,
                outbound_second_leg: second,
                interchange_walk_minutes: walk_minutes,
            });
        }
        options.sort_by_key(JourneyOption::arrives_at_destination);
        options.truncate(4);
        Ok(options)
    }
}

async fn wait_until(when: Timestamp) {
    let now = now_fixed();
    if when <= now {
        return;
    }

    // TODO: We can convert from Span to Duration directly but we should rewrite our waiting logic
    // first
    let seconds = now.duration_until(when).as_secs().try_into().unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(seconds)).await;
}

fn now_fixed() -> Timestamp {
    Timestamp::now()
}

fn next_weekday(mut date: Date, weekday: jiff::civil::Weekday) -> Date {
    while date.weekday() != weekday {
        date = date.tomorrow().expect("next day should be in range");
    }
    date
}
