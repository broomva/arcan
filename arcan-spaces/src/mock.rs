use crate::error::SpacesBridgeError;
use crate::port::{
    SpacesChannel, SpacesChannelType, SpacesDirectMessage, SpacesMessage, SpacesMessageType,
    SpacesPort,
};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Mock implementation of [`SpacesPort`] for testing.
///
/// Uses interior mutability to record sent messages and DMs.
/// Deterministic IDs via atomic counter.
pub struct MockSpacesClient {
    channels: Mutex<Vec<SpacesChannel>>,
    messages: Mutex<Vec<SpacesMessage>>,
    dms: Mutex<Vec<SpacesDirectMessage>>,
    next_id: AtomicU64,
    /// Set to `Some(error_message)` to force all operations to fail.
    pub force_error: Mutex<Option<String>>,
}

impl MockSpacesClient {
    /// Create a mock with pre-seeded channels (general, agent-logs, system).
    pub fn default_hub() -> Self {
        let channels = vec![
            SpacesChannel {
                id: 1,
                server_id: 1,
                name: "general".to_string(),
                channel_type: SpacesChannelType::Text,
                description: Some("General discussion".to_string()),
            },
            SpacesChannel {
                id: 2,
                server_id: 1,
                name: "agent-logs".to_string(),
                channel_type: SpacesChannelType::AgentLog,
                description: Some("Agent activity logs".to_string()),
            },
            SpacesChannel {
                id: 3,
                server_id: 1,
                name: "system".to_string(),
                channel_type: SpacesChannelType::Announcement,
                description: Some("System announcements".to_string()),
            },
        ];

        Self {
            channels: Mutex::new(channels),
            messages: Mutex::new(Vec::new()),
            dms: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(100),
            force_error: Mutex::new(None),
        }
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn check_error(&self) -> Result<(), SpacesBridgeError> {
        let guard = self.force_error.lock().expect("force_error lock poisoned");
        if let Some(msg) = guard.as_ref() {
            return Err(SpacesBridgeError::Connection(msg.clone()));
        }
        Ok(())
    }

    /// Return all messages sent through this mock.
    pub fn sent_messages(&self) -> Vec<SpacesMessage> {
        self.messages
            .lock()
            .expect("messages lock poisoned")
            .clone()
    }

    /// Return all DMs sent through this mock.
    pub fn sent_dms(&self) -> Vec<SpacesDirectMessage> {
        self.dms.lock().expect("dms lock poisoned").clone()
    }

    /// Number of messages sent.
    pub fn message_count(&self) -> usize {
        self.messages.lock().expect("messages lock poisoned").len()
    }

    /// Number of DMs sent.
    pub fn dm_count(&self) -> usize {
        self.dms.lock().expect("dms lock poisoned").len()
    }
}

impl SpacesPort for MockSpacesClient {
    fn send_message(
        &self,
        channel_id: u64,
        content: &str,
        thread_id: Option<u64>,
        reply_to_id: Option<u64>,
    ) -> Result<SpacesMessage, SpacesBridgeError> {
        self.check_error()?;

        let channels = self.channels.lock().expect("channels lock poisoned");
        if !channels.iter().any(|c| c.id == channel_id) {
            return Err(SpacesBridgeError::ChannelNotFound(format!(
                "channel {channel_id}"
            )));
        }
        drop(channels);

        let msg = SpacesMessage {
            id: self.next_id(),
            channel_id,
            sender: "mock-agent".to_string(),
            content: content.to_string(),
            message_type: SpacesMessageType::Text,
            created_at: 1_700_000_000_000_000,
            thread_id,
            reply_to_id,
        };

        self.messages
            .lock()
            .expect("messages lock poisoned")
            .push(msg.clone());
        Ok(msg)
    }

    fn list_channels(&self, server_id: u64) -> Result<Vec<SpacesChannel>, SpacesBridgeError> {
        self.check_error()?;

        let channels = self.channels.lock().expect("channels lock poisoned");
        let filtered: Vec<SpacesChannel> = channels
            .iter()
            .filter(|c| c.server_id == server_id)
            .cloned()
            .collect();
        Ok(filtered)
    }

    fn read_messages(
        &self,
        channel_id: u64,
        limit: u32,
        before_id: Option<u64>,
    ) -> Result<Vec<SpacesMessage>, SpacesBridgeError> {
        self.check_error()?;

        let messages = self.messages.lock().expect("messages lock poisoned");
        let filtered: Vec<SpacesMessage> = messages
            .iter()
            .filter(|m| m.channel_id == channel_id)
            .filter(|m| match before_id {
                Some(bid) => m.id < bid,
                None => true,
            })
            .take(limit as usize)
            .cloned()
            .collect();
        Ok(filtered)
    }

    fn send_dm(
        &self,
        recipient: &str,
        content: &str,
    ) -> Result<SpacesDirectMessage, SpacesBridgeError> {
        self.check_error()?;

        let dm = SpacesDirectMessage {
            id: self.next_id(),
            sender: "mock-agent".to_string(),
            recipient: recipient.to_string(),
            content: content.to_string(),
            created_at: 1_700_000_000_000_000,
        };

        self.dms.lock().expect("dms lock poisoned").push(dm.clone());
        Ok(dm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hub_has_three_channels() {
        let mock = MockSpacesClient::default_hub();
        let channels = mock.list_channels(1).unwrap();
        assert_eq!(channels.len(), 3);
        assert_eq!(channels[0].name, "general");
        assert_eq!(channels[1].name, "agent-logs");
        assert_eq!(channels[2].name, "system");
    }

    #[test]
    fn send_message_records_and_returns() {
        let mock = MockSpacesClient::default_hub();
        let msg = mock.send_message(1, "hello", None, None).unwrap();
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.channel_id, 1);
        assert_eq!(mock.message_count(), 1);
    }

    #[test]
    fn send_message_to_nonexistent_channel_fails() {
        let mock = MockSpacesClient::default_hub();
        let result = mock.send_message(999, "hello", None, None);
        assert!(result.is_err());
    }

    #[test]
    fn force_error_propagates() {
        let mock = MockSpacesClient::default_hub();
        *mock.force_error.lock().unwrap() = Some("test failure".to_string());
        assert!(mock.list_channels(1).is_err());
        assert!(mock.send_message(1, "x", None, None).is_err());
        assert!(mock.send_dm("abc", "x").is_err());
    }
}
