use crate::tls::Acceptor;

use multichat_proto::{Attachment, ClientMessage, ServerInit, ServerMessage};
use slab::Slab;
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tokio::io::{self, AsyncRead, AsyncWrite, BufReader, BufWriter};
use tokio::net::TcpListener;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::{self, Sender};
use tokio::sync::{mpsc, RwLock};
use tracing::Instrument;

pub async fn run(
    listen_addr: SocketAddr,
    acceptor: impl Acceptor,
    groups: impl IntoIterator<Item = String>,
    update_buffer: Option<NonZeroUsize>,
) -> Result<(), Error> {
    let listener = TcpListener::bind(&listen_addr).await?;

    tracing::info!("Listening on {}", listen_addr);

    let update_buffer = update_buffer.map(|num| num.get()).unwrap_or(256);
    let state = State {
        update_buffer,
        groups: groups
            .into_iter()
            .map(|name| Group {
                name,
                users: RwLock::new(Slab::new()),
                sender: broadcast::channel(update_buffer).0,
            })
            .collect(),
    };

    let state = Arc::new(state);
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

                match connection(stream, addr, &state).await {
                    Ok(_) => tracing::info!("Disconnected"),
                    Err(err) => tracing::error!("Disconnected: {}", err),
                }

                // Remove all users created by this connection which didn't leave on their own.
                for group in &state.groups {
                    group.cleanup_users(addr).await;
                }
            }
            .instrument(span),
        );
    }
}

