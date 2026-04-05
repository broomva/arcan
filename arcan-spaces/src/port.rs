use crate::error::SpacesBridgeError;
use serde::{Deserialize, Serialize};

/// Channel type mirroring the Spaces WASM module's `ChannelType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpacesChannelType {
    Text,
    Voice,
    Announcement,
    AgentLog,
}

/// A Spaces channel (bridge-local type, no SDK dependency).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpacesChannel {
    pub id: u64,
    pub server_id: u64,
    pub name: String,
    pub channel_type: SpacesChannelType,
    pub description: Option<String>,
}

/// Message type mirroring the Spaces WASM module's `MessageType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpacesMessageType {
    Text,
    System,
    Join,
    Leave,
    AgentEvent,
}

/// A message in a Spaces channel (bridge-local type).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpacesMessage {
    pub id: u64,
    pub channel_id: u64,
    /// Sender identity as hex string.
    pub sender: String,
    pub content: String,
    pub message_type: SpacesMessageType,
    /// Microseconds since epoch.
    pub created_at: i64,
    pub thread_id: Option<u64>,
    pub reply_to_id: Option<u64>,
}

/// A direct message (bridge-local type).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpacesDirectMessage {
    pub id: u64,
    /// Sender identity as hex string.
    pub sender: String,
    /// Recipient identity as hex string.
    pub recipient: String,
    pub content: String,
    /// Microseconds since epoch.
    pub created_at: i64,
}

/// Port trait for Spaces operations.
///
/// All methods are synchronous to match the `Tool` trait contract.
/// Concrete implementations that use async SDK calls should handle
/// the async-to-sync bridge internally (e.g. via `spawn_blocking`).
pub trait SpacesPort: Send + Sync {
    /// Send a message to a channel.
    fn send_message(
        &self,
        channel_id: u64,
        content: &str,
        thread_id: Option<u64>,
        reply_to_id: Option<u64>,
    ) -> Result<SpacesMessage, SpacesBridgeError>;

    /// List channels in a server.
    fn list_channels(&self, server_id: u64) -> Result<Vec<SpacesChannel>, SpacesBridgeError>;

    /// Read messages from a channel.
    fn read_messages(
        &self,
        channel_id: u64,
        limit: u32,
        before_id: Option<u64>,
    ) -> Result<Vec<SpacesMessage>, SpacesBridgeError>;

    /// Send a direct message to a recipient.
    fn send_dm(
        &self,
        recipient: &str,
        content: &str,
    ) -> Result<SpacesDirectMessage, SpacesBridgeError>;
}
