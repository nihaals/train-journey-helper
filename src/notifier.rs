use std::sync::Arc;

use anyhow::{Result, ensure};
use jiff::Timestamp;
use tokio::sync::Mutex;

use crate::{config::Config, custom_types::TrainService, timezone::TimestampExt};

pub trait Notifier {
    fn new(config: &Config, client: reqwest::Client) -> Self;

    async fn send_notification(
        &self,
        title: &str,
        message: &str,
        tag: &str,
        group: &str,
    ) -> Result<()>;

    async fn clear_notification(&self, tag: &str) -> Result<()>;
}

const NOTIFICATION_GROUP: &str = env!("CARGO_PKG_NAME");
const NOTIFICATION_TITLE: &str = "Train journey update";

pub struct JourneyNotifier<N> {
    notifier: N,
    // TODO: Do we need Arc or Mutex?
    last_notification: Arc<Mutex<Option<String>>>,
    status_tag: String,
}

impl<N> JourneyNotifier<N> {
    pub fn new(notifier: N) -> Self {
        Self {
            notifier,
            last_notification: Arc::new(Mutex::new(None)),
            // TODO: Derive from journey config
            status_tag: "train-journey-status".to_owned(),
        }
    }
}

impl<N: Notifier> JourneyNotifier<N> {
    pub async fn send_status_report(
        &self,
        outbound_first: &[TrainService],
        outbound_second: &[TrainService],
        return_first: &[TrainService],
        return_second: &[TrainService],
    ) -> Result<()> {
        let mut message = String::from("Outbound today:\n");
        append_services(&mut message, "First leg", outbound_first)?;
        append_services(&mut message, "Second leg", outbound_second)?;
        message.push_str("\nReturn check: ");
        if return_first.is_empty() || return_second.is_empty() {
            message.push_str("no complete train route found around estimated leave time.\n");
        } else {
            ensure!(return_first.is_sorted_by_key(|service| service.from.estimated_departure));
            ensure!(return_second.is_sorted_by_key(|service| service.from.estimated_departure));
            message.push_str(&format!(
                "trains seen around {}-{} for the way home.\n",
                format_time(
                    return_first
                        .first()
                        .expect("checked non-empty")
                        .from
                        .estimated_departure
                )?,
                format_time(
                    return_second
                        .last()
                        .expect("checked non-empty")
                        .to
                        .estimated_arrival
                )?,
            ));
        }
        self.send_if_changed(&message).await
    }

    pub async fn send_outbound_update(
        &self,
        first: &[TrainService],
        second: &[TrainService],
    ) -> Result<()> {
        let mut message = String::from("Outbound options:\n");
        append_services(&mut message, "First leg", first)?;
        append_services(&mut message, "Second leg", second)?;
        self.send_if_changed(&message).await
    }

    pub async fn send_return_update(
        &self,
        first: &[TrainService],
        second: &[TrainService],
    ) -> Result<()> {
        let mut message = String::from("Return options:\n");
        append_services(&mut message, "First leg", first)?;
        append_services(&mut message, "Second leg", second)?;
        self.send_if_changed(&message).await
    }

    pub async fn send_leg_update(&self, heading: &str, services: &[TrainService]) -> Result<()> {
        let mut message = format!("{heading}:\n");
        append_services(&mut message, "Options", services)?;
        self.send_if_changed(&message).await
    }

    pub async fn send_selected_train_update(
        &self,
        heading: &str,
        service: &TrainService,
    ) -> Result<()> {
        let message = format!("{heading}: {}\n", format_service(service)?);
        self.send_if_changed(&message).await
    }

    pub async fn resend_last_notification(&self) -> Result<bool> {
        let last = self.last_notification.lock().await;
        if let Some(last) = last.as_ref() {
            self.send_notification(last).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn send_notification(&self, message: &str) -> Result<()> {
        self.notifier
            .send_notification(
                NOTIFICATION_TITLE,
                message,
                &self.status_tag,
                NOTIFICATION_GROUP,
            )
            .await
    }

    async fn send_if_changed(&self, message: &str) -> Result<()> {
        let mut last = self.last_notification.lock().await;
        if last.as_deref() == Some(message) {
            return Ok(());
        }
        self.send_notification(message).await?;
        *last = Some(message.to_owned());
        Ok(())
    }
}

fn append_services(message: &mut String, heading: &str, services: &[TrainService]) -> Result<()> {
    ensure!(services.is_sorted_by_key(|service| service.from.estimated_departure));
    message.push_str(heading);
    message.push_str(":\n");
    if services.is_empty() {
        message.push_str("- none found\n");
        return Ok(());
    }
    for service in services.iter().take(4) {
        message.push_str("- ");
        message.push_str(&format_service(service)?);
        message.push('\n');
    }
    Ok(())
}

fn format_service(service: &TrainService) -> Result<String> {
    let mut text = format!(
        "{} {}→{} arr {} {}",
        format_time(service.from.estimated_departure)?,
        service.from.station,
        service.to.station,
        format_time(service.to.estimated_arrival)?,
        service.company,
    );
    if let Some(platform) = &service.from.platform {
        text.push_str(&format!(" plat {platform}"));
    }
    // TODO: Include negatives
    if service.departure_delay_minutes() > 0 || service.arrival_delay_minutes() > 0 {
        text.push_str(&format!(
            " delay +{}/+{}m",
            service.departure_delay_minutes(),
            service.arrival_delay_minutes()
        ));
    }
    Ok(text)
}

fn format_time(time: Timestamp) -> Result<String> {
    Ok(time.to_london_zoned().strftime("%H:%M").to_string())
}
