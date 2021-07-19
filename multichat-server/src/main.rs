mod config;
mod tls;

use config::Config;
use log::LevelFilter;
use multichat_proto::{ClientMessage, Message, ServerInit, ServerMessage};
use slab::Slab;
use std::borrow::Cow;
use std::collections::HashMap;
use std::convert::Infallible;
use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use structopt::StructOpt;
use tls::{Acceptor, DefaultAcceptor};
use tokio::fs;
use tokio::io::{self, AsyncRead, AsyncWrite, BufReader, BufWriter};
use tokio::net::TcpListener;
use tokio::sync::mpsc::{self, Sender};
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

    let state = State {
        groups: config
            .groups
            .into_iter()
            .map(|name| Group {
                name,
                users: RwLock::new(Slab::new()),
                subscribers: RwLock::new(HashMap::new()),
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
                group.cleanup_subscribers(addr).await;
                group.cleanup_users(addr).await;
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
    let (server_sender, mut server_receiver) = mpsc::channel(1);
    tokio::spawn(async move {
        loop {
            let result = multichat_proto::read(&mut stream_read).await;
            if result.is_err() | server_sender.send(result).await.is_err() {
                break;
            }
        }
    });

    let (update_sender, mut update_receiver) = mpsc::channel(state.groups.len().min(1));
    loop {
        enum LocalUpdate {
            Client(ClientMessage<'static, 'static>),
            Group((usize, Update)),
        }

        // It's not possible for the unwraps to fail unless either task panics and at that
        // point we can just bring the whole thing down.
        let update = tokio::select! {
            result = update_receiver.recv() => LocalUpdate::Group(result.unwrap()),
            result = server_receiver.recv() => LocalUpdate::Client(result.unwrap()?),
        };

        match update {
            LocalUpdate::Client(message) => match message {
                ClientMessage::JoinGroup { gid } => {
                    let mut subscribers = state
                        .groups
                        .get(gid)
                        .map(|group| &group.subscribers)
                        .ok_or_else(|| {
                            Error::new(ErrorKind::Other, "Attempt to join a nonexistent group")
                        })?
                        .write()
                        .await;

                    let (sender, mut receiver) = mpsc::channel(1);
                    if subscribers.insert(addr, sender).is_some() {
                        return Err(Error::new(
                            ErrorKind::Other,
                            "Attempt to join a group twice",
                        ));
                    }

                    drop(subscribers);

                    let update_sender = update_sender.clone();
                    tokio::spawn(async move {
                        while let Some(update) = receiver.recv().await {
                            if update_sender.send((gid, update)).await.is_err() {
                                return;
                            }
                        }
                    });

                    log::debug!("{}: Join group - GID: {}", addr, gid);
                }
                ClientMessage::LeaveGroup { gid } => {
                    let group = state.groups.get(gid).ok_or_else(|| {
                        Error::new(ErrorKind::Other, "Attempt to leave a nonexistent group")
                    })?;

                    // Remove ourselves.
                    if !group.cleanup_subscribers(addr).await {
                        return Err(Error::new(
                            ErrorKind::Other,
                            "Attempt to leave a nonjoined group",
                        ));
                    }

                    group.cleanup_users(addr).await;

                    log::debug!("{}: Leave group - GID: {}", addr, gid);
                }
                ClientMessage::JoinUser { gid, name } => {
                    let group = state.groups.get(gid).ok_or_else(|| {
                        Error::new(
                            ErrorKind::Other,
                            "Attempt to join an user to a nonexistent group",
                        )
                    })?;

                    let mut users = group.users.write().await;

                    let name = name.into_owned();
                    let uid = users.insert(User {
                        name: name.clone(),
                        owner: addr,
                    });

                    drop(users);

                    // Notify our group.
                    group
                        .send(
                            addr,
                            Update::Join {
                                uid,
                                name: name.clone(),
                            },
                        )
                        .await;

                    // Notify our client.
                    multichat_proto::write(
                        &mut stream_write,
                        &ServerMessage::ConfirmClient { uid },
                    )
                    .await?;

                    log::debug!("{}: Join user - GID: {}, name: {:?}", addr, gid, name);
                }
                ClientMessage::LeaveUser { gid, uid } => {
                    let group = state.groups.get(gid).ok_or_else(|| {
                        Error::new(
                            ErrorKind::Other,
                            "Attempt to leave an user in a nonexistent group",
                        )
                    })?;

                    let mut users = group.users.write().await;
                    if !users.contains(uid) {
                        return Err(Error::new(
                            ErrorKind::Other,
                            "Attempt to leave a nonexistent user",
                        ));
                    }

                    users.remove(uid);

                    drop(users);

                    // Notify our group.
                    group.send(addr, Update::Leave { uid }).await;

                    log::debug!("{}: Leave user - GID: {}, UID: {}", addr, gid, uid);
                }
                ClientMessage::SendMessage { gid, uid, message } => {
                    // Check validity.
                    let group = state.groups.get(gid).ok_or_else(|| {
                        Error::new(
                            ErrorKind::Other,
                            "Attempt to send a message as an user in a nonexistent group",
                        )
                    })?;

                    let users = group.users.read().await;
                    let user = users.get(uid).ok_or_else(|| {
                        Error::new(
                            ErrorKind::Other,
                            "Attempt to send a message as a nonexistent user",
                        )
                    })?;

                    if user.owner != addr {
                        return Err(Error::new(
                            ErrorKind::Other,
                            "Attempt to send a message as a foreign user",
                        ));
                    }

                    drop(users);

                    let text = message.text().into_owned();

                    // Notify our group.
                    group
                        .send(
                            addr,
                            Update::Message {
                                uid,
                                message: message.into_owned(),
                            },
                        )
                        .await;

                    log::debug!(
                        "{}: Send message - GID: {}, UID: {}, text: {:?}",
                        addr,
                        gid,
                        uid,
                        text
                    );
                }
                ClientMessage::RenameUser { gid, uid, name } => {
                    let group = state.groups.get(gid).ok_or_else(|| {
                        Error::new(
                            ErrorKind::Other,
                            "Attempted to rename an user from a nonexistent group",
                        )
                    })?;

                    let mut users = group.users.write().await;
                    let user = users.get_mut(uid).ok_or_else(|| {
                        Error::new(ErrorKind::Other, "Attempted to rename a nonexistent user")
                    })?;

                    if user.owner != addr {
                        return Err(Error::new(
                            ErrorKind::Other,
                            "Attempted to rename a non owned user",
                        ));
                    }

                    user.name = name.clone().into();

                    drop(users);

                    // Notify our group.
                    group
                        .send(
                            addr,
                            Update::Rename {
                                uid,
                                name: name.clone().into(),
                            },
                        )
                        .await;

                    log::debug!(
                        "{}: Rename - GID: {}, UID: {}, name: {:?}",
                        addr,
                        gid,
                        uid,
                        name
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
                    Update::Rename { uid, name } => ServerMessage::RenameUser {
                        gid,
                        uid,
                        name: name.into(),
                    },
                    Update::Message { uid, message } => ServerMessage::Message {
                        gid,
                        uid,
                        message: Cow::Owned(message),
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
    users: RwLock<Slab<User>>,
    subscribers: RwLock<HashMap<SocketAddr, Sender<Update>>>,
}

impl Group {
    async fn send(&self, ignore: SocketAddr, update: Update) {
        for (addr, sender) in &*self.subscribers.read().await {
            if *addr == ignore {
                continue;
            }

            let _ = sender.send(update.clone()).await;
        }
    }

    async fn cleanup_subscribers(&self, addr: SocketAddr) -> bool {
        self.subscribers.write().await.remove(&addr).is_some()
    }

    async fn cleanup_users(&self, addr: SocketAddr) {
        let mut users = self.users.write().await;
        for uid in 0..users.len() {
            if !users.contains(uid) {
                continue;
            }

            users.remove(uid);

            self.send(addr, Update::Leave { uid }).await;
        }
    }
}

pub struct User {
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
    Rename {
        uid: usize,
        name: String,
    },
    Message {
        uid: usize,
        message: Message<'static>,
    },
}
