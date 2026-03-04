//! SpacetimeDB HTTP REST API client implementing [`SpacesPort`].
//!
//! Connects to a SpacetimeDB instance via its HTTP endpoints:
//! - SQL reads: `POST /v1/database/{db}/sql`
//! - Reducer calls: `POST /v1/database/{db}/call/{reducer}`
//!
//! Authentication uses a Bearer token resolved from (in order):
//! 1. Explicit `token` field
//! 2. `SPACETIMEDB_TOKEN` environment variable
//! 3. `~/.config/spacetime/cli.toml` (`web_session_token`)

use crate::error::SpacesBridgeError;
use crate::port::{
    SpacesChannel, SpacesChannelType, SpacesDirectMessage, SpacesMessage, SpacesMessageType,
    SpacesPort,
};
use reqwest::blocking::Client;
use serde_json::Value;

/// Configuration for connecting to a SpacetimeDB instance.
#[derive(Debug, Clone)]
pub struct SpacetimeDbConfig {
    pub host: String,
    pub database_id: String,
    pub token: String,
}

impl SpacetimeDbConfig {
    /// Resolve configuration with layered fallback:
    /// explicit arg > env var > CLI config file > default.
    pub fn resolve(
        host: Option<&str>,
        database_id: Option<&str>,
        token: Option<&str>,
    ) -> Result<Self, SpacesBridgeError> {
        let host = host
            .map(String::from)
            .or_else(|| std::env::var("SPACETIMEDB_HOST").ok())
            .unwrap_or_else(|| "https://maincloud.spacetimedb.com".to_string());

        let database_id = database_id
            .map(String::from)
            .or_else(|| std::env::var("SPACETIMEDB_DATABASE").ok())
            .ok_or_else(|| {
                SpacesBridgeError::AuthError(
                    "no database_id: set SPACETIMEDB_DATABASE or pass --spaces-database".into(),
                )
            })?;

        let token = token
            .map(String::from)
            .or_else(|| std::env::var("SPACETIMEDB_TOKEN").ok())
            .or_else(read_cli_token)
            .ok_or_else(|| {
                SpacesBridgeError::AuthError(
                    "no SpacetimeDB token: set SPACETIMEDB_TOKEN, pass --spaces-token, or run `spacetime login`".into(),
                )
            })?;

        Ok(Self {
            host,
            database_id,
            token,
        })
    }
}

/// Read the web_session_token from `~/.config/spacetime/cli.toml`.
fn read_cli_token() -> Option<String> {
    let config_dir = dirs::config_dir()?;
    let path = config_dir.join("spacetime").join("cli.toml");
    let content = std::fs::read_to_string(path).ok()?;
    let table: toml::Table = content.parse().ok()?;
    table
        .get("web_session_token")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// HTTP client for SpacetimeDB's REST API.
pub struct SpacetimeDbClient {
    client: Client,
    config: SpacetimeDbConfig,
}

impl SpacetimeDbClient {
    pub fn new(config: SpacetimeDbConfig) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");
        Self { client, config }
    }

    /// Execute a SQL query and return the rows from the first result set.
    fn sql_query(&self, sql: &str) -> Result<Vec<Vec<Value>>, SpacesBridgeError> {
        let url = format!(
            "{}/v1/database/{}/sql",
            self.config.host, self.config.database_id
        );

        tracing::debug!(sql = %sql, "SpacetimeDB SQL query");

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.config.token)
            .body(sql.to_string())
            .send()
            .map_err(|e| SpacesBridgeError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(SpacesBridgeError::Http(format!(
                "SQL query failed ({status}): {body}"
            )));
        }

        let body: Value = resp
            .json()
            .map_err(|e| SpacesBridgeError::ParseError(format!("failed to parse JSON: {e}")))?;

        // Response shape: [{"schema": {...}, "rows": [[...], ...]}, ...]
        let result_sets = body
            .as_array()
            .ok_or_else(|| SpacesBridgeError::ParseError("expected array response".into()))?;

        if result_sets.is_empty() {
            return Ok(Vec::new());
        }

        let rows = result_sets[0]
            .get("rows")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        let parsed: Vec<Vec<Value>> = rows
            .into_iter()
            .filter_map(|row| row.as_array().cloned())
            .collect();

        Ok(parsed)
    }

    /// Call a reducer with JSON arguments.
    fn call_reducer(&self, name: &str, args: &Value) -> Result<(), SpacesBridgeError> {
        let url = format!(
            "{}/v1/database/{}/call/{}",
            self.config.host, self.config.database_id, name
        );

        tracing::debug!(reducer = %name, "SpacetimeDB reducer call");

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.config.token)
            .json(args)
            .send()
            .map_err(|e| SpacesBridgeError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(SpacesBridgeError::ReducerFailed(format!(
                "{name} failed ({status}): {body}"
            )));
        }

        Ok(())
    }
}

