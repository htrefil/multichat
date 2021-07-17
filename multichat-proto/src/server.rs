use crate::text::Message;

use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;

/// Message sent by server to client.
#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
pub enum ServerMessage<'a> {
    /// A new user has joined a group.
    InitUser {
        gid: usize,
        uid: usize,
        name: Cow<'a, str>,
    },
    /// An user has left a group.
    LeaveUser { gid: usize, uid: usize },
    /// A message was sent to a group that a client has susbcribed to.
    Message {
        gid: usize,
        uid: usize,
        message: Cow<'a, Message<'a>>,
    },
    /// An user was renamed.
    RenameUser {
        gid: usize,
        uid: usize,
        name: Cow<'a, str>,
    },
    /// Server confirms [`ClientMessage::JoinUser`](crate::client::ClientMessage::JoinUser) request.
    ConfirmClient { uid: usize },
}

/// Initial message sent by server, right after sending its [`Version`](crate::version::Version), to a new client.
#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
pub struct ServerInit<'a> {
    pub groups: HashMap<Cow<'a, str>, usize>,
}
