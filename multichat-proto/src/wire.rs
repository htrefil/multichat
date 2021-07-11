use rmp_serde::decode::Error as RmpError;
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
        mut stream: impl AsyncRead + Unpin,
    ) -> Result<T, Error> {
        let mut buffer = Vec::new();
        loop {
            let chunk_length = stream.read_u8().await? as usize;
            if buffer.len() + chunk_length > self.max_size {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "Data size exceeded limit",
                ));
            }

            let mut chunk_buffer = [0; 0xFF];
            stream.read_exact(&mut chunk_buffer[..chunk_length]).await?;

            buffer.extend_from_slice(&chunk_buffer);

            if chunk_length < 0xFF {
                break;
            }
        }

        rmp_serde::from_read(&*buffer).map_err(|err| match err {
            RmpError::InvalidMarkerRead(err) | RmpError::InvalidDataRead(err) => err,
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
        mut stream: impl AsyncWrite + Unpin,
        data: &impl Serialize,
    ) -> Result<(), Error> {
        let data =
            rmp_serde::to_vec(data).map_err(|err| Error::new(ErrorKind::InvalidInput, err))?;
        if data.len() > self.max_size {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "Data size exceeded limit",
            ));
        }

        for chunk in data.chunks(0xFF) {
            stream.write_u8(chunk.len() as u8).await?;
            stream.write_all(chunk).await?;
        }

        // Last chunk length 0xFF, so send a 0 length to indicate end of message.
        if data.len() % 0xFF == 0 {
            stream.write_u8(0).await?;
        }

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
pub async fn read<T: DeserializeOwned>(stream: impl AsyncRead + Unpin) -> Result<T, Error> {
    Config::default().read(stream).await
}

/// Writes a message to a stream with default [`Config`].
///
/// See [`Config::write`] for details.
pub async fn write(stream: impl AsyncWrite + Unpin, data: &impl Serialize) -> Result<(), Error> {
    Config::default().write(stream, data).await
}
