use serde::{Deserialize, Serialize};
use std::borrow::Cow;

use crate::access_token::AccessToken;

/// Message sent by client to server.
#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
pub enum ClientMessage<'a, 'b> {
    /// Subscribe to a groups updates.
    /// Creates a new group if it does not exist.
    JoinGroup { name: Cow<'a, str> },
    /// Unsubscribe from a groups messages.
    LeaveGroup { gid: u32 },
    /// Join a group as a user.
    InitUser { gid: u32, name: Cow<'a, str> },
    /// Leave a group as a user.
    DestroyUser { gid: u32, uid: u32 },
    /// Change the name of a user.
    Rename {
        gid: u32,
        uid: u32,
        name: Cow<'a, str>,
    },
    /// Send a message as a user.
    SendMessage {
        gid: u32,
        uid: u32,
        message: Cow<'b, str>,
        attachments: Cow<'b, [Cow<'a, [u8]>]>,
    },
    /// Download an attachment.
    DownloadAttachment { id: u32 },
    /// Ignore an attachment.
    IgnoreAttachment { id: u32 },
    /// Reply to a ping message.
    Pong,
}

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct AuthRequest {
    pub access_token: AccessToken,
}
