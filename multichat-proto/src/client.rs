use serde::{Deserialize, Serialize};

/// Message sent by client to server.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub enum ClientMessage {
    /// Subscribe to a groups updates.
    JoinGroup { gid: usize },
    /// Unsubscribe from a groups messages.
    LeaveGroup { gid: usize },
    /// Join a group as an user.
    JoinUser { gid: usize, name: String },
    /// Leave a group as an user.
    LeaveUser { gid: usize, uid: usize },
    /// Send a message as an user.
    SendMessage {
        gid: usize,
        uid: usize,
        message: String,
    },
}
