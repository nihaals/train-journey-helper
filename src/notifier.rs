use anyhow::Result;
use jiff::Timestamp;

use crate::{config::Config, custom_types::JourneyOption};

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

pub struct JourneyNotifier<N> {
    notifier: N,
    group: &'static str,
    status_tag: &'static str,
    return_tag: &'static str,
    status_title: &'static str,
    delayed_title: &'static str,
    return_title: &'static str,
    delayed_prefix: &'static str,
}

impl<N> JourneyNotifier<N> {
    pub fn new(notifier: N) -> Self {
        Self {
            notifier,
            group: "train-journey-helper",
            status_tag: "train-journey-status",
            return_tag: "train-journey-return",
            status_title: "Train options",
            delayed_title: "Train delayed",
            return_title: "Return train options",
            delayed_prefix: "A train is delayed. Best current option:",
        }
    }
}

impl<N: Notifier> JourneyNotifier<N> {
    pub async fn send_status_report(&self, options: &[JourneyOption]) -> Result<()> {
        // TODO: We should give pairs of 1-2 and 4-5 trips and some indication that return isn't
        // cancelled
        let mut message = String::new();
        for option in options {
            message.push_str(&format_journey(option)?);
            message.push('\n');
        }
        self.notifier
            .send_notification(self.status_title, &message, self.status_tag, self.group)
            .await
    }

    pub async fn send_train_delayed_before_train_1(&self, best: &JourneyOption) -> Result<()> {
        self.notifier
            .send_notification(
                self.delayed_title,
                &format!("{}\n{}", self.delayed_prefix, format_journey(best)?),
                self.status_tag,
                self.group,
            )
            .await
    }

    pub async fn clear_status_report(&self) -> Result<()> {
        self.notifier.clear_notification(self.status_tag).await
    }

    pub async fn clear_return_details(&self) -> Result<()> {
        self.notifier.clear_notification(self.return_tag).await
    }

    pub async fn clear_notifications(&self) -> Result<()> {
        self.clear_status_report().await?;
        self.clear_return_details().await
    }

    pub async fn send_return_details(&self, options: &[JourneyOption]) -> Result<()> {
        let mut message = String::new();
        for option in options {
            message.push_str(&format_journey(option)?);
            message.push('\n');
        }
        self.notifier
            .send_notification(self.return_title, &message, self.return_tag, self.group)
            .await
    }
}

fn format_journey(option: &JourneyOption) -> Result<String> {
    Ok(format!(
        "{} {}→{} {} (arr {}) then {}→{} {} (arr {}) via {}; walk {}m",
        format_time(option.outbound_first_leg.estimated_departure)?,
        option.outbound_first_leg.from,
        option.outbound_first_leg.to,
        option.outbound_first_leg.company,
        format_time(option.outbound_first_leg.estimated_arrival)?,
        option.outbound_second_leg.from,
        option.outbound_second_leg.to,
        option.outbound_second_leg.company,
        format_time(option.outbound_second_leg.estimated_arrival)?,
        option.outbound_second_leg.route_destination,
        option.interchange_walk_minutes,
    ))
}

fn format_time(time: Timestamp) -> Result<String> {
    Ok(time.in_tz("Europe/London")?.strftime("%H:%M").to_string())
}