// --- Row parsing helpers ---

/// Parse a SpacetimeDB Option (sum type): `[0, value]` = Some, `[1, []]` = None.
fn parse_option_str(val: &Value) -> Option<String> {
    let arr = val.as_array()?;
    if arr.len() != 2 {
        return None;
    }
    match arr[0].as_u64()? {
        0 => arr[1].as_str().map(String::from),
        _ => None,
    }
}

/// Parse a SpacetimeDB Option<u64>.
fn parse_option_u64(val: &Value) -> Option<u64> {
    let arr = val.as_array()?;
    if arr.len() != 2 {
        return None;
    }
    match arr[0].as_u64()? {
        0 => arr[1].as_u64(),
        _ => None,
    }
}

/// Parse a SpacetimeDB enum variant index into a channel type.
fn parse_channel_type(val: &Value) -> SpacesChannelType {
    // Sum type encoding: [variant_index, payload]
    // Or might be just a plain string or number depending on schema.
    if let Some(arr) = val.as_array() {
        if let Some(idx) = arr.first().and_then(Value::as_u64) {
            return match idx {
                0 => SpacesChannelType::Text,
                1 => SpacesChannelType::Voice,
                2 => SpacesChannelType::Announcement,
                3 => SpacesChannelType::AgentLog,
                _ => SpacesChannelType::Text,
            };
        }
    }
    // Fallback: try as string
    if let Some(s) = val.as_str() {
        return match s {
            "Voice" => SpacesChannelType::Voice,
            "Announcement" => SpacesChannelType::Announcement,
            "AgentLog" => SpacesChannelType::AgentLog,
            _ => SpacesChannelType::Text,
        };
    }
    SpacesChannelType::Text
}

/// Parse a SpacetimeDB enum variant index into a message type.
fn parse_message_type(val: &Value) -> SpacesMessageType {
    if let Some(arr) = val.as_array() {
        if let Some(idx) = arr.first().and_then(Value::as_u64) {
            return match idx {
                0 => SpacesMessageType::Text,
                1 => SpacesMessageType::System,
                2 => SpacesMessageType::Join,
                3 => SpacesMessageType::Leave,
                4 => SpacesMessageType::AgentEvent,
                _ => SpacesMessageType::Text,
            };
        }
    }
    if let Some(s) = val.as_str() {
        return match s {
            "System" => SpacesMessageType::System,
            "Join" => SpacesMessageType::Join,
            "Leave" => SpacesMessageType::Leave,
            "AgentEvent" => SpacesMessageType::AgentEvent,
            _ => SpacesMessageType::Text,
        };
    }
    SpacesMessageType::Text
}

/// Parse identity bytes (hex-encoded or raw bytes array) into a hex string.
fn parse_identity(val: &Value) -> String {
    if let Some(s) = val.as_str() {
        return s.to_string();
    }
    // SpacetimeDB may encode Identity as an array of bytes
    if let Some(arr) = val.as_array() {
        let bytes: Vec<u8> = arr
            .iter()
            .filter_map(|v| v.as_u64().map(|n| n as u8))
            .collect();
        if !bytes.is_empty() {
            return hex::encode(bytes);
        }
    }
    // Nested object with __identity_bytes
    if let Some(obj) = val.as_object() {
        if let Some(hex_str) = obj.get("__identity_bytes").and_then(|v| v.as_str()) {
            return hex_str.to_string();
        }
    }
    val.to_string()
}

