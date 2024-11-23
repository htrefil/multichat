use bincode::{DefaultOptions, Options};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::{Error, ErrorKind};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Configuration for (de)coding data from the wire format.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    max_size: usize,
}

impl Config {
    /// Sets the max size of a wire frame to prevent DoS attacks by exhausting available memory.
    ///
    /// Default value is 65535 bytes.
    pub fn max_size(&mut self, max_size: usize) -> &mut Self {
        self.max_size = max_size;
        self
    }

    /// Read a message from a stream.
    ///
    /// It is highly recommended that the stream is internally buffered as this
    /// function can make a lot of small read calls.
    pub async fn read<T: DeserializeOwned>(
        &self,
        stream: &mut (impl AsyncRead + Unpin),
    ) -> Result<T, Error> {
        let length = stream.read_u32().await?;
        let length = length.try_into().map_err(|_| incoming_limit())?;

        if length > self.max_size {
            return Err(incoming_limit());
        }

        let mut buffer = vec![0; length];
        stream.read_exact(&mut buffer).await?;

        options().deserialize(&*buffer).map_err(|err| match *err {
            bincode::ErrorKind::Io(err) => err,
            err => Error::new(ErrorKind::InvalidData, err),
        })
    }

    /// Writes a message to a stream.
    ///
    /// Upon completion the stream is flushed, so there is no need to do it manually afterwards.
    ///
    /// It is highly recommended that the stream is internally buffered as this
    /// function can make a lot of small write calls.
    pub async fn write(
        &self,
        stream: &mut (impl AsyncWrite + Unpin),
        data: &impl Serialize,
    ) -> Result<(), Error> {
        let data = options().serialize(data).map_err(|err| match *err {
            bincode::ErrorKind::Io(err) => err,
            err => Error::new(ErrorKind::InvalidData, err),
        })?;

        if data.len() > self.max_size {
            return Err(outgoing_limit());
        }

        let length = data.len().try_into().map_err(|_| outgoing_limit())?;
        stream.write_u32(length).await?;
        stream.write_all(&data).await?;

        stream.flush().await?;

        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self { max_size: 65535 }
    }
}

/// Read a message from a stream with default [`Config`].
///
/// See [`Config::read`] for details.
pub async fn read<T: DeserializeOwned>(stream: &mut (impl AsyncRead + Unpin)) -> Result<T, Error> {
    Config::default().read(stream).await
}

/// Writes a message to a stream with default [`Config`].
///
/// See [`Config::write`] for details.
pub async fn write(
    stream: &mut (impl AsyncWrite + Unpin),
    data: &impl Serialize,
) -> Result<(), Error> {
    Config::default().write(stream, data).await
}

fn incoming_limit() -> Error {
    Error::new(ErrorKind::InvalidInput, "Incoming data size exceeded limit")
}

fn outgoing_limit() -> Error {
    Error::new(ErrorKind::InvalidInput, "Outgoing data size exceeded limit")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ClientMessage;
    use crate::server::{AuthResponse, ServerMessage};

    use std::collections::HashMap;
    use std::fmt::Debug;
    use std::time::Duration;

    async fn roundtrip_serialize<T: DeserializeOwned + Serialize + Debug + Eq>(item: &T) {
        let mut buffer = Vec::new();
        write(&mut buffer, item).await.unwrap();

        let mut buffer = buffer.as_slice();
        let deserialized: T = read(&mut buffer).await.unwrap();

        // Check that there is no unused leftover data.
        assert_eq!(buffer.len(), 0);
        assert_eq!(item, &deserialized);
    }

    #[tokio::test]
    async fn roundtrip() {
        roundtrip_serialize(&AuthResponse::Success {
            groups: {
                let mut groups = HashMap::new();
                groups.insert("first".into(), 1);
                groups.insert("second".into(), 2);

                groups
            },
            ping_interval: Duration::from_secs(10),
            ping_timeout: Duration::from_secs(5),
        })
        .await;

        roundtrip_serialize(&ServerMessage::ConfirmClient { uid: 123456 }).await;

        roundtrip_serialize(&ClientMessage::JoinUser {
            gid: 56789,
            name: "Borůvka".into(),
        })
        .await;

        roundtrip_serialize(&ClientMessage::SendMessage {
            gid: 58458,
            uid: 111213,
            message: "hello".into(),
            attachments: Vec::new().into(),
        })
        .await;
    }

    #[tokio::test]
    async fn length_write() {
        let config = *Config::default().max_size(10);

        assert_eq!(
            config
                .write(
                    &mut Vec::new(),
                    &ClientMessage::SendMessage {
                        gid: 0,
                        uid: 0,
                        message: "0123456789".into(),
                        attachments: Vec::new().into()
                    }
                )
                .await
                .is_err(),
            true
        );
    }

    #[tokio::test]
    async fn length_read() {
        let mut buffer = Vec::new();
        write(
            &mut buffer,
            &ClientMessage::SendMessage {
                gid: 0,
                uid: 0,
                message: "0123456789".into(),
                attachments: Vec::new().into(),
            },
        )
        .await
        .unwrap();

        let config = *Config::default().max_size(10);
        let result: Result<ClientMessage, _> = config.read(&mut buffer.as_slice()).await;

        assert_eq!(result.is_err(), true);
    }
}

fn options() -> impl Options {
    DefaultOptions::new()
}
