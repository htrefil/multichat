use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::time::Duration;

/// Message sent by server to client.
#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
pub enum ServerMessage<'a> {
    /// A new group has been created.
    InitGroup { name: Cow<'a, str>, gid: u32 },
    /// A group has been destroyed.
    DestroyGroup { gid: u32 },
    /// A new user has joined a group.
    InitUser {
        gid: u32,
        uid: u32,
        name: Cow<'a, str>,
    },
    /// A user has left a group.
    DestroyUser { gid: u32, uid: u32 },
    /// A message was sent to a group that a client has susbcribed to.
    Message {
        gid: u32,
        uid: u32,
        message: Cow<'a, str>,
        attachments: Vec<Attachment>,
    },
    /// A user was renamed.
    Rename {
        gid: u32,
        uid: u32,
        name: Cow<'a, str>,
    },
    /// Server confirms a [`ClientMessage::JoinUser`](crate::client::ClientMessage::JoinUser) request.
    ConfirmUser { uid: u32 },
    /// Server confirms a [`ClientMessage::JoinGroup`](crate::client::ClientMessage::JoinGroup) request.
    ConfirmGroup { gid: u32 },
    /// Server sends an attachment.
    Attachment { data: Cow<'a, [u8]> },
    /// Ping, used to keep the connection alive.
    Ping,
}

/// Attachment to a message.
#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
pub struct Attachment {
    /// Per-connection ID of the attachment.
    /// After this attachment is either ignored or downloaded, the ID may be reused.
    pub id: u32,
    /// Size of the attachment in bytes.
    pub size: u64,
}

/// Response to an [`AuthRequest`](crate::client::AuthRequest).
#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
pub enum AuthResponse {
    /// The client has been authenticated.
    Success {
        ping_interval: Duration,
        ping_timeout: Duration,
    },
    /// The client could not be authenticated.
    Failed,
}
