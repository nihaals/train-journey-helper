use crate::{
    config::Config, custom_types::JourneyOption, home_assistant::HomeAssistantNotifier,
    provider::TrainProvider,
};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Datelike, Duration, FixedOffset, Local, TimeZone, Weekday};
use std::sync::Arc;
use tokio::sync::Mutex;

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

pub struct App<P> {
    pub config: Config,
    provider: P,
    notifier: HomeAssistantNotifier,
    state: Arc<Mutex<JourneyState>>,
}

impl<P: TrainProvider> App<P> {
    pub fn new(config: Config, provider: P, notifier: HomeAssistantNotifier) -> Self {
        Self {
            config,
            provider,
            notifier,
            state: Arc::new(Mutex::new(JourneyState::Waiting)),
        }
    }

    pub fn state_handle(&self) -> Arc<Mutex<JourneyState>> {
        Arc::clone(&self.state)
    }

    pub async fn send_healthcheck(&self) -> Result<()> {
        self.notifier.send_healthcheck().await
    }

    pub async fn run_scheduler(self: Arc<Self>) -> Result<()> {
        wait_until(self.monitoring_start_time()?).await;
        let options = self.outbound_options().await?;
        self.notifier.send_status_report(&options).await?;

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(120));
        loop {
            interval.tick().await;
            match *self.state.lock().await {
                JourneyState::SkippedDay | JourneyState::Complete => break,
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
        let start =
            now_fixed()? + Duration::minutes(self.config.walk.home_to_station_1_minutes.max(0));
        self.options_via(
            &self.config.stations.home,
            &self.config.stations.line_one_interchange_primary,
            &self.config.stations.destination_line_interchange,
            &self.config.stations.destination,
            self.config.walk.station_2_to_4_minutes,
            start,
        )
        .await
    }

    pub async fn return_options(&self) -> Result<Vec<JourneyOption>> {
        let start = now_fixed()?;
        let mut via_3 = self
            .options_via(
                &self.config.stations.destination,
                &self.config.stations.destination_line_interchange,
                &self.config.stations.line_one_interchange_return_preferred,
                &self.config.stations.home,
                self.config.walk.station_4_to_3_minutes,
                start,
            )
            .await?;
        let mut via_2 = self
            .options_via(
                &self.config.stations.destination,
                &self.config.stations.destination_line_interchange,
                &self.config.stations.line_one_interchange_primary,
                &self.config.stations.home,
                self.config.walk.station_2_to_4_minutes,
                start,
            )
            .await?;
        via_3.append(&mut via_2);
        via_3.sort_by_key(JourneyOption::arrives_at_destination);
        via_3.truncate(4);
        Ok(via_3)
    }

    fn monitoring_start_time(&self) -> Result<DateTime<FixedOffset>> {
        let today = Local::now().date_naive();
        let date = next_weekday(today, self.config.travel_day);
        let offset = FixedOffset::east_opt(Local::now().offset().local_minus_utc())
            .context("local offset is invalid")?;
        let arrival = offset
            .from_local_datetime(&date.and_time(self.config.destination_arrival_time))
            .single()
            .context("could not construct destination arrival datetime")?;
        let rough_journey = self.config.walk.home_to_station_1_minutes
            + self.config.walk.station_2_to_4_minutes
            + self.config.walk.station_5_to_final_destination_minutes
            + 90;
        Ok(arrival - Duration::minutes((rough_journey as f64 * 1.75).round() as i64))
    }

    async fn options_via(
        &self,
        leg1_from: &str,
        leg1_to: &str,
        leg2_from: &str,
        leg2_to: &str,
        walk_minutes: i64,
        not_before: DateTime<FixedOffset>,
    ) -> Result<Vec<JourneyOption>> {
        ensure!(walk_minutes >= 0, "walk time must not be negative");
        let first_legs = self
            .provider
            .departures_between(leg1_from, leg1_to, not_before)
            .await?;
        let mut options = Vec::new();
        for first in first_legs.into_iter().take(6) {
            let second_not_before = first.estimated_arrival + Duration::minutes(walk_minutes);
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

async fn wait_until(when: DateTime<FixedOffset>) {
    if let Ok(now) = now_fixed() {
        if when > now {
            let wait = (when - now).to_std().unwrap_or_default();
            tokio::time::sleep(wait).await;
        }
    }
}

fn now_fixed() -> Result<DateTime<FixedOffset>> {
    let now = Local::now();
    let offset = FixedOffset::east_opt(now.offset().local_minus_utc()).context("invalid offset")?;
    Ok(now.with_timezone(&offset))
}

fn next_weekday(mut date: chrono::NaiveDate, weekday: Weekday) -> chrono::NaiveDate {
    while date.weekday() != weekday {
        date += Duration::days(1);
    }
    date
}
