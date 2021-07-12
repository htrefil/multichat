use crate::text::Chunk;

use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// Message sent by client to server.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub enum ClientMessage<'a> {
    /// Subscribe to a groups updates.
    JoinGroup { gid: usize },
    /// Unsubscribe from a groups messages.
    LeaveGroup { gid: usize },
    /// Join a group as an user.
    JoinUser { gid: usize, name: Cow<'a, str> },
    /// Leave a group as an user.
    LeaveUser { gid: usize, uid: usize },
    /// Send a message as an user.
    SendMessage {
        gid: usize,
        uid: usize,
        message: Cow<'a, [Chunk<'a>]>,
    },
}