/// Parse a timestamp value (microseconds since epoch).
fn parse_timestamp(val: &Value) -> i64 {
    // SpacetimeDB Timestamp can be encoded as:
    // - plain integer (microseconds)
    // - object with "microseconds" field
    if let Some(n) = val.as_i64() {
        return n;
    }
    if let Some(obj) = val.as_object() {
        if let Some(us) = obj.get("microseconds").and_then(Value::as_i64) {
            return us;
        }
    }
    0
}

/// Parse a channel row.
///
/// Expected column order: `id, server_id, name, channel_type, description, position, created_at`.
fn parse_channel_row(row: &[Value]) -> Result<SpacesChannel, SpacesBridgeError> {
    if row.len() < 5 {
        return Err(SpacesBridgeError::ParseError(format!(
            "channel row too short: {} columns",
            row.len()
        )));
    }

    Ok(SpacesChannel {
        id: row[0]
            .as_u64()
            .ok_or_else(|| SpacesBridgeError::ParseError("channel id not u64".into()))?,
        server_id: row[1]
            .as_u64()
            .ok_or_else(|| SpacesBridgeError::ParseError("server_id not u64".into()))?,
        name: row[2]
            .as_str()
            .ok_or_else(|| SpacesBridgeError::ParseError("channel name not string".into()))?
            .to_string(),
        channel_type: parse_channel_type(&row[3]),
        description: parse_option_str(&row[4]),
    })
}

/// Parse a message row.
///
/// Expected column order: `id, channel_id, sender, content, message_type, created_at, thread_id, reply_to_id`.
fn parse_message_row(row: &[Value]) -> Result<SpacesMessage, SpacesBridgeError> {
    if row.len() < 6 {
        return Err(SpacesBridgeError::ParseError(format!(
            "message row too short: {} columns",
            row.len()
        )));
    }

    Ok(SpacesMessage {
        id: row[0]
            .as_u64()
            .ok_or_else(|| SpacesBridgeError::ParseError("message id not u64".into()))?,
        channel_id: row[1]
            .as_u64()
            .ok_or_else(|| SpacesBridgeError::ParseError("channel_id not u64".into()))?,
        sender: parse_identity(&row[2]),
        content: row[3]
            .as_str()
            .ok_or_else(|| SpacesBridgeError::ParseError("content not string".into()))?
            .to_string(),
        message_type: parse_message_type(&row[4]),
        created_at: parse_timestamp(&row[5]),
        thread_id: if row.len() > 6 {
            parse_option_u64(&row[6])
        } else {
            None
        },
        reply_to_id: if row.len() > 7 {
            parse_option_u64(&row[7])
        } else {
            None
        },
    })
}

// --- SpacesPort implementation ---

impl SpacesPort for SpacetimeDbClient {
    fn list_channels(&self, server_id: u64) -> Result<Vec<SpacesChannel>, SpacesBridgeError> {
        let sql = format!("SELECT * FROM channel WHERE server_id = {server_id}");
        let rows = self.sql_query(&sql)?;

        let mut channels: Vec<SpacesChannel> = rows
            .iter()
            .map(|r| parse_channel_row(r))
            .collect::<Result<Vec<_>, _>>()?;

        // ORDER BY not supported in SpacetimeDB SQL — sort client-side by position/id.
        // Position is column 5 if present, otherwise sort by id.
        channels.sort_by_key(|c| c.id);

        Ok(channels)
    }

    fn read_messages(
        &self,
        channel_id: u64,
        limit: u32,
        before_id: Option<u64>,
    ) -> Result<Vec<SpacesMessage>, SpacesBridgeError> {
        let sql = match before_id {
            Some(bid) => {
                format!("SELECT * FROM message WHERE channel_id = {channel_id} AND id < {bid}")
            }
            None => format!("SELECT * FROM message WHERE channel_id = {channel_id}"),
        };
        let rows = self.sql_query(&sql)?;

        let mut messages: Vec<SpacesMessage> = rows
            .iter()
            .map(|r| parse_message_row(r))
            .collect::<Result<Vec<_>, _>>()?;

        // Sort by id ascending (oldest first), then take last `limit`.
        messages.sort_by_key(|m| m.id);

        // Take the last `limit` messages (most recent).
        let start = messages.len().saturating_sub(limit as usize);
        let recent = messages[start..].to_vec();

        Ok(recent)
    }

