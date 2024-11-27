use crate::tls::Acceptor;

use multichat_proto::{
    AccessToken, Attachment, AuthRequest, AuthResponse, ClientMessage, Config, ServerMessage,
    Version,
};
use slab::Slab;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::future;
use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{self, AsyncRead, AsyncWrite, BufReader, BufWriter};
use tokio::net::TcpListener;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::{self, Sender};
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use tokio::time;
use tracing::Instrument;

pub async fn run(
    listen_addr: SocketAddr,
    acceptor: impl Acceptor,
    update_buffer: Option<NonZeroUsize>,
    access_tokens: HashSet<AccessToken>,
    config: Config,
    ping_timeout: Option<Duration>,
    ping_interval: Option<Duration>,
) -> Result<(), Error> {
    let listener = TcpListener::bind(&listen_addr).await?;

    tracing::info!("Listening on {}", listen_addr);

    let update_buffer = update_buffer.map(|num| num.get()).unwrap_or(256);

    let state = Arc::new(State {
        update_buffer,
        groups: RwLock::new(Slab::new()),
        access_tokens,
        sender: broadcast::channel(update_buffer).0,
    });

    let ping_interval = ping_interval.unwrap_or(Duration::from_secs(30));
    let ping_timeout = ping_timeout.unwrap_or(Duration::from_secs(5));

    loop {
        let (stream, addr) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let state = state.clone();
        let span = tracing::info_span!("connection", %addr);

        tokio::spawn(
            async move {
                tracing::info!("Connected");

                let stream = match acceptor.accept(stream).await {
                    Ok(stream) => stream,
                    Err(err) => {
                        tracing::error!("TLS error: {}", err);
                        return;
                    }
                };

                let mut memberships = HashMap::new();

                let result = connection(
                    stream,
                    addr,
                    &state,
                    config,
                    ping_interval,
                    ping_timeout,
                    &mut memberships,
                )
                .await;

                match result {
                    Ok(_) => tracing::info!("Disconnected"),
                    Err(err) => tracing::error!("Disconnected: {}", err),
                }

                // Garbage collect users and groups.
                for (_, membership) in memberships {
                    membership.handle.abort();
                    let _ = membership.handle.await;
                }

                let mut groups = state.groups.write().await;
                groups.retain(|gid, group| {
                    group.cleanup_users(addr);

                    if group.sender.receiver_count() == 0 {
                        tracing::debug!(%gid, name = ?group.name, "Destroying group");

                        let _ = state.sender.send(GlobalUpdate {
                            gid: gid.try_into().unwrap(),
                            kind: GlobalUpdateKind::DestroyGroup,
                        });

                        return false;
                    }

                    true
                });
            }
            .instrument(span),
        );
    }
}

