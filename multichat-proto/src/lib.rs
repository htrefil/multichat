//! # multichat-proto
//! Crate containing definitions and utilities for working with the Multichat protocol - a small and efficient
//! protocol used for bridging textual communications from various sources over the internet.
mod client;
mod server;
mod version;
mod wire;

pub use client::ClientMessage;
pub use server::{ServerInit, ServerMessage};
pub use version::{Version, VERSION};
pub use wire::{read, write, Config};
