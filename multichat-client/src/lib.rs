//! Crate containing code for buiding clients for the Multichat protocol - a small and efficient
//! protocol used for bridging textual communications from various sources over the internet.
//!
//! # Cargo features
//! - `tls` -- enables clients to connect to TLS encrypted servers with native_tls; enabled by default
//!
//! # Example echo client
//! ```rust
//! use multichat_client::{UpdateKind, ClientBuilder};
//! use std::error::Error;
//!
//! async fn echo() -> Result<(), Box<dyn Error>> {
//!     // This is a dummy access token for demonstration purposes.
//!     let access_token = "52f0395327987f07f805c3ac54fe38ac123303fcdb62a61fdfc9b8082195486c".parse()?;
//!     let (groups, mut client) = ClientBuilder::basic().connect("127.0.0.1:8585", access_token).await?;
//!
//!     // Find a group named "fun" and join it.
//!     let gid = *groups.get("fun").ok_or("Group not found")?;
//!     client.join_group(gid).await?;
//!
//!     // Create a new user in the group.
//!     let uid = client.join_user(gid, "example").await?;
//!
//!     loop {
//!         // Read what others say and repeat it.
//!         let update = client.read_update().await?;
//!         if update.uid != uid {
//!             if let UpdateKind::Message(message) = update.kind {
//!                 client
//!                     .send_message(gid, uid, &message.message, &[])
//!                     .await?;
//!             }
//!         }
//!     }
//! }
//! ```

#![allow(async_fn_in_trait)]

mod builder;
mod client;
mod net;

use std::convert::Infallible;

pub use builder::{ClientBuilder, ConnectError};
pub use client::{Client, Message, Update, UpdateKind};
pub use multichat_proto as proto;
pub use net::{Connector, EitherStream, Stream};

use tokio::net::TcpStream;

#[cfg(feature = "tls")]
use tokio_rustls::client::TlsStream;

/// Alias for a convenient way of naming the type of a TLS client.
#[cfg(feature = "tls")]
pub type TlsClient = Client<TlsStream<TcpStream>>;

#[cfg(feature = "tls")]
pub type MaybeTlsClient = Client<EitherStream<TlsStream<TcpStream>>>;

#[cfg(feature = "tls")]
pub type EitherTls = EitherStream<TlsStream<TcpStream>>;

/// Alias for a convenient way of naming the type of a basic client.
pub type BasicClient = Client<TcpStream>;
pub type BasicConnectError = ConnectError<Infallible>;
