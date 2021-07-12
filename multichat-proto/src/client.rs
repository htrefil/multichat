use crate::text::Chunk;

use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// Message sent by client to server.
#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
pub enum ClientMessage<'a> {
    /// Subscribe to a groups updates.
    JoinGroup { gid: usize },
    /// Unsubscribe from a groups messages.
    LeaveGroup { gid: usize },
    /// Join a group as an user.
    JoinUser { gid: usize, name: Cow<'a, str> },
    /// Leave a group as an user.
    LeaveUser { gid: usize, uid: usize },
    /// Change the name of an user.
    RenameUser {
        gid: usize,
        uid: usize,
        name: Cow<'a, str>,
    },
    /// Send a message as an user.
    SendMessage {
        gid: usize,
        uid: usize,
        message: Cow<'a, [Chunk<'a>]>,
    },
}