    fn send_message(
        &self,
        channel_id: u64,
        content: &str,
        _thread_id: Option<u64>,
        _reply_to_id: Option<u64>,
    ) -> Result<SpacesMessage, SpacesBridgeError> {
        // Call the send_message reducer.
        let args = serde_json::json!({
            "channel_id": channel_id,
            "content": content
        });
        self.call_reducer("send_message", &args)?;

        // Read back the latest message from this channel as confirmation.
        let messages = self.read_messages(channel_id, 1, None)?;
        messages.into_iter().last().ok_or_else(|| {
            SpacesBridgeError::ParseError(
                "send_message succeeded but read-back returned no messages".into(),
            )
        })
    }

    fn send_dm(
        &self,
        recipient: &str,
        content: &str,
    ) -> Result<SpacesDirectMessage, SpacesBridgeError> {
        let args = serde_json::json!({
            "recipient": recipient,
            "content": content
        });
        self.call_reducer("send_direct_message", &args)?;

        // Best-effort read-back: query DMs for this recipient.
        let sql = format!("SELECT * FROM direct_message WHERE recipient = X'{recipient}'");
        let rows = self.sql_query(&sql).unwrap_or_default();

        // Parse last DM if available, otherwise synthesize.
        if let Some(row) = rows.last() {
            if row.len() >= 5 {
                return Ok(SpacesDirectMessage {
                    id: row[0].as_u64().unwrap_or(0),
                    sender: parse_identity(&row[1]),
                    recipient: parse_identity(&row[2]),
                    content: row[3].as_str().unwrap_or(content).to_string(),
                    created_at: parse_timestamp(&row[4]),
                });
            }
        }

        // Fallback: return synthetic confirmation.
        Ok(SpacesDirectMessage {
            id: 0,
            sender: "self".to_string(),
            recipient: recipient.to_string(),
            content: content.to_string(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_micros() as i64)
                .unwrap_or(0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Row parsing tests ---

    #[test]
    fn parse_option_str_some() {
        let val = serde_json::json!([0, "hello"]);
        assert_eq!(parse_option_str(&val), Some("hello".to_string()));
    }

    #[test]
    fn parse_option_str_none() {
        let val = serde_json::json!([1, []]);
        assert_eq!(parse_option_str(&val), None);
    }

    #[test]
    fn parse_option_u64_some() {
        let val = serde_json::json!([0, 42]);
        assert_eq!(parse_option_u64(&val), Some(42));
    }

    #[test]
    fn parse_option_u64_none() {
        let val = serde_json::json!([1, []]);
        assert_eq!(parse_option_u64(&val), None);
    }

    #[test]
    fn parse_channel_type_from_sum_type() {
        assert_eq!(
            parse_channel_type(&serde_json::json!([0, []])),
            SpacesChannelType::Text
        );
        assert_eq!(
            parse_channel_type(&serde_json::json!([1, []])),
            SpacesChannelType::Voice
        );
        assert_eq!(
            parse_channel_type(&serde_json::json!([2, []])),
            SpacesChannelType::Announcement
        );
        assert_eq!(
            parse_channel_type(&serde_json::json!([3, []])),
            SpacesChannelType::AgentLog
        );
    }

    #[test]
    fn parse_message_type_from_sum_type() {
        assert_eq!(
            parse_message_type(&serde_json::json!([0, []])),
            SpacesMessageType::Text
        );
        assert_eq!(
            parse_message_type(&serde_json::json!([1, []])),
            SpacesMessageType::System
        );
        assert_eq!(
            parse_message_type(&serde_json::json!([4, []])),
            SpacesMessageType::AgentEvent
        );
    }

    #[test]
    fn parse_identity_hex_string() {
        assert_eq!(parse_identity(&serde_json::json!("abc123")), "abc123");
    }

    #[test]
    fn parse_timestamp_integer() {
        assert_eq!(
            parse_timestamp(&serde_json::json!(1700000000000000_i64)),
            1700000000000000
        );
    }

    #[test]
    fn parse_timestamp_object() {
        let val = serde_json::json!({"microseconds": 1700000000000000_i64});
        assert_eq!(parse_timestamp(&val), 1700000000000000);
    }

    #[test]
    fn parse_channel_row_valid() {
        let row = vec![
            serde_json::json!(1),
            serde_json::json!(1),
            serde_json::json!("general"),
            serde_json::json!([0, []]),
            serde_json::json!([0, "General discussion"]),
            serde_json::json!(0),
            serde_json::json!(1700000000000000_i64),
        ];
        let ch = parse_channel_row(&row).unwrap();
        assert_eq!(ch.id, 1);
        assert_eq!(ch.name, "general");
        assert_eq!(ch.channel_type, SpacesChannelType::Text);
        assert_eq!(ch.description.as_deref(), Some("General discussion"));
    }

    #[test]
    fn parse_channel_row_too_short() {
        let row = vec![serde_json::json!(1), serde_json::json!(1)];
        assert!(parse_channel_row(&row).is_err());
    }

    #[test]
    fn parse_message_row_valid() {
        let row = vec![
            serde_json::json!(42),
            serde_json::json!(1),
            serde_json::json!("abc123"),
            serde_json::json!("hello world"),
            serde_json::json!([0, []]),
            serde_json::json!(1700000000000000_i64),
            serde_json::json!([1, []]),
            serde_json::json!([1, []]),
        ];
        let msg = parse_message_row(&row).unwrap();
        assert_eq!(msg.id, 42);
        assert_eq!(msg.channel_id, 1);
        assert_eq!(msg.sender, "abc123");
        assert_eq!(msg.content, "hello world");
        assert_eq!(msg.message_type, SpacesMessageType::Text);
        assert!(msg.thread_id.is_none());
        assert!(msg.reply_to_id.is_none());
    }

    #[test]
    fn parse_message_row_too_short() {
        let row = vec![serde_json::json!(1), serde_json::json!(1)];
        assert!(parse_message_row(&row).is_err());
    }

    #[test]
    fn parse_message_row_with_thread() {
        let row = vec![
            serde_json::json!(42),
            serde_json::json!(1),
            serde_json::json!("abc"),
            serde_json::json!("hi"),
            serde_json::json!([0, []]),
            serde_json::json!(1700000000000000_i64),
            serde_json::json!([0, 10]),
            serde_json::json!([0, 5]),
        ];
        let msg = parse_message_row(&row).unwrap();
        assert_eq!(msg.thread_id, Some(10));
        assert_eq!(msg.reply_to_id, Some(5));
    }

    // --- Config resolution tests ---

    #[test]
    fn config_resolve_requires_database_id() {
        let result = SpacetimeDbConfig::resolve(None, None, Some("token123"));
        assert!(result.is_err());
    }

    #[test]
    fn config_resolve_with_explicit_token_succeeds() {
        // When all three values are explicit, resolve always succeeds.
        let result = SpacetimeDbConfig::resolve(None, Some("db123"), Some("tok"));
        assert!(result.is_ok());
    }

    #[test]
    fn config_resolve_explicit_values() {
        let cfg =
            SpacetimeDbConfig::resolve(Some("https://example.com"), Some("mydb"), Some("tok123"))
                .unwrap();
        assert_eq!(cfg.host, "https://example.com");
        assert_eq!(cfg.database_id, "mydb");
        assert_eq!(cfg.token, "tok123");
    }

    // --- Live integration tests (require real SpacetimeDB) ---

    #[test]
    #[ignore]
    fn live_list_channels() {
        let config = SpacetimeDbConfig::resolve(None, None, None)
            .expect("SPACETIMEDB_DATABASE and token required");
        let client = SpacetimeDbClient::new(config);
        let channels = client.list_channels(1).expect("list_channels failed");
        assert!(!channels.is_empty(), "expected at least one channel");
        for ch in &channels {
            eprintln!("  #{}: {} ({:?})", ch.id, ch.name, ch.channel_type);
        }
    }

    #[test]
    #[ignore]
    fn live_read_messages() {
        let config = SpacetimeDbConfig::resolve(None, None, None)
            .expect("SPACETIMEDB_DATABASE and token required");
        let client = SpacetimeDbClient::new(config);
        let messages = client
            .read_messages(1, 5, None)
            .expect("read_messages failed");
        for msg in &messages {
            eprintln!("  #{}: [{}] {}", msg.id, msg.sender, msg.content);
        }
    }
}