async fn connection(
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + 'static,
    addr: SocketAddr,
    state: &State,
    config: Config,
    ping_interval: Duration,
    ping_timeout: Duration,
    memberships: &mut HashMap<u32, Membership>,
) -> Result<(), Error> {
    let (stream_read, stream_write) = io::split(stream);

    let mut stream_read = BufReader::new(stream_read);
    let mut stream_write = BufWriter::new(stream_write);

    // Send our version.
    // Intentionally bypass config write because Version does not implement Serialize.
    Version::CURRENT.write(&mut stream_write).await?;

    let version = Version::read(&mut stream_read).await?;
    if version != Version::CURRENT {
        return Err(Error::new(ErrorKind::Other, "Incompatible version"));
    }

    // Read the client's auth request.
    let auth_request = config.read::<AuthRequest>(&mut stream_read).await?;
    if !state.access_tokens.contains(&auth_request.access_token) {
        config
            .write(&mut stream_write, &AuthResponse::Failed)
            .await?;

        return Err(Error::new(ErrorKind::Other, "Invalid access token"));
    }

    // Auth successful.
    config
        .write(
            &mut stream_write,
            &AuthResponse::Success {
                ping_interval,
                ping_timeout,
            },
        )
        .await?;

    // C2S.
    let (server_sender, mut server_receiver) = mpsc::channel(1);
    tokio::spawn(async move {
        loop {
            let result = config.read(&mut stream_read).await;
            if result.is_err() | server_sender.send(result).await.is_err() {
                break;
            }
        }
    });

    let groups = state
        .groups
        .read()
        .await
        .iter()
        .map(|(gid, group)| (gid, group.name.clone()))
        .collect::<Vec<_>>();

    // Send intitial updates.
    for (gid, name) in groups {
        config
            .write(
                &mut stream_write,
                &ServerMessage::InitGroup {
                    gid: gid.try_into().unwrap(),
                    name: name.into(),
                },
            )
            .await?;
    }

    let (update_sender, mut update_receiver) = mpsc::channel(state.update_buffer);

    let mut attachments = Slab::<Arc<Vec<u8>>>::new();
    let mut ping_interval = time::interval(ping_interval);
    let mut pong_interval = time::interval(ping_timeout);
    let mut waiting_pong = false;
    let mut receiver = state.sender.subscribe();

    loop {
        enum LocalUpdate {
            Client(ClientMessage<'static, 'static>),
            Global(GlobalUpdate),
            Group((u32, GroupUpdate)),
            Ping,
        }

        let pong = async {
            if waiting_pong {
                pong_interval.tick().await
            } else {
                future::pending().await
            }
        };

        // It's not possible for the unwraps to fail unless either task panics and at that
        // point we can just bring the whole thing down.
        let update = tokio::select! {
            result = server_receiver.recv() => LocalUpdate::Client(result.unwrap()?),
            result = update_receiver.recv() => {
                match result.unwrap() {
                    Ok(update) => LocalUpdate::Group(update),
                    Err(num) => return Err(Error::new(ErrorKind::Other, format!("Skipped {} group update(s)", num))),
                }
            }
            result = receiver.recv() => {
                match result {
                    Ok(update) => LocalUpdate::Global(update),
                    Err(num) => return Err(Error::new(ErrorKind::Other, format!("Skipped {} global update(s)", num))),
                }
            }
            _ = ping_interval.tick() => LocalUpdate::Ping,
            _ = pong => return Err(Error::new(ErrorKind::Other, "Pong timeout")),
        };

        match update {
            LocalUpdate::Client(message) => {
                ping_interval.reset();
                pong_interval.reset();

                waiting_pong = false;

                match message {
                    ClientMessage::JoinGroup { name } => {
                        let mut groups = state.groups.write().await;

                        let find = groups.iter_mut().find(|(_, group)| group.name == name);
                        let (gid, group, new) = match find {
                            Some((gid, group)) => (gid, group, false),
                            None => {
                                let (sender, _) = broadcast::channel(state.update_buffer);
                                let gid = groups.insert(Group {
                                    name: name.clone().into(),
                                    users: Slab::new(),
                                    sender,
                                });

                                (gid, groups.get_mut(gid).unwrap(), true)
                            }
                        };

                        let gid = gid.try_into().unwrap();
                        let sender = group.sender.clone();
                        let mut receiver = sender.subscribe();
                        let update_sender = update_sender.clone();

                        let handle = tokio::spawn(async move {
                            loop {
                                let result = match receiver.recv().await {
                                    Ok(update) => Ok((gid, update)),
                                    Err(RecvError::Lagged(num)) => Err(num),
                                    Err(RecvError::Closed) => return,
                                };

                                // The binary or is intentional, we want the result to be
                                // sent regardless of being an error.
                                if result.is_err() | update_sender.send(result).await.is_err() {
                                    return;
                                }
                            }
                        });

                        let membership = Membership {
                            handle,
                            newly_joined: true,
                        };

                        if memberships.insert(gid, membership).is_some() {
                            return Err(Error::new(
                                ErrorKind::Other,
                                "Attempted to join a group twice",
                            ));
                        }

                        if new {
                            let _ = state.sender.send(GlobalUpdate {
                                gid,
                                kind: GlobalUpdateKind::InitGroup {
                                    name: name.clone().into(),
                                },
                            });
                        } else {
                            let users = group
                                .users
                                .iter()
                                .map(|(uid, user)| (uid, user.name.clone()))
                                .collect::<Vec<_>>();

                            drop(groups);

                            for (uid, name) in users {
                                config
                                    .write(
                                        &mut stream_write,
                                        &ServerMessage::InitUser {
                                            gid,
                                            uid: uid.try_into().unwrap(),
                                            name: name.clone().into(),
                                        },
                                    )
                                    .await?;
                            }
                        }

                        config
                            .write(&mut stream_write, &ServerMessage::ConfirmGroup { gid })
                            .await?;

                        tracing::debug!(%gid, ?name, "Join group");
                    }
                    ClientMessage::LeaveGroup { gid } => {
                        let mut groups = state.groups.write().await;

                        let group = gid
                            .try_into()
                            .ok()
                            .and_then(|gid: usize| groups.get_mut(gid))
                            .ok_or_else(|| {
                                Error::new(
                                    ErrorKind::Other,
                                    "Attempted to leave a nonexistent group",
                                )
                            })?;

                        let handle = memberships
                            .remove(&gid)
                            .ok_or_else(|| {
                                Error::new(
                                    ErrorKind::Other,
                                    "Attempted to leave a non-joined group",
                                )
                            })?
                            .handle;

                        // Wait for the task to finish.
                        handle.abort();
                        let _ = handle.await;

                        group.cleanup_users(addr);

                        if group.sender.receiver_count() == 0 {
                            let group = groups.remove(gid.try_into().unwrap());
                            let _ = state.sender.send(GlobalUpdate {
                                gid,
                                kind: GlobalUpdateKind::DestroyGroup,
                            });

                            tracing::debug!(%gid, name = ?group.name, "Destroyed group");
                        }

                        tracing::debug!(%gid, "Leave group");
                    }
                    ClientMessage::InitUser { gid, name } => {
                        let mut groups = state.groups.write().await;

                        let group = gid
                            .try_into()
                            .ok()
                            .and_then(|gid: usize| groups.get_mut(gid))
                            .ok_or_else(|| {
                                Error::new(
                                    ErrorKind::Other,
                                    "Attempted to init a user in a nonexistent group",
                                )
                            })?;

                        let uid = group
                            .users
                            .insert(User {
                                name: name.clone().into(),
                                owner: addr,
                            })
                            .try_into()
                            .unwrap();

                        config
                            .write(&mut stream_write, &ServerMessage::ConfirmUser { uid })
                            .await?;

                        let _ = group.sender.send(GroupUpdate {
                            uid,
                            kind: GroupUpdateKind::InitUser {
                                name: name.clone().into(),
                            },
                        });

                        tracing::debug!(%gid, ?name, %uid, "Init user");
                    }
                    ClientMessage::DestroyUser { gid, uid } => {
                        let mut groups = state.groups.write().await;

                        let group = gid
                            .try_into()
                            .ok()
                            .and_then(|gid: usize| groups.get_mut(gid))
                            .ok_or_else(|| {
                                Error::new(
                                    ErrorKind::Other,
                                    "Attempted to destroy a user from a nonexistent group",
                                )
                            })?;

                        let err = || {
                            Error::new(ErrorKind::Other, "Attempted to destroy a nonexistent user")
                        };

                        let uid = uid.try_into().map_err(|_| err())?;
                        let user = group.users.get(uid).ok_or_else(err)?;

                        if user.owner != addr {
                            return Err(Error::new(
                                ErrorKind::Other,
                                "Attempted to destroy a non owned user",
                            ));
                        }

                        group.users.remove(uid);

                        let _ = group.sender.send(GroupUpdate {
                            uid: uid.try_into().unwrap(),
                            kind: GroupUpdateKind::DestroyUser,
                        });

                        tracing::debug!(%gid, %uid, "Leave user");
                    }
                    ClientMessage::SendMessage {
                        gid,
                        uid,
                        message,
                        attachments,
                    } => {
                        let groups = state.groups.read().await;

                        let group = gid
                            .try_into()
                            .ok()
                            .and_then(|gid: usize| groups.get(gid))
                            .ok_or_else(|| {
                                Error::new(
                                    ErrorKind::Other,
                                    "Attempted to send a message to a nonexistent group",
                                )
                            })?;

                        let err = || {
                            Error::new(
                                ErrorKind::Other,
                                "Attempted to send a message as a nonexistent user",
                            )
                        };

                        let uid = uid.try_into().map_err(|_| err())?;
                        let user = group.users.get(uid).ok_or_else(err)?;

                        if user.owner != addr {
                            return Err(Error::new(
                                ErrorKind::Other,
                                "Attempted to send a message as a non owned user",
                            ));
                        }

                        let message_clone = message.clone();

                        let _ = group.sender.send(GroupUpdate {
                            uid: uid.try_into().unwrap(),
                            kind: GroupUpdateKind::Message {
                                message: message.into_owned(),
                                attachments: attachments
                                    .into_owned() // Already owned.
                                    .into_iter()
                                    .map(Cow::into_owned) // Already owned.
                                    .map(Arc::new)
                                    .collect(),
                            },
                        });

                        tracing::debug!(%gid, %uid, msg = ?message_clone, "Send message");
                    }
                    ClientMessage::Rename { gid, uid, name } => {
                        let mut groups = state.groups.write().await;

                        let group = gid
                            .try_into()
                            .ok()
                            .and_then(|gid: usize| groups.get_mut(gid))
                            .ok_or_else(|| {
                                Error::new(
                                    ErrorKind::Other,
                                    "Attempted to rename a user from a nonexistent group",
                                )
                            })?;

                        let user = uid
                            .try_into()
                            .ok()
                            .and_then(|uid: usize| group.users.get_mut(uid))
                            .ok_or_else(|| {
                                Error::new(
                                    ErrorKind::Other,
                                    "Attempted to rename a nonexistent user",
                                )
                            })?;

                        if user.owner != addr {
                            return Err(Error::new(
                                ErrorKind::Other,
                                "Attempted to rename a non owned user",
                            ));
                        }

                        user.name = name.clone().into();

                        let _ = group.sender.send(GroupUpdate {
                            uid,
                            kind: GroupUpdateKind::Rename {
                                name: name.clone().into(),
                            },
                        });

                        tracing::debug!(%gid, %uid, ?name, "Rename");
                    }
                    ClientMessage::DownloadAttachment { id } => {
                        let attachment = id
                            .try_into()
                            .ok()
                            .and_then(|id: usize| attachments.try_remove(id))
                            .ok_or_else(|| {
                                Error::new(
                                    ErrorKind::Other,
                                    "Attempted to download a nonexistent attachment",
                                )
                            })?;

                        config
                            .write(
                                &mut stream_write,
                                &ServerMessage::Attachment {
                                    data: attachment.as_slice().into(),
                                },
                            )
                            .await?;

                        tracing::debug!(%id, "Download attachment");
                    }
                    ClientMessage::IgnoreAttachment { id } => {
                        let _ = id
                            .try_into()
                            .ok()
                            .and_then(|id: usize| attachments.try_remove(id))
                            .ok_or_else(|| {
                                Error::new(
                                    ErrorKind::Other,
                                    "Attempted to ignore a nonexistent attachment",
                                )
                            })?;

                        tracing::debug!(%id, "Ignore attachment");
                    }
                    ClientMessage::Pong => tracing::trace!("Pong"),
                    ClientMessage::Shutdown => {
                        tracing::debug!("Shutdown");
                        return Ok(());
                    }
                }
            }
            LocalUpdate::Global(update) => {
                ping_interval.reset();

                let init = matches!(update.kind, GlobalUpdateKind::InitGroup { .. });
                let message = match update.kind {
                    GlobalUpdateKind::InitGroup { name } => ServerMessage::InitGroup {
                        name: name.into(),
                        gid: update.gid,
                    },
                    GlobalUpdateKind::DestroyGroup => {
                        ServerMessage::DestroyGroup { gid: update.gid }
                    }
                };

                config.write(&mut stream_write, &message).await?;

                if !init {
                    continue;
                }

                let membership = match memberships.get_mut(&update.gid) {
                    Some(membership) => membership,
                    None => continue,
                };

                if !membership.newly_joined {
                    continue;
                }

                membership.newly_joined = false;

                let groups = state.groups.read().await;
                let users = groups[update.gid.try_into().unwrap()]
                    .users
                    .iter()
                    .map(|(uid, user)| (uid, user.name.clone()))
                    .collect::<Vec<_>>();

                drop(groups);

                for (uid, name) in users {
                    config
                        .write(
                            &mut stream_write,
                            &ServerMessage::InitUser {
                                gid: update.gid,
                                uid: uid.try_into().unwrap(),
                                name: name.clone().into(),
                            },
                        )
                        .await?;
                }
            }
            LocalUpdate::Group((gid, update)) => {
                ping_interval.reset();

                let message = match update.kind {
                    GroupUpdateKind::InitUser { name } => ServerMessage::InitUser {
                        gid,
                        uid: update.uid,
                        name: name.into(),
                    },
                    GroupUpdateKind::DestroyUser => ServerMessage::DestroyUser {
                        gid,
                        uid: update.uid,
                    },
                    GroupUpdateKind::Rename { name } => ServerMessage::Rename {
                        gid,
                        uid: update.uid,
                        name: name.into(),
                    },
                    GroupUpdateKind::Message {
                        message,
                        attachments: update_attachments,
                    } => {
                        let mut message_attachments = Vec::new();
                        for attachment in update_attachments {
                            let len = attachment.len();
                            let id = attachments.insert(attachment);

                            message_attachments.push(Attachment {
                                id: id.try_into().unwrap(),
                                size: len.try_into().unwrap(),
                            });
                        }

                        ServerMessage::Message {
                            gid,
                            uid: update.uid,
                            message: message.into(),
                            attachments: message_attachments,
                        }
                    }
                };

                config.write(&mut stream_write, &message).await?;
            }
            LocalUpdate::Ping => {
                tracing::trace!("Sending ping");

                config
                    .write(&mut stream_write, &ServerMessage::Ping)
                    .await?;

                ping_interval.reset();
                pong_interval.reset();

                waiting_pong = true;
            }
        }
    }
}

