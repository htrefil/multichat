//! Crate containing definitions and utilities for working with the Multichat protocol - a small and efficient
//! protocol used for bridging chat communication from various sources over the internet.
mod access_token;
mod client;
mod server;
mod version;
mod wire;

pub use access_token::AccessToken;
pub use client::{AuthRequest, ClientMessage};
pub use server::{Attachment, AuthResponse, ServerMessage};
pub use version::{Version, VERSION};
pub use wire::{read, write, Config};
