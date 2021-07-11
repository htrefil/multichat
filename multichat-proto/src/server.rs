use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Message sent by server to client.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub enum ServerMessage {
    /// Response to [`ClientMessage::ListGroups`](crate::client::ClientMessage::ListGroups).
    ListGroups { groups: HashMap<usize, String> },
    /// A new client has joined a group.
    InitClient {
        gid: usize,
        cid: usize,
        name: String,
    },
    /// A client has left a group.
    LeaveClient { gid: usize, cid: usize },
    /// A message was sent to a group that a client has susbcribed to.
    Message {
        gid: usize,
        cid: usize,
        message: String,
    },
    /// Server confirms [`ClientMessage::JoinClient`](crate::client::ClientMessage::JoinClient) request.
    ConfirmClient { cid: usize },
}
