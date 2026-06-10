mod types;

use anyhow::{Context, Result, ensure};
use jiff::{Span, Timestamp};
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::{
    config::{Config, RttConfig},
    custom_types::TrainService,
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
        let response: types::GetAccessTokenResponse = self
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
        let access_token = self.access_token().await?;
        let response: types::SearchResponse = self
            .http
            .get("https://data.rtt.io/gb-nr/location")
            .bearer_auth(access_token)
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

        response
            .services
            .into_iter()
            .map(TrainService::try_from)
            .filter(|service| {
                service
                    .as_ref()
                    .map_or(true, |service| service.estimated_departure >= not_before)
            })
            .collect()
    }
}
