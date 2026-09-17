#![warn(clippy::clone_on_ref_ptr)]
#![warn(clippy::needless_pass_by_value)]

mod app;
mod config;
mod custom_types;
mod home_assistant;
mod notifier;
mod provider;
mod rtt;
mod station;
mod timezone;
mod web;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use clap::{CommandFactory, Parser, Subcommand};
use jiff::{Timestamp, civil::DateTime};
use tokio::task::JoinSet;

use crate::{
    app::App, config::Config, home_assistant::HomeAssistantNotifier, notifier::Notifier,
    provider::TrainProvider, rtt::RttClient, station::Station, timezone::DateTimeExt,
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

        /// Proxy to use (disables certificate verification), e.g. `http://localhost:8080`
        #[arg(long)]
        proxy: Option<String>,

        from: Station,
        to: Station,

        /// Earliest departure time to consider, defaults to now
        #[arg(short = 't', long)]
        not_before: Option<DateTime>,
    },

    /// Generate initial status report
    StatusReport {
        /// Path to the JSON configuration file
        #[arg(short, long, default_value = "config.json")]
        config: PathBuf,

        /// Proxy to use (disables certificate verification), e.g. `http://localhost:8080`
        #[arg(long)]
        proxy: Option<String>,
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
                proxy,
                from,
                to,
                not_before,
            } => {
                let config = Config::from_json(&config)?;
                let client = if let Some(proxy) = proxy {
                    reqwest::Client::builder()
                        .proxy(reqwest::Proxy::all(proxy)?)
                        .danger_accept_invalid_certs(true)
                        .build()?
                } else {
                    reqwest::Client::new()
                };
                let provider = RttClient::new(&config, client);
                let not_before = if let Some(not_before) = not_before {
                    not_before.to_london_zoned()?.timestamp()
                } else {
                    Timestamp::now()
                };
                let trains = provider
                    .departures_between(from, to, not_before)
                    .await
                    .context("Failed to get trains")?
                    .into_iter_by_departure()
                    .collect::<Vec<_>>();
                println!("{:#?}", trains);
            }
            DebugCommands::StatusReport { config, proxy } => {
                let config = Config::from_json(&config)?;
                let client = if let Some(proxy) = proxy {
                    reqwest::Client::builder()
                        .proxy(reqwest::Proxy::all(proxy)?)
                        .danger_accept_invalid_certs(true)
                        .build()?
                } else {
                    reqwest::Client::new()
                };
                let app = App::<RttClient, HomeAssistantNotifier>::new(config, client);
                let report = app
                    .debug_status_report(Timestamp::now())
                    .await
                    .context("Failed to get status report")?;
                println!("{report}");
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

    let listener = tokio::net::TcpListener::bind(app.config.listen_addr).await?;
    tracing::info!(addr = %app.config.listen_addr, "listening");

    let mut join_set = JoinSet::new();
    join_set.spawn({
        let app = Arc::clone(&app);
        async move {
            match app.run_scheduler().await {
                Ok(()) => Err(anyhow!("Scheduler exited")),
                Err(error) => Err(error).context("Scheduler failed"),
            }
        }
    });
    join_set.spawn(async move {
        match web::serve(listener, app).await {
            Ok(()) => Err(anyhow!("HTTP server exited")),
            Err(error) => Err(error).context("HTTP server failed"),
        }
    });

    let first_finished = join_set.join_next().await;
    join_set.abort_all();
    join_set.join_next().await;

    match first_finished {
        Some(Ok(Ok(()))) => Ok(()),
        Some(Ok(Err(error))) => Err(error),
        Some(Err(join_error)) => Err(anyhow!(join_error).context("Failed to join task")),
        None => unreachable!("JoinSet should have at least one task running"),
    }
}
