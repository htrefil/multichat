use std::convert::Infallible;
use std::fmt::Display;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_native_tls::native_tls::Error as TlsError;
use tokio_native_tls::{TlsAcceptor, TlsStream};

#[async_trait::async_trait]
pub trait Acceptor: Clone + Send + Sync + 'static {
    type Stream: AsyncRead + AsyncWrite + Unpin + Send;
    type Error: Display;

    async fn accept(&self, stream: TcpStream) -> Result<Self::Stream, Self::Error>;
}

#[async_trait::async_trait]
impl Acceptor for TlsAcceptor {
    type Stream = TlsStream<TcpStream>;
    type Error = TlsError;

    async fn accept(&self, stream: TcpStream) -> Result<Self::Stream, Self::Error> {
        self.accept(stream).await
    }
}

#[derive(Clone)]
pub struct DefaultAcceptor;

#[async_trait::async_trait]
impl Acceptor for DefaultAcceptor {
    type Stream = TcpStream;
    type Error = Infallible;

    async fn accept(&self, stream: TcpStream) -> Result<Self::Stream, Self::Error> {
        Ok(stream)
    }
}
