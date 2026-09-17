use std::sync::Arc;

use anyhow::Result;
use jiff::Timestamp;
use tokio::sync::Mutex;

use crate::{
    config::Config, custom_types::TrainService, provider::TrainServices, timezone::TimestampExt,
};

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
    pub fn new(notifier: N, config: &Config) -> Self {
        Self {
            notifier,
            last_notification: Arc::new(Mutex::new(None)),
            status_tag: journey_tag(config),
        }
    }
}

fn journey_tag(config: &Config) -> String {
    let mut hash = StableHash::new();
    hash.update_str(config.stations.home.as_str());
    hash.update_str(config.stations.line_one_interchange_primary.as_str());
    hash.update_str(
        config
            .stations
            .line_one_interchange_return_preferred
            .as_str(),
    );
    hash.update_str(config.stations.destination_line_interchange.as_str());
    hash.update_str(config.stations.destination.as_str());
    hash.update_str(&config.destination_arrival_time.to_string());
    hash.update_str(&config.travel_day.to_monday_zero_offset().to_string());
    let hash = hash.finish();
    base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        hash.to_le_bytes(),
    )
}

/// FNV-1a 32-bit implementation.
struct StableHash(u32);

impl StableHash {
    const OFFSET_BASIS: u32 = 0x811c9dc5;
    const PRIME: u32 = 0x01000193;

    fn new() -> Self {
        Self(Self::OFFSET_BASIS)
    }

    fn update_str(&mut self, value: &str) {
        self.update(&value.len().to_le_bytes());
        self.update(value.as_bytes());
    }

    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u32::from(*byte);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    fn finish(self) -> u32 {
        self.0
    }
}

impl<N: Notifier> JourneyNotifier<N> {
    pub async fn send_status_report(
        &self,
        outbound_first: &TrainServices,
        outbound_second: &TrainServices,
        return_first: &TrainServices,
        return_second: &TrainServices,
    ) -> Result<()> {
        let message =
            format_status_report(outbound_first, outbound_second, return_first, return_second)?;
        self.send_if_changed(&message).await
    }

    pub async fn send_outbound_update(
        &self,
        first: &TrainServices,
        second: &TrainServices,
    ) -> Result<()> {
        let mut message = String::from("Outbound options:\n");
        append_services(&mut message, "First leg", first)?;
        append_services(&mut message, "Second leg", second)?;
        self.send_if_changed(&message).await
    }

    pub async fn send_return_update(
        &self,
        first: &TrainServices,
        second: &TrainServices,
    ) -> Result<()> {
        let mut message = String::from("Return options:\n");
        append_services(&mut message, "First leg", first)?;
        append_services(&mut message, "Second leg", second)?;
        self.send_if_changed(&message).await
    }

    pub async fn send_leg_update(&self, heading: &str, services: &TrainServices) -> Result<()> {
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

pub fn format_status_report(
    outbound_first: &TrainServices,
    outbound_second: &TrainServices,
    return_first: &TrainServices,
    return_second: &TrainServices,
) -> Result<String> {
    let mut message = String::from("Outbound today:\n");
    append_services(&mut message, "First leg", outbound_first)?;
    append_services(&mut message, "Second leg", outbound_second)?;
    message.push_str("\nReturn check: ");
    if return_first.is_empty() || return_second.is_empty() {
        message.push_str("no complete train route found around estimated leave time.\n");
    } else {
        message.push_str(&format!(
            "trains seen around {}-{} for the way home.\n",
            format_time(
                return_first
                    .first_by_departure()
                    .expect("checked non-empty")
                    .from
                    .estimated_departure
            )?,
            format_time(
                return_second
                    .last_by_departure()
                    .expect("checked non-empty")
                    .to
                    .estimated_arrival
            )?,
        ));
    }
    Ok(message)
}

fn append_services(message: &mut String, heading: &str, services: &TrainServices) -> Result<()> {
    message.push_str(heading);
    message.push_str(":\n");
    if services.is_empty() {
        message.push_str("- none found\n");
        return Ok(());
    }
    for service in services.first_n_by_departure(4) {
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
    if service.departure_delay_minutes() != 0 || service.arrival_delay_minutes() != 0 {
        text.push_str(&format!(
            " delay {:+}/{:+}m",
            service.departure_delay_minutes(),
            service.arrival_delay_minutes()
        ));
    }
    Ok(text)
}

fn format_time(time: Timestamp) -> Result<String> {
    Ok(time.to_london_zoned().strftime("%H:%M").to_string())
}
