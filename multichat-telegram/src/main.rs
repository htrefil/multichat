mod config;
mod markdown_safe;
mod multichat;
mod telegram;
mod tls;

use clap::Parser;
use config::Config;
use multichat_client::proto::Config as ProtoConfig;
use multichat_client::ClientBuilder;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::ExitCode;
use teloxide::types::ChatId;
use teloxide::Bot;
use tokio::fs;
use tokio::sync::mpsc;
use tracing::subscriber;
use tracing_subscriber::filter::{EnvFilter, LevelFilter};
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;

#[derive(Parser)]
struct Args {
    #[clap(help = "Path to config file")]
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

    tracing::info!("Reading config from {}", args.config.display());

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

    let bot = Bot::new(config.telegram.token);

    let connector = match config.multichat.certificate {
        Some(certificate) => match tls::configure(&certificate).await {
            Ok(connector) => Some(connector),
            Err(err) => {
                tracing::error!("Error configuring TLS: {}", err);
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };

    let mut proto_config = ProtoConfig::default();
    proto_config.max_size(512 * 1024 * 1024); // 512 MiB

    let mut client = match ClientBuilder::maybe_tls(connector)
        .config(proto_config)
        .connect(&config.multichat.server, config.multichat.access_token)
        .await
    {
        Ok(client) => client,
        Err(err) => {
            tracing::error!("Error connecting to multichat: {}", err);
            return ExitCode::FAILURE;
        }
    };

    tracing::info!("Connected to Multichat");

    let mut chat_to_group = HashMap::new();
    let mut group_to_chat = HashMap::new();

    for chat in config.chats {
        let gid = match client.join_group(&chat.multichat_group).await {
            Ok(gid) => gid,
            Err(err) => {
                tracing::error!("Error joining group: {}", err);
                return ExitCode::FAILURE;
            }
        };

        let inserted = chat_to_group
            .entry(ChatId(chat.telegram_chat))
            .or_insert_with(HashSet::new)
            .insert(gid);

        if !inserted {
            tracing::error!(
                "Telegram chat {} is already associated with Multichat group {}",
                chat.telegram_chat,
                chat.multichat_group
            );

            return ExitCode::FAILURE;
        }

        let inserted = group_to_chat
            .entry(gid)
            .or_insert_with(HashSet::new)
            .insert(ChatId(chat.telegram_chat));

        if !inserted {
            tracing::error!(
                "Multichat group {} is already associated with Telegram chat {}",
                chat.multichat_group,
                chat.telegram_chat
            );

            return ExitCode::FAILURE;
        }
    }

    let (sender, receiver) = mpsc::channel(1);

    let telegram = tokio::spawn(telegram::run(bot.clone(), sender));
    let multichat = tokio::spawn(async move {
        multichat::run(client, bot, &chat_to_group, &group_to_chat, receiver).await
    });

    let result = tokio::select! {
        result = telegram => {
            result.unwrap();
            Ok(())
        },
        result = multichat => result.unwrap(),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!("Error: {}", err);
            ExitCode::FAILURE
        }
    }
}