struct State {
    update_buffer: usize,
    access_tokens: HashSet<AccessToken>,
    groups: RwLock<Slab<Group>>,
    sender: Sender<GlobalUpdate>,
}

struct Group {
    name: String,
    users: Slab<User>,
    sender: Sender<GroupUpdate>,
}

impl Group {
    fn cleanup_users(&mut self, addr: SocketAddr) {
        self.users.retain(|uid, user| {
            if user.owner == addr {
                let _ = self.sender.send(GroupUpdate {
                    uid: uid.try_into().unwrap(),
                    kind: GroupUpdateKind::DestroyUser,
                });

                return false;
            }

            true
        });
    }
}

struct User {
    name: String,
    // Owning connection.
    owner: SocketAddr,
}

struct Membership {
    handle: JoinHandle<()>,
    newly_joined: bool,
}

#[derive(Clone)]
struct GlobalUpdate {
    gid: u32,
    kind: GlobalUpdateKind,
}

#[derive(Clone)]
enum GlobalUpdateKind {
    InitGroup {
        // Name is included here due to the ABA problem.
        name: String,
    },
    DestroyGroup,
}

#[derive(Clone)]
struct GroupUpdate {
    uid: u32,
    kind: GroupUpdateKind,
}

#[derive(Clone)]
enum GroupUpdateKind {
    InitUser {
        // Name is included here due to the ABA problem.
        name: String,
    },
    DestroyUser,
    Message {
        message: String,
        attachments: Vec<Arc<Vec<u8>>>,
    },
    Rename {
        name: String,
    },
}
