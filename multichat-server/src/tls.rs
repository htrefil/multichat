use rustls::ServerConfig;
use std::convert::Infallible;
use std::fmt::Display;
use std::future::Future;
use std::io;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;
use tokio::fs;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::server::TlsStream;
use tokio_rustls::{rustls, TlsAcceptor};

pub trait Acceptor: Clone + Send + Sync + 'static {
    type Stream: AsyncRead + AsyncWrite + Unpin + Send;
    type Error: Display;

    fn accept(
        &self,
        stream: TcpStream,
    ) -> impl Future<Output = Result<Self::Stream, Self::Error>> + Send;
}

impl Acceptor for TlsAcceptor {
    type Stream = TlsStream<TcpStream>;
    type Error = io::Error;

    async fn accept(&self, stream: TcpStream) -> Result<Self::Stream, Self::Error> {
        self.accept(stream).await
    }
}

#[derive(Clone)]
pub struct DefaultAcceptor;

impl Acceptor for DefaultAcceptor {
    type Stream = TcpStream;
    type Error = Infallible;

    async fn accept(&self, stream: TcpStream) -> Result<Self::Stream, Self::Error> {
        Ok(stream)
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Rustls(#[from] rustls::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("No private key provided")]
    NoKeys,
}

pub async fn configure(certificate: &Path, key: &Path) -> Result<TlsAcceptor, Error> {
    let certificate = fs::read(certificate).await?;
    let certificate = rustls_pemfile::certs(&mut &*certificate).collect::<Result<_, _>>()?;

    let key = fs::read(key).await?;
    let key = rustls_pemfile::private_key(&mut &*key)?.ok_or(Error::NoKeys)?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certificate, key)?;

    let config = ServerConfig::from(config);
    let config = Arc::new(config);

    Ok(TlsAcceptor::from(config))
}
