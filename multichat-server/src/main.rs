mod config;
mod tls;

use config::Config;
use log::LevelFilter;
use multichat_proto::{Chunk, ClientMessage, ServerInit, ServerMessage};
use slab::Slab;
use std::collections::HashMap;
use std::convert::Infallible;
use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use structopt::StructOpt;
use tls::{Acceptor, DefaultAcceptor};
use tokio::fs;
use tokio::io::{self, AsyncRead, AsyncWrite, BufReader, BufWriter};
use tokio::net::TcpListener;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::{self, Sender};
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use tokio_native_tls::native_tls::{self, Identity};
use tokio_native_tls::TlsAcceptor;

#[derive(StructOpt)]
#[structopt(name = "multichat-server", about = "Multichat server")]
struct Args {
    #[structopt(help = "Path to configuration file")]
    config_path: PathBuf,
}

#[tokio::main]
async fn main() {
    env_logger::builder()
        .format_timestamp(None)
        .filter(None, LevelFilter::Info)
        .parse_default_env()
        .init();

    let args = Args::from_args();
    let config = match fs::read_to_string(&args.config_path).await {
        Ok(config) => config,
        Err(err) => {
            log::error!("Error reading config: {}", err);
            process::exit(1);
        }
    };

    let config: Config = match toml::from_str(&config) {
        Ok(config) => config,
        Err(err) => {
            log::error!("Error parsing config: {}", err);
            process::exit(1);
        }
    };

    let update_buffer = config.update_buffer.map(NonZeroUsize::get).unwrap_or(256);
    let state = State {
        groups: config
            .groups
            .into_iter()
            .map(|name| Group {
                name,
                users: RwLock::new(Slab::new()),
                sender: broadcast::channel(update_buffer).0,
            })
            .collect(),
    };

    let result = match config.tls {
        Some(tls) => {
            let identity = match fs::read(&tls.identity_path).await {
                Ok(identity) => identity,
                Err(err) => {
                    log::error!("Error reading identity file: {}", err);
                    process::exit(1);
                }
            };

            let identity = match Identity::from_pkcs12(&identity, &tls.identity_password) {
                Ok(identity) => identity,
                Err(err) => {
                    log::error!("Error parsing identity: {}", err);
                    process::exit(1);
                }
            };

            let acceptor: TlsAcceptor = match native_tls::TlsAcceptor::new(identity) {
                Ok(acceptor) => acceptor.into(),
                Err(err) => {
                    log::error!("Error creating TLS acceptor: {}", err);
                    process::exit(1);
                }
            };

            handle_server(config.listen_addr, acceptor, state).await
        }
        None => handle_server(config.listen_addr, DefaultAcceptor, state).await,
    };

    log::error!("Server error: {}", result.unwrap_err());
    process::exit(1);
}

async fn handle_server(
    listen_addr: SocketAddr,
    acceptor: impl Acceptor,
    state: State,
) -> Result<Infallible, Error> {
    let listener = TcpListener::bind(&listen_addr).await?;

    log::info!("Listening on {}", listen_addr);

    let state = Arc::new(state);
    loop {
        let (stream, addr) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let state = state.clone();

        tokio::spawn(async move {
            log::info!("{}: Connected", addr);

            let stream = match acceptor.accept(stream).await {
                Ok(stream) => stream,
                Err(err) => {
                    log::error!("{}: TLS error: {}", addr, err);
                    return;
                }
            };

            match handle_connection(stream, addr, &state).await {
                Ok(_) => log::info!("{}: Disconnected", addr),
                Err(err) => log::error!("{}: Disconnected: {}", addr, err),
            }

            // Remove all users created by this connection which didn't leave on their own.
            for group in &state.groups {
                group.users.write().await.retain(|uid, client| {
                    if client.owner != addr {
                        let _ = group.sender.send(Update::Leave { uid });
                        return false;
                    }

                    true
                });
            }
        });
    }
}

