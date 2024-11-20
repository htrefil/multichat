use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;

/// Message sent by server to client.
#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
pub enum ServerMessage<'a> {
    /// A new user has joined a group.
    InitUser {
        gid: u32,
        uid: u32,
        name: Cow<'a, str>,
    },
    /// A user has left a group.
    LeaveUser { gid: u32, uid: u32 },
    /// A message was sent to a group that a client has susbcribed to.
    Message {
        gid: u32,
        uid: u32,
        message: Cow<'a, str>,
        attachments: Vec<Attachment>,
    },
    /// A user was renamed.
    RenameUser {
        gid: u32,
        uid: u32,
        name: Cow<'a, str>,
    },
    /// Server confirms a [`ClientMessage::JoinUser`](crate::client::ClientMessage::JoinUser) request.
    ConfirmClient { uid: u32 },
    /// Server sends an attachment.
    Attachment { data: Cow<'a, [u8]> },
}

/// Attachment to a message.
#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
pub struct Attachment {
    pub id: u32,
    pub size: u64,
}

/// Response to an [`AuthRequest`](crate::client::AuthRequest).
#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
pub enum AuthResponse<'a> {
    /// The client has been authenticated.
    Success {
        /// The groups available to the client.
        groups: HashMap<Cow<'a, str>, u32>,
    },
    /// The client could not be authenticated.
    Failed,
}
