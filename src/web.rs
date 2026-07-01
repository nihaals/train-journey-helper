use std::{future::IntoFuture, sync::Arc};

use anyhow::Result;
use axum::{Router, extract::State, http::StatusCode};
use jiff::{Timestamp, civil::DateTime};
use serde::Deserialize;
use tokio::net::TcpListener;

use crate::{
    app::{self, App, JourneyState},
    home_assistant::HomeAssistantNotifier,
    rtt::RttClient,
    timezone::DateTimeExt,
};

pub fn serve(
    listener: TcpListener,
    // TODO: Make generic
    app: Arc<App<RttClient, HomeAssistantNotifier>>,
) -> impl IntoFuture<Output = std::io::Result<()>> {
    let router = router(app);
    axum::serve(listener, router)
        // TODO: Investigate
        .with_graceful_shutdown(shutdown_signal())
}

fn router(app: Arc<App<RttClient, HomeAssistantNotifier>>) -> Router {
    // TODO: Add span for each request
    Router::new()
        .route("/config", axum::routing::get(get_config))
        .route(
            "/services/leg/home-to-primary-interchange",
            axum::routing::post(get_home_to_primary_interchange_services),
        )
        .route(
            "/services/leg/interchange-to-destination",
            axum::routing::post(get_interchange_to_destination_services),
        )
        .route(
            "/services/leg/destination-to-interchange",
            axum::routing::post(get_destination_to_interchange_services),
        )
        .route(
            "/services/leg/return-preferred-interchange-to-home",
            axum::routing::post(get_return_preferred_interchange_to_home_services),
        )
        .route(
            "/services/leg/primary-interchange-to-home",
            axum::routing::post(get_primary_interchange_to_home_services),
        )
        .route(
            "/status/on-train/home-to-primary-interchange",
            axum::routing::post(set_on_train_home_to_primary_interchange),
        )
        .route(
            "/status/on-train/interchange-to-destination",
            axum::routing::post(set_on_train_interchange_to_destination),
        )
        .route(
            "/status/on-train/destination-to-interchange",
            axum::routing::post(set_on_train_destination_to_interchange),
        )
        .route(
            "/status/on-train/return-preferred-interchange-to-home",
            axum::routing::post(set_on_train_return_preferred_interchange_to_home),
        )
        .route(
            "/status/on-train/primary-interchange-to-home",
            axum::routing::post(set_on_train_primary_interchange_to_home),
        )
        .route("/status/skip-day", axum::routing::post(set_skip_day))
        .route(
            "/status/at-destination",
            axum::routing::post(set_at_destination),
        )
        .route("/status/complete", axum::routing::post(set_complete))
        .route(
            "/notification/resend",
            axum::routing::post(resend_notification),
        )
        .with_state(app)
}

type SharedApp = State<Arc<App<RttClient, HomeAssistantNotifier>>>;

#[derive(Deserialize)]
struct TimeRequest {
    time: DateTime,
}

#[derive(Deserialize)]
struct ServiceRequest {
    service_id: String,
}

fn request_time(request: TimeRequest) -> Result<Timestamp> {
    Ok(request.time.to_london_zoned()?.timestamp())
}

async fn get_config(State(app): SharedApp) -> axum::Json<app::StationConfigResponse> {
    axum::Json(app.config_response())
}

