use std::sync::Arc;

use anyhow::Result;
use axum::{Router, extract::State, http::StatusCode, response::Response, routing::MethodRouter};
use jiff::{Timestamp, civil::DateTime};
use serde::Deserialize;
use tokio::net::TcpListener;
use tracing::{Instrument, error, info_span};

use crate::{
    app::{self, App, JourneyState},
    home_assistant::HomeAssistantNotifier,
    rtt::RttClient,
    timezone::DateTimeExt,
};

// TODO: Make generic
type AppState = Arc<App<RttClient, HomeAssistantNotifier>>;
type AppStateState = State<AppState>;

pub async fn serve(listener: TcpListener, app: AppState) -> std::io::Result<()> {
    let router = router(app);
    axum::serve(listener, router).await
}

fn router(app: AppState) -> Router {
    Router::new()
        .route("/config", axum::routing::get(get_config))
        .route(
            "/services/leg/home-to-primary-interchange",
            get_leg_options(app::JourneyLeg::HomeToPrimaryInterchange),
        )
        .route(
            "/services/leg/interchange-to-destination",
            get_leg_options(app::JourneyLeg::InterchangeToDestination),
        )
        .route(
            "/services/leg/destination-to-interchange",
            get_leg_options(app::JourneyLeg::DestinationToInterchange),
        )
        .route(
            "/services/leg/return-preferred-interchange-to-home",
            get_leg_options(app::JourneyLeg::ReturnPreferredInterchangeToHome),
        )
        .route(
            "/services/leg/primary-interchange-to-home",
            get_leg_options(app::JourneyLeg::PrimaryInterchangeToHome),
        )
        .route(
            "/status/on-train/home-to-primary-interchange",
            set_on_train(app::JourneyLeg::HomeToPrimaryInterchange),
        )
        .route(
            "/status/on-train/interchange-to-destination",
            set_on_train(app::JourneyLeg::InterchangeToDestination),
        )
        .route(
            "/status/on-train/destination-to-interchange",
            set_on_train(app::JourneyLeg::DestinationToInterchange),
        )
        .route(
            "/status/on-train/return-preferred-interchange-to-home",
            set_on_train(app::JourneyLeg::ReturnPreferredInterchangeToHome),
        )
        .route(
            "/status/on-train/primary-interchange-to-home",
            set_on_train(app::JourneyLeg::PrimaryInterchangeToHome),
        )
        .route("/status/skip-day", axum::routing::post(set_complete))
        .route(
            "/status/at-destination",
            axum::routing::post(set_at_destination),
        )
        .route("/status/complete", axum::routing::post(set_complete))
        .route(
            "/notification/resend",
            axum::routing::post(resend_notification),
        )
        .layer(axum::middleware::from_fn(trace_requests))
        .with_state(app)
}

#[derive(Deserialize)]
struct TimeRequest {
    time: DateTime,
}

#[derive(Deserialize)]
struct ServiceRequest {
    service_id: String,
}

fn request_time(request: &TimeRequest) -> Result<Timestamp> {
    Ok(request.time.to_london_zoned()?.timestamp())
}

async fn trace_requests(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let span = info_span!(
        "http_request",
        method = %req.method(),
        path = req.uri().path(),
        status = tracing::field::Empty,
    );
    let response = next.run(req).instrument(span.clone()).await;
    span.record("status", response.status().as_u16());
    response
}

async fn get_config(State(app): AppStateState) -> axum::Json<app::StationConfigResponse> {
    axum::Json(app.config_response())
}

fn get_leg_options(leg: app::JourneyLeg) -> MethodRouter<AppState> {
    axum::routing::post(
        move |State(app): AppStateState, axum::Json(request): axum::Json<TimeRequest>| async move {
            let time = request_time(&request).map_err(|_| StatusCode::BAD_REQUEST)?;
            let options = app.trains_for_leg(leg, time).await.map_err(|error| {
                error!(?error, "failed to get leg options");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
            Ok::<_, StatusCode>(axum::Json(
                // TODO: If this was an iterator we wouldn't use all of it
                options
                    .into_iter_by_departure()
                    .map(app::TrainResponse::from)
                    .collect::<Vec<_>>(),
            ))
        },
    )
}

fn set_on_train(leg: app::JourneyLeg) -> MethodRouter<AppState> {
    axum::routing::post(
        move |State(app): AppStateState, axum::Json(request): axum::Json<ServiceRequest>| async move {
            match app.set_on_train(leg, &request.service_id).await {
                Ok(()) => StatusCode::NO_CONTENT,
                Err(error) => {
                    error!(?error, "failed to set selected train");
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }
        },
    )
}

async fn set_at_destination(State(app): AppStateState) -> StatusCode {
    app.set_state(JourneyState::AtDestination).await;
    StatusCode::NO_CONTENT
}

async fn set_complete(State(app): AppStateState) -> StatusCode {
    match app.complete_journey().await {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(error) => {
            error!(?error, "failed to complete journey");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn resend_notification(State(app): AppStateState) -> StatusCode {
    match app.send_last_notification().await {
        Ok(true) => StatusCode::NO_CONTENT,
        Ok(false) => StatusCode::NOT_FOUND,
        Err(error) => {
            error!(?error, "failed to resend notification");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