async fn handle_connection(
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + 'static,
    addr: SocketAddr,
    state: &State,
) -> Result<(), Error> {
    let (stream_read, stream_write) = io::split(stream);

    let mut stream_read = BufReader::new(stream_read);
    let mut stream_write = BufWriter::new(stream_write);

    // Send our version.
    multichat_proto::VERSION.write(&mut stream_write).await?;

    // ...and our groups.
    let groups = state
        .groups
        .iter()
        .enumerate()
        .map(|(gid, group)| (group.name.as_str().into(), gid))
        .collect();

    multichat_proto::write(&mut stream_write, &ServerInit { groups }).await?;

    // C2S.
    let (server_sender, mut server_receiver) = mpsc::unbounded_channel::<ClientMessage>();
    let mut server_handle = tokio::spawn(async move {
        loop {
            let message = multichat_proto::read(&mut stream_read).await?;
            if server_sender.send(message).is_err() {
                break;
            }
        }

        Ok::<_, Error>(())
    });

    let (update_sender, mut update_receiver) = mpsc::unbounded_channel();
    let mut group_handles = HashMap::new();

    loop {
        enum LocalUpdate {
            Client(ClientMessage<'static>),
            Group((usize, Update)),
        }

        let update = tokio::select! {
            message = server_receiver.recv() => {
                match message {
                    Some(message) => LocalUpdate::Client(message),
                    // The task exited due to an error, it will be propagated to us on the next iteration of the loop.
                    None => continue,
                }
            }
            update = update_receiver.recv() => {
                // It's not possible for it to fail unless the task panics at which point we can just bring the whole thing down.
                match update.unwrap() {
                    Ok(update) => LocalUpdate::Group(update),
                    Err(num) => return Err(Error::new(ErrorKind::Other, format!("Skipped {} group updates", num))),
                }
            }
            Ok(Err(err)) = &mut server_handle => return Err(err),
        };

        match update {
            LocalUpdate::Client(message) => match message {
                ClientMessage::JoinGroup { gid } => {
                    let group = state.groups.get(gid).ok_or_else(|| {
                        Error::new(ErrorKind::Other, "Attempted to join a nonexistent group")
                    })?;

                    let update_sender = update_sender.clone();
                    let mut receiver = group.sender.subscribe();
                    let prev = group_handles.insert(
                        gid,
                        tokio::spawn(async move {
                            loop {
                                let result = match receiver.recv().await {
                                    Ok(update) => Ok((gid, update)),
                                    Err(RecvError::Lagged(num)) => Err(num),
                                    Err(RecvError::Closed) => return,
                                };

                                // The binary or is intentional, we want the result to be
                                // sent regardless of being an error.
                                if result.is_err() | update_sender.send(result).is_err() {
                                    return;
                                }
                            }
                        }),
                    );

                    if prev.is_some() {
                        return Err(Error::new(
                            ErrorKind::Other,
                            "Attempted to join a group twice",
                        ));
                    }

                    for (uid, client) in &*group.users.read().await {
                        // XXX: Perhaps it would be wise to write the message
                        //      while the client list is not locked.
                        multichat_proto::write(
                            &mut stream_write,
                            &ServerMessage::InitUser {
                                gid,
                                uid,
                                name: client.name.as_str().into(),
                            },
                        )
                        .await?;
                    }

                    log::debug!("{}: Join group - GID: {}", addr, gid);
                }
                ClientMessage::LeaveGroup { gid } => {
                    group_handles
                        .remove(&gid)
                        .ok_or_else(|| {
                            Error::new(ErrorKind::Other, "Attempted to leave a non-joined group")
                        })?
                        .abort();

                    log::debug!("{}: Leave group - GID: {}", addr, gid);
                }
                ClientMessage::JoinUser { gid, name } => {
                    let group = state.groups.get(gid).ok_or_else(|| {
                        Error::new(
                            ErrorKind::Other,
                            "Attempted to join a client to a nonexistent group",
                        )
                    })?;

                    let uid = group.users.write().await.insert(Client {
                        name: name.clone().into(),
                        owner: addr,
                    });

                    // Notify our client.
                    multichat_proto::write(
                        &mut stream_write,
                        &ServerMessage::ConfirmClient { uid },
                    )
                    .await?;

                    // Notify our group.
                    let _ = group.sender.send(Update::Join {
                        uid,
                        name: name.clone().into(),
                    });

                    log::debug!(
                        "{}: Join client - GID: {}, name: {:?}, CID: {}",
                        addr,
                        gid,
                        name,
                        uid
                    );
                }
                ClientMessage::LeaveUser { gid, uid } => {
                    let group = state.groups.get(gid).ok_or_else(|| {
                        Error::new(
                            ErrorKind::Other,
                            "Attempted to remove a client from a nonexistent group",
                        )
                    })?;

                    let mut users = group.users.write().await;
                    let client = users.get(uid).ok_or_else(|| {
                        Error::new(ErrorKind::Other, "Attempted to remove a nonexistent client")
                    })?;

                    if client.owner != addr {
                        return Err(Error::new(
                            ErrorKind::Other,
                            "Attempted to remove a non owned client",
                        ));
                    }

                    users.remove(uid);

                    // Notify our group.
                    let _ = group.sender.send(Update::Leave { uid });

                    log::debug!("{}: Remove client - GID: {}, CID: {}", addr, gid, uid);
                }
                ClientMessage::SendMessage { gid, uid, message } => {
                    let group = state.groups.get(gid).ok_or_else(|| {
                        Error::new(
                            ErrorKind::Other,
                            "Attempted to send a message as a client from a nonexistent group",
                        )
                    })?;

                    let users = group.users.read().await;
                    let client = users.get(uid).ok_or_else(|| {
                        Error::new(
                            ErrorKind::Other,
                            "Attempted to send a message as a nonexistent client",
                        )
                    })?;

                    if client.owner != addr {
                        return Err(Error::new(
                            ErrorKind::Other,
                            "Attempted to send a message as a non owned client",
                        ));
                    }

                    let message_text = message
                        .iter()
                        .map(|chunk| chunk.contents.as_ref())
                        .collect::<String>();

                    // Notify our group.
                    let _ = group.sender.send(Update::Message {
                        uid,
                        message: message.into_owned().into(),
                    });

                    log::debug!(
                        "{}: Send message - GID: {}, CID: {}, message: {:?}",
                        addr,
                        gid,
                        uid,
                        message_text
                    );
                }
            },
            LocalUpdate::Group((gid, update)) => {
                let message = match update {
                    Update::Join { uid, name } => ServerMessage::InitUser {
                        gid,
                        uid,
                        name: name.into(),
                    },
                    Update::Leave { uid } => ServerMessage::LeaveUser { gid, uid },
                    Update::Message { uid, message } => ServerMessage::Message {
                        gid,
                        uid,
                        message: message.into(),
                    },
                };

                multichat_proto::write(&mut stream_write, &message).await?;
            }
        }
    }
}

struct State {
    groups: Vec<Group>,
}

struct Group {
    name: String,
    users: RwLock<Slab<Client>>,
    sender: Sender<Update>,
}

pub struct Client {
    name: String,
    // Owning connection.
    owner: SocketAddr,
}

#[derive(Clone)]
enum Update {
    Join {
        uid: usize,
        // Name is included here due to the ABA problem.
        name: String,
    },
    Leave {
        uid: usize,
    },
    Message {
        uid: usize,
        message: Vec<Chunk<'static>>,
    },
}
