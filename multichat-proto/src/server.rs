use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Message sent by server to client.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub enum ServerMessage {
    /// Response to [`ClientMessage::ListGroups`](crate::client::ClientMessage::ListGroups).
    ListGroups { groups: HashMap<String, usize> },
    /// A new user has joined a group.
    InitUser {
        gid: usize,
        uid: usize,
        name: String,
    },
    /// An user  has left a group.
    LeaveUser { gid: usize, uid: usize },
    /// A message was sent to a group that a client has susbcribed to.
    Message {
        gid: usize,
        uid: usize,
        message: String,
    },
    /// Server confirms [`ClientMessage::JoinUser`](crate::client::ClientMessage::JoinUser) request.
    ConfirmClient { uid: usize },
}
