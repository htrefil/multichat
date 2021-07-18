use crate::client::{Client, InitError};
use crate::net::{Addr, BasicConnector, Connector};

use multichat_proto::{Config, ServerInit, Version};
use std::convert::TryInto;
use std::error;
use std::fmt::{self, Debug, Display, Formatter};
use std::io::Error;
use std::num::NonZeroUsize;
use tokio::net::TcpStream;
#[cfg(feature = "tls")]
use tokio_native_tls::TlsConnector;

/// Configurable client builder.
#[derive(Clone, Copy, Debug)]
pub struct ClientBuilder<T> {
    connector: T,
    incoming_buffer: Result<Option<NonZeroUsize>, ()>,
    config: Config,
}

impl<T: Connector> ClientBuilder<T> {
    /// Sets the incoming messages buffer parameter.
    ///
    /// To achieve cancel safety, [`Client`] uses a separate task and a channel for receiving server messages.
    ///
    /// By default, only one message can be queued at a time to emulate a traditional read operation,
    /// however, if there is need to avoid stalling reading messages from the server, a higher number can be chosen.
    pub fn incoming_buffer(&mut self, value: impl TryInto<NonZeroUsize>) -> &mut Self {
        self.incoming_buffer = value.try_into().map(Some).map_err(drop);
        self
    }

    /// Sets Multichat protocol config.
    ///
    /// It is recommended to leave it unchanged unless you know what you're doing.
    pub fn config(&mut self, value: Config) -> &mut Self {
        self.config = value;
        self
    }

    /// Connects to a Multichat server identified by `addr`.
    pub async fn connect(
        &mut self,
        addr: impl Addr<'_>,
    ) -> Result<(ServerInit<'static>, Client<T::Stream>), ConnectError<T::Err>> {
        let incoming_buffer = self
            .incoming_buffer
            .map_err(|_| ConnectError::InvalidParameter)?
            .map(NonZeroUsize::get)
            .unwrap_or(1);

        let stream = TcpStream::connect(addr).await?;
        let stream = self
            .connector
            .connect(&addr.domain_name(), stream)
            .await
            .map_err(ConnectError::Tls)?;

        Client::from_io(incoming_buffer, stream, self.config)
            .await
            .map_err(From::from)
    }
}

impl ClientBuilder<BasicConnector> {
    /// Creates a basic unencrypted client builder.
    pub fn basic() -> Self {
        Self {
            connector: BasicConnector,
            incoming_buffer: Ok(None),
            config: Config::default(),
        }
    }
}

#[cfg(feature = "tls")]
impl ClientBuilder<TlsConnector> {
    /// Creates a TLS builder using the provided connector.
    pub fn tls(connector: TlsConnector) -> Self {
        Self {
            connector,
            incoming_buffer: Ok(None),
            config: Config::default(),
        }
    }
}

#[cfg(feature = "tls")]
impl ClientBuilder<Option<TlsConnector>> {
    /// Creates a builder that might be TLS-backed or not.
    ///
    /// Useful for disambiguating whether you want TLS at runtime.
    pub fn maybe_tls(connector: Option<TlsConnector>) -> Self {
        Self {
            connector,
            incoming_buffer: Ok(None),
            config: Config::default(),
        }
    }
}

/// Connection error.
#[derive(Debug)]
pub enum ConnectError<T> {
    Io(Error),
    Tls(T),
    ProtocolVersion(Version),
    InvalidParameter,
}

impl<T: Display> Display for ConnectError<T> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{}", err),
            Self::Tls(err) => write!(f, "{}", err),
            Self::ProtocolVersion(version) => {
                write!(f, "Incompatible server protocol version {}", version)
            }
            Self::InvalidParameter => write!(f, "Invalid parameter"),
        }
    }
}

impl<T> From<Error> for ConnectError<T> {
    fn from(err: Error) -> Self {
        Self::Io(err)
    }
}

impl<T> From<InitError> for ConnectError<T> {
    fn from(err: InitError) -> Self {
        match err {
            InitError::Io(err) => Self::Io(err),
            InitError::ProtocolVersion(version) => Self::ProtocolVersion(version),
        }
    }
}

impl<T: Display + Debug> error::Error for ConnectError<T> {}
