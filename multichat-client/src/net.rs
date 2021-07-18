use std::borrow::Cow;
use std::convert::Infallible;
use std::io::{Error, IoSlice};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpStream, ToSocketAddrs};

#[cfg(feature = "tls")]
use tokio_native_tls::native_tls::Error as TlsError;
#[cfg(feature = "tls")]
use tokio_native_tls::{TlsConnector, TlsStream};

/// Trait implemented for all async IO streams suitable for a [`Client`](crate::client::Client).
///
/// Useful as a trait alias so that you don't have to write trait bounds like:
///
/// `async fn run<T: AsyncRead + AsyncWrite + Unpin + Send + 'static>(client: Client<T>) {}`
pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Stream for T {}

#[async_trait::async_trait(?Send)]
pub trait Connector {
    type Stream: Stream;
    type Err;

    async fn connect(&self, domain: &str, stream: TcpStream) -> Result<Self::Stream, Self::Err>;
}

#[async_trait::async_trait(?Send)]
impl<T: Connector + Send + Unpin> Connector for Option<T> {
    type Stream = EitherStream<T::Stream>;
    type Err = T::Err;

    async fn connect(&self, domain: &str, stream: TcpStream) -> Result<Self::Stream, Self::Err> {
        if let Some(connector) = self {
            return connector
                .connect(domain, stream)
                .await
                .map(EitherStream::Right);
        }

        Ok(EitherStream::Left(stream))
    }
}

/// A stream containing either a raw TCP stream or a TLS stream.
pub enum EitherStream<T> {
    Left(TcpStream),
    Right(T),
}

impl<T: AsyncRead + Unpin> AsyncRead for EitherStream<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context,
        buffer: &mut ReadBuf,
    ) -> Poll<Result<(), Error>> {
        match &mut *self {
            Self::Left(stream) => Pin::new(stream).poll_read(context, buffer),
            Self::Right(stream) => Pin::new(stream).poll_read(context, buffer),
        }
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for EitherStream<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<Result<usize, Error>> {
        match &mut *self {
            Self::Left(stream) => Pin::new(stream).poll_write(context, buffer),
            Self::Right(stream) => Pin::new(stream).poll_write(context, buffer),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Result<(), Error>> {
        match &mut *self {
            Self::Left(stream) => Pin::new(stream).poll_flush(context),
            Self::Right(stream) => Pin::new(stream).poll_flush(context),
        }
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Result<(), Error>> {
        match &mut *self {
            Self::Left(stream) => Pin::new(stream).poll_shutdown(context),
            Self::Right(stream) => Pin::new(stream).poll_shutdown(context),
        }
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffers: &[IoSlice<'_>],
    ) -> Poll<Result<usize, Error>> {
        match &mut *self {
            Self::Left(stream) => Pin::new(stream).poll_write_vectored(context, buffers),
            Self::Right(stream) => Pin::new(stream).poll_write_vectored(context, buffers),
        }
    }

    fn is_write_vectored(&self) -> bool {
        match self {
            Self::Left(stream) => stream.is_write_vectored(),
            Self::Right(stream) => stream.is_write_vectored(),
        }
    }
}

#[cfg(feature = "tls")]
#[async_trait::async_trait(?Send)]
impl Connector for TlsConnector {
    type Stream = TlsStream<TcpStream>;
    type Err = TlsError;

    async fn connect(&self, domain: &str, stream: TcpStream) -> Result<Self::Stream, Self::Err> {
        TlsConnector::connect(self, domain, stream).await
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BasicConnector;

#[async_trait::async_trait(?Send)]
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
