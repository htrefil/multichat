use std::borrow::Cow;
use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpStream, ToSocketAddrs};
#[cfg(feature = "tls")]
use tokio_native_tls::native_tls::Error as TlsError;
#[cfg(feature = "tls")]
use tokio_native_tls::{TlsConnector, TlsStream};

#[async_trait::async_trait]
pub trait Connector {
    type Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static;
    type Err;

    async fn connect(&self, domain: &str, stream: TcpStream) -> Result<Self::Stream, Self::Err>;
}

#[cfg(feature = "tls")]
#[async_trait::async_trait]
impl Connector for TlsConnector {
    type Stream = TlsStream<TcpStream>;
    type Err = TlsError;

    async fn connect(&self, domain: &str, stream: TcpStream) -> Result<Self::Stream, Self::Err> {
        TlsConnector::connect(self, domain, stream).await
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BasicConnector;

#[async_trait::async_trait]
impl Connector for BasicConnector {
    type Stream = TcpStream;
    type Err = Infallible;

    async fn connect(&self, _domain: &str, stream: TcpStream) -> Result<Self::Stream, Self::Err> {
        Ok(stream)
    }
}

/// Trait for efficient extraction of domain names from ToSocketAddr-like types.
pub trait Addr<'a>: ToSocketAddrs + Clone + Copy {
    fn domain_name(self) -> Cow<'a, str>;
}

impl<'a> Addr<'a> for (&'a str, u16) {
    fn domain_name(self) -> Cow<'a, str> {
        Cow::Borrowed(self.0)
    }
}

impl<'a> Addr<'a> for &'a str {
    fn domain_name(self) -> Cow<'a, str> {
        Cow::Borrowed(self)
    }
}

impl<'a> Addr<'a> for &'a String {
    fn domain_name(self) -> Cow<'a, str> {
        Cow::Borrowed(self)
    }
}

impl Addr<'static> for SocketAddr {
    fn domain_name(self) -> Cow<'static, str> {
        Cow::Owned(self.ip().to_string())
    }
}

impl Addr<'static> for SocketAddrV4 {
    fn domain_name(self) -> Cow<'static, str> {
        Cow::Owned(self.ip().to_string())
    }
}

impl Addr<'static> for SocketAddrV6 {
    fn domain_name(self) -> Cow<'static, str> {
        Cow::Owned(self.ip().to_string())
    }
}

impl Addr<'static> for (IpAddr, u16) {
    fn domain_name(self) -> Cow<'static, str> {
        Cow::Owned(self.0.to_string())
    }
}

impl Addr<'static> for (Ipv4Addr, u16) {
    fn domain_name(self) -> Cow<'static, str> {
        Cow::Owned(self.0.to_string())
    }
}

impl Addr<'static> for (Ipv6Addr, u16) {
    fn domain_name(self) -> Cow<'static, str> {
        Cow::Owned(self.0.to_string())
    }
}
