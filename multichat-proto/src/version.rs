use std::fmt::{self, Display, Formatter};
use std::io::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Protocol version, sent by server as the first message when a connection is established.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Version(pub u16);

impl Version {
    pub const CURRENT: Self = Self(1);

    /// Reads a version from a stream. It is recommended that the stream is buffered.
    ///
    /// This is provided as a separate function instead of leveraging [`read`](crate::wire::read)
    /// because the wire format is subject to change across protocol versions.
    ///
    /// The format of the version indicator, however, is not subject to change.
    pub async fn read(stream: &mut (impl AsyncRead + Unpin)) -> Result<Self, Error> {
        let value = stream.read_u16().await?;

        Ok(Self(value))
    }

    /// Writes self to a stream. It is recommended that the stream is buffered.
    ///
    /// Upon completion the stream is flushed, so there is no need to do it manually afterwards.
    ///
    /// This is provided as a separate function instead of leveraging [`write`](crate::wire::write)
    /// because the wire format is subject to change across protocol versions.
    ///
    /// The format of the version indicator, however, is not subject to change.
    pub async fn write(&self, stream: &mut (impl AsyncWrite + Unpin)) -> Result<(), Error> {
        stream.write_u16(self.0).await?;
        stream.flush().await?;

        Ok(())
    }
}

impl Display for Version {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn roundtrip_serialize(version: &Version) {
        let mut buffer = Vec::new();
        version.write(&mut buffer).await.unwrap();

        let mut buffer = buffer.as_slice();
        let deserialized = Version::read(&mut buffer).await.unwrap();

        // Check that there is no unused leftover data.
        assert_eq!(buffer.len(), 0);
        assert_eq!(version, &deserialized);
    }

    #[tokio::test]
    async fn roundtrip() {
        roundtrip_serialize(&Version(1)).await;
        roundtrip_serialize(&Version(0xFFFF)).await;
    }
}
