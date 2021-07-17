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
//!     let (init, mut client) = ClientBuilder::basic().connect("127.0.0.1:8585").await?;
//!
//!     // Find a group named "fun" and join it.
//!     let gid = *init.groups.get("fun").ok_or("Group not found")?;
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
//!                     .send_message(gid, uid, &message)
//!                     .await?;
//!             }
//!         }
//!     }
//! }
//! ```

mod builder;
mod client;
mod tls;

pub use builder::{ClientBuilder, ConnectError};
pub use client::{Client, Update, UpdateKind};

use tokio::net::TcpStream;

/// Alias for a convenient way of naming the type of a TLS client.
#[cfg(feature = "tls")]
pub type TlsClient = Client<tokio_native_tls::TlsStream<TcpStream>>;

/// Alias for a convenient way of naming the type of a basic client.
pub type BasicClient = Client<TcpStream>;

// Re-export initial data from server.
pub use multichat_proto::ServerInit;
