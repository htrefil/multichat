//! # multichat-client
//! Crate containing code for buiding clients for the Multichat protocol - a small and efficient
//! protocol used for bridging textual communications from various sources over the internet.
//!
//! # Features
//! - `tls` -- enables clients to connect to TLS encrypted servers with native_tls; enabled by default
//!
//! # Example echo client
//! ```rust
//! let (init, mut client) = ClientBuilder::basic().connect("127.0.0.1:8585").await?;
//!
//! // Find a group named "fun" and join it.
//! let gid = init.groups.get("fun").unwrap();
//! client.join_group(gid).await?;
//!
//! // Create a new user in the group.
//! let uid = client.join_client(gid, "example").await?;
//!
//! loop {
//!     // Read what others say and repeat it.
//!     let update = client.read_update().await?;
//!     if update.uid != uid {
//!         if let UpdateKind::Message(message) = update.kind {
//!             client
//!                 .send_message(uid, &message)
//!                 .await?;
//!         }
//!     }
//! }
//! ```

mod builder;
mod client;
mod tls;

pub use builder::{ClientBuilder, ConnectError};
pub use client::{Client, Update, UpdateKind};
