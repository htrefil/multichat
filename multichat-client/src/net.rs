use std::borrow::Cow;
use std::convert::Infallible;
use std::io::{Error, ErrorKind, IoSlice};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpStream, ToSocketAddrs};

#[cfg(feature = "tls")]
use tokio_rustls::{client::TlsStream, rustls::pki_types::ServerName, TlsConnector};

/// Trait implemented for all async IO streams suitable for a [`Client`](crate::client::Client).
///
/// Useful as a trait alias so that you don't have to write trait bounds like:
///
/// `async fn run<T: AsyncRead + AsyncWrite + Unpin + Send + 'static>(client: Client<T>) {}`
pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Stream for T {}

pub trait Connector {
    type Stream: Stream;
    type Err;

    async fn connect(
        &self,
        server_name: &str,
        stream: TcpStream,
    ) -> Result<Self::Stream, Self::Err>;
}

impl<T: Connector + Send + Unpin + Sync> Connector for Option<T> {
    type Stream = EitherStream<T::Stream>;
    type Err = T::Err;

    async fn connect(
        &self,
        server_name: &str,
        stream: TcpStream,
    ) -> Result<Self::Stream, Self::Err> {
        if let Some(connector) = self {
            return (*connector)
                .connect(server_name, stream)
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

unsafe impl<T: Send> Send for EitherStream<T> {}

#[cfg(feature = "tls")]
impl Connector for TlsConnector {
    type Stream = TlsStream<TcpStream>;
    type Err = Error;

    async fn connect(
        &self,
        server_name: &str,
        stream: TcpStream,
    ) -> Result<Self::Stream, Self::Err> {
        let server_name = ServerName::try_from(server_name)
            .map_err(|err| Error::new(ErrorKind::InvalidInput, err))?
            .to_owned();

        TlsConnector::connect(self, server_name, stream).await
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BasicConnector;

impl Connector for BasicConnector {
    type Stream = TcpStream;
    type Err = Infallible;

    async fn connect(
        &self,
        _server_name: &str,
        stream: TcpStream,
    ) -> Result<Self::Stream, Self::Err> {
        Ok(stream)
    }
}

/// Trait for efficient extraction of domain names from ToSocketAddr-like types.
pub trait Addr<'a>: ToSocketAddrs + Clone + Copy {
    fn server_name(self) -> Cow<'a, str>;
}

impl<'a> Addr<'a> for (&'a str, u16) {
    fn server_name(self) -> Cow<'a, str> {
        Cow::Borrowed(self.0)
    }
}

impl<'a> Addr<'a> for &'a str {
    fn server_name(self) -> Cow<'a, str> {
        Cow::Borrowed(self)
    }
}

impl<'a> Addr<'a> for &'a String {
    fn server_name(self) -> Cow<'a, str> {
        Cow::Borrowed(self)
    }
}

impl Addr<'static> for SocketAddr {
    fn server_name(self) -> Cow<'static, str> {
        Cow::Owned(self.ip().to_string())
    }
}

impl Addr<'static> for SocketAddrV4 {
    fn server_name(self) -> Cow<'static, str> {
        Cow::Owned(self.ip().to_string())
    }
}

impl Addr<'static> for SocketAddrV6 {
    fn server_name(self) -> Cow<'static, str> {
        Cow::Owned(self.ip().to_string())
    }
}

impl Addr<'static> for (IpAddr, u16) {
    fn server_name(self) -> Cow<'static, str> {
        Cow::Owned(self.0.to_string())
    }
}

impl Addr<'static> for (Ipv4Addr, u16) {
    fn server_name(self) -> Cow<'static, str> {
        Cow::Owned(self.0.to_string())
    }
}

impl Addr<'static> for (Ipv6Addr, u16) {
    fn server_name(self) -> Cow<'static, str> {
        Cow::Owned(self.0.to_string())
    }
}