async fn connection(
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
        .map(|(gid, group)| (group.name.as_str().into(), gid.try_into().unwrap()))
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

    let update_buffer = state.groups.len().min(1) * state.update_buffer;
    let (update_sender, mut update_receiver) = mpsc::channel(update_buffer);

    let mut group_handles = HashMap::new();
    let mut attachments = Slab::<Arc<Vec<u8>>>::new();

    loop {
        enum LocalUpdate {
            Client(ClientMessage<'static, 'static>),
            Group((u32, Update)),
        }

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
        };

        match update {
            LocalUpdate::Client(message) => match message {
                ClientMessage::JoinGroup { gid } => {
                    let group = gid
                        .try_into()
                        .ok()
                        .and_then(|gid: usize| state.groups.get(gid))
                        .ok_or_else(|| {
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
                                if result.is_err() | update_sender.send(result).await.is_err() {
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

                    for (uid, user) in &*group.users.read().await {
                        // XXX: Perhaps it would be wise to write the message
                        //      while the user list is not locked.
                        multichat_proto::write(
                            &mut stream_write,
                            &ServerMessage::InitUser {
                                gid,
                                uid: uid.try_into().unwrap(),
                                name: user.name.as_str().into(),
                            },
                        )
                        .await?;
                    }

                    tracing::debug!(%gid, "Join group");
                }
                ClientMessage::LeaveGroup { gid } => {
                    gid.try_into()
                        .ok()
                        .and_then(|gid: usize| state.groups.get(gid))
                        .ok_or_else(|| {
                            Error::new(ErrorKind::Other, "Attempted to leave a nonexistent group")
                        })?
                        .cleanup_users(addr)
                        .await;

                    group_handles
                        .remove(&gid)
                        .ok_or_else(|| {
                            Error::new(ErrorKind::Other, "Attempted to leave a non-joined group")
                        })?
                        .abort();

                    tracing::debug!(%gid, "Leave group");
                }
                ClientMessage::JoinUser { gid, name } => {
                    let group = gid
                        .try_into()
                        .ok()
                        .and_then(|gid: usize| state.groups.get(gid))
                        .ok_or_else(|| {
                            Error::new(
                                ErrorKind::Other,
                                "Attempted to join a user to a nonexistent group",
                            )
                        })?;

                    let uid = group
                        .users
                        .write()
                        .await
                        .insert(User {
                            name: name.clone().into(),
                            owner: addr,
                        })
                        .try_into()
                        .unwrap();

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

                    tracing::debug!(%gid, ?name, %uid, "Join user");
                }
                ClientMessage::LeaveUser { gid, uid } => {
                    let group = gid
                        .try_into()
                        .ok()
                        .and_then(|gid: usize| state.groups.get(gid))
                        .ok_or_else(|| {
                            Error::new(
                                ErrorKind::Other,
                                "Attempted to remove a user from a nonexistent group",
                            )
                        })?;

                    let mut users = group.users.write().await;

                    let err =
                        || Error::new(ErrorKind::Other, "Attempted to remove a nonexistent user");

                    let uid = uid.try_into().map_err(|_| err())?;
                    let user = users.get(uid).ok_or_else(err)?;

                    if user.owner != addr {
                        return Err(Error::new(
                            ErrorKind::Other,
                            "Attempted to remove a non owned user",
                        ));
                    }

                    users.remove(uid);

                    // Notify our group.
                    let _ = group.sender.send(Update::Leave {
                        uid: uid.try_into().unwrap(),
                    });

                    tracing::debug!(%gid, %uid, "Leave user");
                }
                ClientMessage::SendMessage {
                    gid,
                    uid,
                    message,
                    attachments,
                } => {
                    let group = gid
                        .try_into()
                        .ok()
                        .and_then(|gid: usize| state.groups.get(gid))
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

                    let users = group.users.read().await;

                    let uid = uid.try_into().map_err(|_| err())?;
                    let user = users.get(uid).ok_or_else(err)?;

                    if user.owner != addr {
                        return Err(Error::new(
                            ErrorKind::Other,
                            "Attempted to send a message as a non owned user",
                        ));
                    }

                    drop(users);

                    let message_clone = message.clone();

                    // Notify our group.
                    let _ = group.sender.send(Update::Message {
                        uid: uid.try_into().unwrap(),
                        message: message.into_owned().into(),
                        attachments: attachments
                            .into_owned() // Already owned.
                            .into_iter()
                            .map(Cow::into_owned) // Already owned.
                            .map(Arc::new)
                            .collect(),
                    });

                    tracing::debug!(%gid, %uid, message = ?message_clone, "Send message");
                }
                ClientMessage::RenameUser { gid, uid, name } => {
                    let group = gid
                        .try_into()
                        .ok()
                        .and_then(|gid: usize| state.groups.get(gid))
                        .ok_or_else(|| {
                            Error::new(
                                ErrorKind::Other,
                                "Attempted to rename a user from a nonexistent group",
                            )
                        })?;

                    let mut users = group.users.write().await;

                    let user = uid
                        .try_into()
                        .ok()
                        .and_then(|uid: usize| users.get_mut(uid))
                        .ok_or_else(|| {
                            Error::new(ErrorKind::Other, "Attempted to rename a nonexistent user")
                        })?;

                    if user.owner != addr {
                        return Err(Error::new(
                            ErrorKind::Other,
                            "Attempted to rename a non owned user",
                        ));
                    }

                    user.name = name.clone().into();

                    // Notify our group.
                    let _ = group.sender.send(Update::Rename {
                        uid,
                        name: name.clone().into(),
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

                    multichat_proto::write(
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
                    Update::Message {
                        uid,
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
                            uid,
                            message: message.into_owned().into(),
                            attachments: message_attachments,
                        }
                    }
                };

                multichat_proto::write(&mut stream_write, &message).await?;
            }
        }
    }
}

struct State {
    update_buffer: usize,
    groups: Vec<Group>,
}

struct Group {
    name: String,
    users: RwLock<Slab<User>>,
    sender: Sender<Update>,
}

impl Group {
    async fn cleanup_users(&self, addr: SocketAddr) {
        self.users.write().await.retain(|uid, user| {
            if user.owner == addr {
                let _ = self.sender.send(Update::Leave {
                    uid: uid.try_into().unwrap(),
                });

                return false;
            }

            true
        });
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
        uid: u32,
        // Name is included here due to the ABA problem.
        name: String,
    },
    Leave {
        uid: u32,
    },
    Rename {
        uid: u32,
        name: String,
    },
    Message {
        uid: u32,
        message: Cow<'static, str>,
        attachments: Vec<Arc<Vec<u8>>>, // Stored as Cows to avoid copying, but they're actually always owned.
    },
}
