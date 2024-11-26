use multichat_proto::{
    AccessToken, Attachment, AuthRequest, AuthResponse, ClientMessage, Config, ServerMessage,
    Version,
};
use std::borrow::Cow;
use std::collections::VecDeque;
use std::io::{Error, ErrorKind};
use tokio::io::{self, AsyncRead, AsyncWrite, BufReader, BufWriter, WriteHalf};
use tokio::sync::mpsc::{self, Receiver};
use tokio::time;

/// A client object representing a connection to a Multichat server.
pub struct Client<T> {
    stream_write: BufWriter<WriteHalf<T>>,
    receiver: Receiver<Result<ServerMessage<'static>, Error>>,
    // Updates queued while waiting for confirmations.
    updates: VecDeque<Update>,
    config: Config,
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

        // Spawn reading task.
        let (sender, receiver) = mpsc::channel(incoming_buffer);
        tokio::spawn(async move {
            let timeout = ping_interval + ping_timeout;

            loop {
                let result = tokio::select! {
                    result = config.read(&mut stream_read) => result,
                    _ = time::sleep(timeout) => Err(Error::new(ErrorKind::TimedOut, "Ping timeout")),
                };

                if result.is_err() | sender.send(result).await.is_err() {
                    return;
                }
            }
        });

        Ok(Self {
            stream_write,
            receiver,
            updates: VecDeque::new(),
            config,
        })
    }

    /// Joins a group and returns its ID.
    /// If the group does not exist, it will be created.
    ///
    /// This method is not cancel-safe.
    pub async fn join_group(&mut self, name: &str) -> Result<u32, Error> {
        self.config
            .write(
                &mut self.stream_write,
                &ClientMessage::JoinGroup { name: name.into() },
            )
            .await?;

        loop {
            let message = self.receiver.recv().await.ok_or(ErrorKind::BrokenPipe)??;
            match translate_message(message) {
                Ok(update) => self.updates.push_back(update),
                Err(Reply::ConfirmGroup(gid)) => return Ok(gid),
                Err(Reply::Ping) => continue,
                Err(_) => return Err(Error::new(ErrorKind::InvalidData, "Unexpected message")),
            }
        }
    }

    /// Creates a user and returns its ID.
    ///
    /// Specifying a nonexistent group is considered an error and will result in client disconnection by server.
    ///
    /// This method is not cancel-safe.
    pub async fn init_user(&mut self, gid: u32, name: &str) -> Result<u32, Error> {
        self.config
            .write(
                &mut self.stream_write,
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
                Err(Reply::Ping) => continue,
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
                &mut self.stream_write,
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
                &mut self.stream_write,
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
                &mut self.stream_write,
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
                &mut self.stream_write,
                &ClientMessage::DownloadAttachment { id },
            )
            .await?;

        loop {
            let message = self.receiver.recv().await.ok_or(ErrorKind::BrokenPipe)??;
            match translate_message(message) {
                Ok(update) => self.updates.push_back(update),
                Err(Reply::Attachment(data)) => return Ok(data),
                Err(Reply::Ping) => continue,
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
                &mut self.stream_write,
                &ClientMessage::IgnoreAttachment { id },
            )
            .await?;

        Ok(())
    }

    /// Reads an update from server.
    ///
    /// This method is cancel-safe, so it can be safely used inside, say, tokio::select!.
    pub async fn read_update(&mut self) -> Result<Update, Error> {
        if let Some(update) = self.updates.pop_front() {
            return Ok(update);
        }

        loop {
            let message = self.receiver.recv().await.ok_or(ErrorKind::BrokenPipe)??;
            match translate_message(message) {
                Ok(update) => return Ok(update),
                Err(Reply::Ping) => {
                    self.config
                        .write(&mut self.stream_write, &ClientMessage::Pong)
                        .await?;

                    continue;
                }
                Err(_) => return Err(Error::new(ErrorKind::InvalidData, "Unexpected message")),
            }
        }
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
    Ping,
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
        ServerMessage::Ping => Err(Reply::Ping),
    }
}
