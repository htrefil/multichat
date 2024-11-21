mod config;
mod server;
mod tls;

use clap::Parser;
use config::Config;
use multichat_proto::Config as ProtoConfig;
use std::path::PathBuf;
use std::process::ExitCode;
use tls::DefaultAcceptor;
use tokio::fs;
use tracing::subscriber;
use tracing_subscriber::filter::{EnvFilter, LevelFilter};
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;

#[derive(Parser)]
#[clap(name = "multichat-server", about = "Multichat server")]
struct Args {
    #[clap(help = "Path to configuration file")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> ExitCode {
    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().without_time().with_target(false));

    subscriber::set_global_default(registry).unwrap();

    let args = Args::parse();
    let config = match fs::read_to_string(&args.config).await {
        Ok(config) => config,
        Err(err) => {
            tracing::error!("Error reading config: {}", err);
            return ExitCode::FAILURE;
        }
    };

    let config = match toml::from_str::<Config>(&config) {
        Ok(config) => config,
        Err(err) => {
            tracing::error!("Error parsing config: {}", err);
            return ExitCode::FAILURE;
        }
    };

    let mut proto_config = ProtoConfig::default();
    proto_config.max_size(config.max_size);

    let result = match config.tls {
        Some(tls) => {
            let acceptor = match tls::configure(&tls.certificate, &tls.key).await {
                Ok(acceptor) => acceptor,
                Err(err) => {
                    tracing::error!("Error configuring TLS: {}", err);
                    return ExitCode::FAILURE;
                }
            };

            server::run(
                config.listen,
                acceptor,
                config.groups,
                config.update_buffer,
                config.access_tokens,
                proto_config,
                config.ping_interval,
                config.ping_timeout,
            )
            .await
        }
        None => {
            server::run(
                config.listen,
                DefaultAcceptor,
                config.groups,
                config.update_buffer,
                config.access_tokens,
                proto_config,
                config.ping_interval,
                config.ping_timeout,
            )
            .await
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!("Error: {}", err);
            ExitCode::FAILURE
        }
    }
}