async fn get_leg_options(
    State(app): SharedApp,
    axum::Json(request): axum::Json<TimeRequest>,
    leg: app::JourneyLeg,
) -> Result<axum::Json<Vec<app::TrainResponse>>, StatusCode> {
    let time = request_time(request).map_err(|_| StatusCode::BAD_REQUEST)?;
    let options = app.trains_for_leg(leg, time).await.map_err(|error| {
        // TODO: Casing?
        tracing::error!(?error, "failed to get leg options");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(axum::Json(
        options
            .iter()
            .cloned()
            .map(app::TrainResponse::from)
            .collect(),
    ))
}

// TODO: DRY
async fn get_home_to_primary_interchange_services(
    state: SharedApp,
    request: axum::Json<TimeRequest>,
) -> Result<axum::Json<Vec<app::TrainResponse>>, StatusCode> {
    get_leg_options(state, request, app::JourneyLeg::HomeToPrimaryInterchange).await
}

async fn get_interchange_to_destination_services(
    state: SharedApp,
    request: axum::Json<TimeRequest>,
) -> Result<axum::Json<Vec<app::TrainResponse>>, StatusCode> {
    get_leg_options(state, request, app::JourneyLeg::InterchangeToDestination).await
}

async fn get_destination_to_interchange_services(
    state: SharedApp,
    request: axum::Json<TimeRequest>,
) -> Result<axum::Json<Vec<app::TrainResponse>>, StatusCode> {
    get_leg_options(state, request, app::JourneyLeg::DestinationToInterchange).await
}

async fn get_return_preferred_interchange_to_home_services(
    state: SharedApp,
    request: axum::Json<TimeRequest>,
) -> Result<axum::Json<Vec<app::TrainResponse>>, StatusCode> {
    get_leg_options(
        state,
        request,
        app::JourneyLeg::ReturnPreferredInterchangeToHome,
    )
    .await
}

async fn get_primary_interchange_to_home_services(
    state: SharedApp,
    request: axum::Json<TimeRequest>,
) -> Result<axum::Json<Vec<app::TrainResponse>>, StatusCode> {
    get_leg_options(state, request, app::JourneyLeg::PrimaryInterchangeToHome).await
}

async fn set_on_train(
    State(app): SharedApp,
    axum::Json(request): axum::Json<ServiceRequest>,
    leg: app::JourneyLeg,
) -> StatusCode {
    match app.set_on_train(leg, &request.service_id).await {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(error) => {
            tracing::error!(?error, "failed to set selected train");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn set_on_train_home_to_primary_interchange(
    state: SharedApp,
    request: axum::Json<ServiceRequest>,
) -> StatusCode {
    set_on_train(state, request, app::JourneyLeg::HomeToPrimaryInterchange).await
}

async fn set_on_train_interchange_to_destination(
    state: SharedApp,
    request: axum::Json<ServiceRequest>,
) -> StatusCode {
    set_on_train(state, request, app::JourneyLeg::InterchangeToDestination).await
}

async fn set_on_train_destination_to_interchange(
    state: SharedApp,
    request: axum::Json<ServiceRequest>,
) -> StatusCode {
    set_on_train(state, request, app::JourneyLeg::DestinationToInterchange).await
}

async fn set_on_train_return_preferred_interchange_to_home(
    state: SharedApp,
    request: axum::Json<ServiceRequest>,
) -> StatusCode {
    set_on_train(
        state,
        request,
        app::JourneyLeg::ReturnPreferredInterchangeToHome,
    )
    .await
}

async fn set_on_train_primary_interchange_to_home(
    state: SharedApp,
    request: axum::Json<ServiceRequest>,
) -> StatusCode {
    set_on_train(state, request, app::JourneyLeg::PrimaryInterchangeToHome).await
}

async fn set_skip_day(State(app): SharedApp) -> StatusCode {
    app.set_state(JourneyState::SkippedDay).await;
    StatusCode::NO_CONTENT
}

async fn set_at_destination(State(app): SharedApp) -> StatusCode {
    app.set_state(JourneyState::AtDestination).await;
    StatusCode::NO_CONTENT
}

async fn set_complete(State(app): SharedApp) -> StatusCode {
    app.set_state(JourneyState::Complete).await;
    StatusCode::NO_CONTENT
}

async fn resend_notification(State(app): SharedApp) -> StatusCode {
    match app.send_last_notification().await {
        Ok(true) => StatusCode::NO_CONTENT,
        Ok(false) => StatusCode::NOT_FOUND,
        Err(error) => {
            tracing::error!(?error, "failed to resend notification");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
