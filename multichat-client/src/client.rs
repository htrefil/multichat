use multichat_proto::{ClientMessage, Config, Message, ServerInit, ServerMessage, Version};
use std::borrow::Cow;
use std::collections::VecDeque;
use std::io::{Error, ErrorKind};
use tokio::io::{self, AsyncRead, AsyncWrite, BufReader, BufWriter, WriteHalf};
use tokio::sync::mpsc::{self, Receiver};

/// A client object representing a connection to a Multichat server.
pub struct Client<T> {
    stream_write: BufWriter<WriteHalf<T>>,
    receiver: Receiver<Result<ServerMessage<'static>, Error>>,
    // Updates queued while waiting for user join confirmation.
    updates: VecDeque<Update>,
    config: Config,
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Client<T> {
    pub(crate) async fn from_io(
        incoming_buffer: usize,
        stream: T,
        config: Config,
    ) -> Result<(ServerInit<'static>, Self), InitError> {
        let (stream_read, stream_write) = io::split(stream);

        let mut stream_read = BufReader::new(stream_read);
        let stream_write = BufWriter::new(stream_write);

        // Read server version.
        let version = Version::read(&mut stream_read).await?;
        if !multichat_proto::VERSION.is_compatible(version) {
            return Err(InitError::ProtocolVersion(version));
        }

        // Read initial server data.
        let init = config.read(&mut stream_read).await?;

        // Spawn reading task.
        let (sender, receiver) = mpsc::channel(incoming_buffer);
        tokio::spawn(async move {
            loop {
                let result = config.read(&mut stream_read).await;
                if result.is_err() | sender.send(result).await.is_err() {
                    return;
                }
            }
        });

        Ok((
            init,
            Self {
                stream_write,
                receiver,
                updates: VecDeque::new(),
                config,
            },
        ))
    }

    /// Joins a group.
    ///
    /// Joining a nonexistent group is considered an error and will result in client disconnection by server.
    ///
    /// This method is not cancel-safe.
    pub async fn join_group(&mut self, gid: usize) -> Result<(), Error> {
        self.config
            .write(&mut self.stream_write, &ClientMessage::JoinGroup { gid })
            .await?;

        Ok(())
    }

    /// Creates an user and returns its UID.
    ///
    /// Specifying a nonexistent group is considered an error and will result in client disconnection by server.
    ///
    /// This method is not cancel-safe.
    pub async fn join_user(&mut self, gid: usize, name: &str) -> Result<usize, Error> {
        self.config
            .write(
                &mut self.stream_write,
                &ClientMessage::JoinUser {
                    gid,
                    name: name.into(),
                },
            )
            .await?;

        loop {
            let message = self.receiver.recv().await.ok_or(ErrorKind::BrokenPipe)??;
            match translate_message(message) {
                Ok(update) => self.updates.push_back(update),
                Err(uid) => return Ok(uid),
            }
        }
    }

    /// Leaves an user.
    ///
    /// Specifying a nonexistent group or user ID is considered an error and will result in client disconnection by server.
    pub async fn leave_user(&mut self, gid: usize, uid: usize) -> Result<(), Error> {
        self.config
            .write(
                &mut self.stream_write,
                &ClientMessage::LeaveUser { gid, uid },
            )
            .await?;

        Ok(())
    }

    /// Renames an user.
    ///
    /// Specifying a nonexistent group or user ID is considered an error and will result in client disconnection by server.
    pub async fn rename_user(&mut self, gid: usize, uid: usize, name: &str) -> Result<(), Error> {
        self.config
            .write(
                &mut self.stream_write,
                &ClientMessage::RenameUser {
                    gid,
                    uid,
                    name: name.into(),
                },
            )
            .await?;

        Ok(())
    }

    /// Sends a message to a group as an user.
    ///
    /// Specifying a nonexistent group or user ID is considered an error and will result in client disconnection by server.
    pub async fn send_message(
        &mut self,
        gid: usize,
        uid: usize,
        message: &Message<'_>,
    ) -> Result<(), Error> {
        self.config
            .write(
                &mut self.stream_write,
                &ClientMessage::SendMessage {
                    gid,
                    uid,
                    message: Cow::Borrowed(message),
                },
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

        let message = self.receiver.recv().await.ok_or(ErrorKind::BrokenPipe)??;
        translate_message(message)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "Unexpected message"))
    }
}

/// Update from a server.
#[derive(Clone, Debug)]
pub struct Update {
    /// The group ID that this update concerns.
    pub gid: usize,
    /// The user ID that this update concerns.
    pub uid: usize,
    /// Type of the update.
    pub kind: UpdateKind,
}

#[derive(Clone, Debug)]
pub enum UpdateKind {
    /// An user joined the group.
    Join(String),
    /// An user left the group.
    Leave,
    /// An user was renamed.
    Rename(String),
    /// An user sent a message.
    Message(Message<'static>),
}

pub(crate) enum InitError {
    Io(Error),
    ProtocolVersion(Version),
}

impl From<Error> for InitError {
    fn from(err: Error) -> Self {
        Self::Io(err)
    }
}

fn translate_message(message: ServerMessage<'static>) -> Result<Update, usize> {
    let update = match message {
        ServerMessage::InitUser { gid, uid, name } => Update {
            gid,
            uid,
            kind: UpdateKind::Join(name.into_owned()),
        },
        ServerMessage::LeaveUser { gid, uid } => Update {
            gid,
            uid,
            kind: UpdateKind::Leave,
        },
        ServerMessage::RenameUser { gid, uid, name } => Update {
            gid,
            uid,
            kind: UpdateKind::Rename(name.into_owned()),
        },
        ServerMessage::Message { gid, uid, message } => Update {
            gid,
            uid,
            kind: UpdateKind::Message(message.into_owned()),
        },
        ServerMessage::ConfirmClient { uid } => return Err(uid),
    };

    Ok(update)
}
