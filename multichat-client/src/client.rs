use multichat_proto::{
    AccessToken, Attachment, AuthRequest, AuthResponse, ClientMessage, Config, ServerMessage,
    Version,
};
use std::borrow::Cow;
use std::collections::VecDeque;
use std::io::{Error, ErrorKind};
use std::sync::Arc;
use tokio::io::{self, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter, WriteHalf};
use tokio::sync::mpsc::{self, Receiver};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time;

/// A client object representing a connection to a Multichat server.
pub struct Client<T> {
    stream_write: Arc<Mutex<BufWriter<WriteHalf<T>>>>,
    receiver: Receiver<Result<ServerMessage<'static>, Error>>,
    // Updates queued while waiting for confirmations.
    updates: VecDeque<Update>,
    config: Config,
    handle: JoinHandle<()>,
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Client<T> {
    pub(crate) async fn from_io(
        incoming_buffer: usize,
        stream: T,
        config: Config,
        access_token: AccessToken,
    ) -> Result<Self, InitError> {
        let (stream_read, stream_write) = io::split(stream);

        let mut stream_read = BufReader::new(stream_read);
        let mut stream_write = BufWriter::new(stream_write);

        // Write client version.
        Version::CURRENT.write(&mut stream_write).await?;

        // Read server version.
        let version = Version::read(&mut stream_read).await?;
        if version != Version::CURRENT {
            return Err(InitError::ProtocolVersion(version));
        }

        // Write auth request.
        config
            .write(&mut stream_write, &AuthRequest { access_token })
            .await?;

        // Read auth response.
        let (ping_interval, ping_timeout) = match config.read(&mut stream_read).await? {
            AuthResponse::Success {
                ping_interval,
                ping_timeout,
            } => (ping_interval, ping_timeout),
            AuthResponse::Failed => return Err(InitError::Auth),
        };

        let stream_write = Arc::new(Mutex::new(stream_write));

        // Spawn reading task.
        let (sender, receiver) = mpsc::channel(incoming_buffer);
        let handle = tokio::spawn({
            let stream_write = stream_write.clone();

            async move {
                let timeout = ping_interval + ping_timeout;

                loop {
                    let result = tokio::select! {
                        result = config.read(&mut stream_read) => result,
                        _ = sender.closed() => break,
                        _ = time::sleep(timeout) => Err(Error::new(ErrorKind::TimedOut, "Ping timeout")),
                    };

                    match result {
                        Ok(ServerMessage::Ping) => {
                            let mut stream_write = stream_write.lock().await;

                            let result =
                                config.write(&mut *stream_write, &ClientMessage::Pong).await;
                            let err = match result {
                                Ok(()) => continue, // Ok, pong sent.
                                Err(err) => err,
                            };

                            drop(stream_write);

                            let _ = sender.send(Err(err)).await;
                            return;
                        }
                        Ok(message) => {
                            if sender.send(Ok(message)).await.is_err() {
                                return;
                            }
                        }
                        Err(err) => {
                            let _ = sender.send(Err(err)).await;
                            return;
                        }
                    }
                }
            }
        });

        Ok(Self {
            stream_write,
            receiver,
            updates: VecDeque::new(),
            config,
            handle,
        })
    }

    /// Joins a group and returns its ID.
    /// If the group does not exist, it will be created.
    pub async fn join_group(&mut self, name: &str) -> Result<u32, Error> {
        self.config
            .write(
                &mut *self.stream_write.lock().await,
                &ClientMessage::JoinGroup { name: name.into() },
            )
            .await?;

        loop {
            let message = self.receiver.recv().await.ok_or(ErrorKind::BrokenPipe)??;
            match translate_message(message) {
                Ok(update) => self.updates.push_back(update),
                Err(Reply::ConfirmGroup(gid)) => return Ok(gid),
                Err(_) => return Err(Error::new(ErrorKind::InvalidData, "Unexpected message")),
            }
        }
    }

    /// Creates a user and returns its ID.
    ///
    /// Specifying a nonexistent group is considered an error and will result in client disconnection by server.
    pub async fn init_user(&mut self, gid: u32, name: &str) -> Result<u32, Error> {
        self.config
            .write(
                &mut *self.stream_write.lock().await,
                &ClientMessage::InitUser {
                    gid,
                    name: name.into(),
                },
            )
            .await?;

        loop {
            let message = self.receiver.recv().await.ok_or(ErrorKind::BrokenPipe)??;
            match translate_message(message) {
                Ok(update) => self.updates.push_back(update),
                Err(Reply::ConfirmClient(uid)) => return Ok(uid),
                Err(_) => return Err(Error::new(ErrorKind::InvalidData, "Unexpected message")),
            }
        }
    }

    /// Destroys a user.
    ///
    /// Specifying a nonexistent group or user ID is considered an error and will result in client disconnection by server.
    pub async fn destroy_user(&mut self, gid: u32, uid: u32) -> Result<(), Error> {
        self.config
            .write(
                &mut *self.stream_write.lock().await,
                &ClientMessage::DestroyUser { gid, uid },
            )
            .await?;

        Ok(())
    }

    /// Renames a user.
    ///
    /// Specifying a nonexistent group or user ID is considered an error and will result in client disconnection by server.
    pub async fn rename_user(&mut self, gid: u32, uid: u32, name: &str) -> Result<(), Error> {
        self.config
            .write(
                &mut *self.stream_write.lock().await,
                &ClientMessage::Rename {
                    gid,
                    uid,
                    name: name.into(),
                },
            )
            .await?;

        Ok(())
    }

    /// Sends a message to a group as a user.
    ///
    /// Specifying a nonexistent group or user ID is considered an error and will result in client disconnection by server.
    pub async fn send_message(
        &mut self,
        gid: u32,
        uid: u32,
        message: &str,
        attachments: &[Cow<'_, [u8]>],
    ) -> Result<(), Error> {
        self.config
            .write(
                &mut *self.stream_write.lock().await,
                &ClientMessage::SendMessage {
                    gid,
                    uid,
                    message: message.into(),
                    attachments: attachments.into(),
                },
            )
            .await?;

        Ok(())
    }

    /// Downloads an attachment.
    ///
    /// Specifying a nonexistent attachment ID is considered an error and will result in client disconnection by server.
    pub async fn download_attachment(&mut self, id: u32) -> Result<Vec<u8>, Error> {
        self.config
            .write(
                &mut *self.stream_write.lock().await,
                &ClientMessage::DownloadAttachment { id },
            )
            .await?;

        loop {
            let message = self.receiver.recv().await.ok_or(ErrorKind::BrokenPipe)??;
            match translate_message(message) {
                Ok(update) => self.updates.push_back(update),
                Err(Reply::Attachment(data)) => return Ok(data),
                Err(_) => return Err(Error::new(ErrorKind::InvalidData, "Unexpected message")),
            }
        }
    }

    /// Ignores an attachment.
    ///
    /// Specifying a nonexistent attachment ID is considered an error and will result in client disconnection by server.
    pub async fn ignore_attachment(&mut self, id: u32) -> Result<(), Error> {
        self.config
            .write(
                &mut *self.stream_write.lock().await,
                &ClientMessage::IgnoreAttachment { id },
            )
            .await?;

        Ok(())
    }

    /// Reads an update from server.
    /// This method should be called frequently in a loop, otherwise the server may disconnect the client.
    ///
    /// This method is cancel-safe.
    pub async fn read_update(&mut self) -> Result<Update, Error> {
        if let Some(update) = self.updates.pop_front() {
            return Ok(update);
        }

        loop {
            let message = self.receiver.recv().await.ok_or(ErrorKind::BrokenPipe)??;
            match translate_message(message) {
                Ok(update) => return Ok(update),
                Err(_) => return Err(Error::new(ErrorKind::InvalidData, "Unexpected message")),
            }
        }
    }

    /// Cleanly shuts down the client.
    ///
    /// This is not strictly necessary but is considered good practice because it will avoid making false error logs on the server side.
    pub async fn shutdown(mut self) -> Result<(), Error> {
        self.receiver.close();
        self.handle.await.unwrap();

        let mut stream_write = self.stream_write.lock().await;

        self.config
            .write(&mut *stream_write, &ClientMessage::Shutdown)
            .await?;

        stream_write.shutdown().await?;

        Ok(())
    }
}

/// Update from a server.
#[derive(Clone, Debug)]
pub struct Update {
    /// The group ID that this update concerns.
    pub gid: u32,
    /// Type of the update.
    pub kind: UpdateKind,
}

#[derive(Clone, Debug)]
pub enum UpdateKind {
    /// A group was created.
    InitGroup { name: String },
    /// A group was destroyed.
    DestroyGroup,
    /// A user joined the group.
    InitUser { uid: u32, name: String },
    /// A user left the group.
    DestroyUser { uid: u32 },
    /// A user was renamed.
    Rename { uid: u32, name: String },
    /// A user sent a message.
    Message { uid: u32, message: Message },
}

/// A message from a user.
#[derive(Clone, Debug)]
pub struct Message {
    /// The message text.
    pub text: String,
    /// The message attachments.
    /// Each attachment must be either [downloaded](Client::download_attachment) or [ignored](Client::ignore_attachment)
    /// as soon as possible since receiving the message.
    pub attachments: Vec<Attachment>,
}

pub(crate) enum InitError {
    Io(Error),
    ProtocolVersion(Version),
    Auth,
}

impl From<Error> for InitError {
    fn from(err: Error) -> Self {
        Self::Io(err)
    }
}

enum Reply {
    Attachment(Vec<u8>),
    ConfirmClient(u32),
    ConfirmGroup(u32),
}

fn translate_message(message: ServerMessage<'static>) -> Result<Update, Reply> {
    match message {
        ServerMessage::InitGroup { name, gid } => Ok(Update {
            gid,
            kind: UpdateKind::InitGroup {
                name: name.into_owned(),
            },
        }),
        ServerMessage::DestroyGroup { gid } => Ok(Update {
            gid,
            kind: UpdateKind::DestroyGroup,
        }),
        ServerMessage::InitUser { gid, uid, name } => Ok(Update {
            gid,
            kind: UpdateKind::InitUser {
                uid,
                name: name.into_owned(),
            },
        }),
        ServerMessage::DestroyUser { gid, uid } => Ok(Update {
            gid,
            kind: UpdateKind::DestroyUser { uid },
        }),
        ServerMessage::Rename { gid, uid, name } => Ok(Update {
            gid,
            kind: UpdateKind::Rename {
                uid,
                name: name.into_owned(),
            },
        }),
        ServerMessage::Message {
            gid,
            uid,
            message,
            attachments,
        } => Ok(Update {
            gid,
            kind: UpdateKind::Message {
                uid,
                message: Message {
                    text: message.into_owned(),
                    attachments,
                },
            },
        }),
        ServerMessage::ConfirmUser { uid } => Err(Reply::ConfirmClient(uid)),
        ServerMessage::ConfirmGroup { gid } => Err(Reply::ConfirmGroup(gid)),
        ServerMessage::Attachment { data } => Err(Reply::Attachment(data.into_owned())),
        ServerMessage::Ping => unreachable!(), // Filtered out by the reading task.
    }
}
