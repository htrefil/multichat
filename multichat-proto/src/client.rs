use serde::{Deserialize, Serialize};

/// Message sent by client to server.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub enum ClientMessage {
    /// Request to list groups.
    ListGroups,
    /// Subscribe to a groups updates.
    JoinGroup { gid: usize },
    /// Unsubscribe from a groups messages.
    LeaveGroup { gid: usize },
    /// Join a group as a particular client.
    JoinClient { gid: usize, name: String },
    /// Leave a group as a particular client.
    LeaveClient { gid: usize, cid: usize },
    /// Send a message as a particular client.
    SendMessage {
        gid: usize,
        cid: usize,
        message: String,
    },
}
