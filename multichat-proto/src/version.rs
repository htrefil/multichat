use std::fmt::{self, Display, Formatter};
use std::io::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Current protocol version - 1.0.
pub const VERSION: Version = Version { major: 1, minor: 0 };

/// Protocol version, sent by server as the first message when a connection is established.
#[derive(Debug, Clone, Copy)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
}

impl Version {
    /// Checks if `self` is compatible with the other version.
    ///
    /// Versions are compatible only if major versions are equal.
    pub fn is_compatible(&self, other: Version) -> bool {
        self.major == other.major
    }

    /// Reads a version from a stream. It is recommended that the stream is buffered.
    ///
    /// This is provided as a separate function instead of leveraging [`read`](crate::wire::read)
    /// because the wire format is subject to change across protocol versions.
    ///
    /// The format of the version indicator, however, is not subject to change.
    pub async fn read(mut stream: impl AsyncRead + Unpin) -> Result<Self, Error> {
        Ok(Self {
            major: stream.read_u16().await?,
            minor: stream.read_u16().await?,
        })
    }

    /// Writes self to a stream. It is recommended that the stream is buffered.
    ///
    /// Upon completion the stream is flushed, so there is no need to do it manually afterwards.
    ///
    /// This is provided as a separate function instead of leveraging [`write`](crate::wire::write)
    /// because the wire format is subject to change across protocol versions.
    ///
    /// The format of the version indicator, however, is not subject to change.
    pub async fn write(&self, mut stream: impl AsyncWrite + Unpin) -> Result<(), Error> {
        stream.write_u16(self.major).await?;
        stream.write_u16(self.minor).await?;
        stream.flush().await?;

        Ok(())
    }
}

impl Display for Version {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}
