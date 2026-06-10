mod app;
mod config;
mod custom_types;
mod home_assistant;
mod notifier;
mod provider;
mod rtt;
mod station;
mod timezone;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use axum::{Router, extract::State, http::StatusCode, routing::post};
use clap::{CommandFactory, Parser, Subcommand};
use jiff::{Timestamp, civil::DateTime};
use tokio::sync::Mutex;

use crate::{
    app::{App, JourneyState},
    config::Config,
    home_assistant::HomeAssistantNotifier,
    notifier::Notifier,
    provider::TrainProvider,
    rtt::RttClient,
    station::Station,
    timezone::DateTimeExt,
};

#[derive(Parser)]
#[command(version, author, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the main poll loop and HTTP server
    Run {
        /// Path to the JSON configuration file
        #[arg(short, long, default_value = "config.json")]
        config: PathBuf,
    },

    /// Test remote APIs
    Debug {
        #[command(subcommand)]
        command: DebugCommands,
    },

    /// Parse and print the config file
    Config {
        /// Path to the JSON configuration file
        #[arg(short, long, default_value = "config.json")]
        config: PathBuf,
    },

    /// Generate shell completions
    Completions {
        /// The shell to generate the completions for
        #[arg(value_enum)]
        shell: clap_complete_command::Shell,
    },
}

#[derive(Subcommand)]
enum DebugCommands {
    /// Send notification
    Notify {
        /// Path to the JSON configuration file
        #[arg(short, long, default_value = "config.json")]
        config: PathBuf,

        #[arg(short, long, default_value = "title")]
        title: String,

        #[arg(short, long, default_value = "message")]
        message: String,

        #[arg(short = 'T', long, default_value = "test-tag")]
        tag: String,

        #[arg(short, long, default_value = "test-group")]
        group: String,
    },

    /// Clear notification
    ClearNotification {
        /// Path to the JSON configuration file
        #[arg(short, long, default_value = "config.json")]
        config: PathBuf,

        #[arg(short = 'T', long, default_value = "test-tag")]
        tag: String,
    },

    /// Get train journeys
    GetTrains {
        /// Path to the JSON configuration file
        #[arg(short, long, default_value = "config.json")]
        config: PathBuf,

        from: Station,
        to: Station,

        /// Earliest departure time to consider, defaults to now
        #[arg(short = 't', long)]
        not_before: Option<DateTime>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    timezone::init()?;

    match cli.command {
        Commands::Run { config } => run(&config).await?,
        Commands::Debug { command } => match command {
            DebugCommands::Notify {
                config,
                title,
                message,
                tag,
                group,
            } => {
                let config = Config::from_json(&config)?;
                let client = reqwest::Client::new();
                let notifier = HomeAssistantNotifier::new(&config, client);
                notifier
                    .send_notification(&title, &message, &tag, &group)
                    .await?;
            }
            DebugCommands::ClearNotification { config, tag } => {
                let config = Config::from_json(&config)?;
                let client = reqwest::Client::new();
                let notifier = HomeAssistantNotifier::new(&config, client);
                notifier.clear_notification(&tag).await?;
            }
            DebugCommands::GetTrains {
                config,
                from,
                to,
                not_before,
            } => {
                let config = Config::from_json(&config)?;
                let client = reqwest::Client::new();
                let provider = RttClient::new(&config, client);
                let not_before = if let Some(not_before) = not_before {
                    not_before.to_london_zoned()?.timestamp()
                } else {
                    Timestamp::now()
                };
                let trains = provider
                    .departures_between(from, to, not_before)
                    .await
                    .context("Failed to get trains")?;
                println!("{:#?}", trains);
            }
        },
        Commands::Config { config } => {
            let config = Config::from_json(&config)?;
            println!("{:#?}", config);
        }
        Commands::Completions { shell } => {
            shell.generate(&mut Cli::command(), &mut std::io::stdout());
        }
    }

    Ok(())
}

async fn run(config_path: &Path) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::from_json(config_path)?;
    let client = reqwest::Client::new();
    let app = Arc::new(App::<RttClient, HomeAssistantNotifier>::new(config, client));
    app.send_healthcheck().await?;

    let state = app.state_handle();
    // TODO: Don't add stations to paths
    // TODO: Add endpoint for exposing configured stations to help UI label endpoints
    // TODO: Add span for each request
    // TODO: Add time as input
    // TODO: Add endpoint to send notification now
    let router = Router::new()
        .route(
            "/update-status/on-train-1-2",
            post(set_on_train_home_to_interchange),
        )
        .route(
            "/update-status/on-train-4-5",
            post(set_on_train_interchange_to_destination),
        )
        .route("/update-status/skip-day", post(set_skip_day))
        .route(
            "/update-status/skip-until-4",
            post(set_skip_until_destination),
        )
        .route("/update-status/at-5", post(set_at_destination))
        .route(
            // TODO: Should be more specific, will be when taking in time
            "/update-status/on-train-5-23",
            post(set_on_train_destination_to_interchange),
        )
        .route(
            "/update-status/on-train-23-1",
            post(set_on_train_interchange_to_home),
        )
        .route("/update-status/at-1", post(set_complete))
        .with_state(state);

    let scheduler = Arc::clone(&app);
    // TODO: Use JoinSet
    tokio::spawn(async move {
        if let Err(error) = scheduler.run_scheduler().await {
            tracing::error!(?error, "scheduler failed");
        }
    });

    let listener = tokio::net::TcpListener::bind(app.config.listen_addr).await?;
    tracing::info!(addr = %app.config.listen_addr, "listening");
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

type SharedState = State<Arc<Mutex<JourneyState>>>;

async fn set_state(State(state): SharedState, next: JourneyState) -> StatusCode {
    *state.lock().await = next;
    StatusCode::NO_CONTENT
}

async fn set_on_train_home_to_interchange(state: SharedState) -> StatusCode {
    set_state(state, JourneyState::OnTrainHomeToInterchange).await
}

async fn set_on_train_interchange_to_destination(state: SharedState) -> StatusCode {
    set_state(state, JourneyState::OnTrainInterchangeToDestination).await
}

async fn set_skip_day(state: SharedState) -> StatusCode {
    set_state(state, JourneyState::SkippedDay).await
}

async fn set_skip_until_destination(state: SharedState) -> StatusCode {
    set_state(state, JourneyState::SkippedUntilDestination).await
}

async fn set_at_destination(state: SharedState) -> StatusCode {
    set_state(state, JourneyState::AtDestination).await
}

async fn set_on_train_destination_to_interchange(state: SharedState) -> StatusCode {
    set_state(state, JourneyState::OnTrainDestinationToInterchange).await
}

async fn set_on_train_interchange_to_home(state: SharedState) -> StatusCode {
    set_state(state, JourneyState::OnTrainInterchangeToHome).await
}

async fn set_complete(state: SharedState) -> StatusCode {
    set_state(state, JourneyState::Complete).await
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
