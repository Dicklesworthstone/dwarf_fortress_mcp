#![forbid(unsafe_code)]

//! Streamable HTTP Transport and Session Resumption Manager.
//!
//! WP-MCP-02: Provides modern-only HTTP streamable session resumption tokens,
//! message sequence offset tracking, and reconnectable replay buffers.

use std::collections::{BTreeMap, VecDeque};

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result, SessionId};

/// Maximum buffered messages per session before oldest are shed.
pub const MAX_RESUMPTION_BUFFER_SIZE: usize = 1000;

/// Cryptographic session resumption token for streamable HTTP transports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpSessionResumeToken {
    pub session_id: SessionId,
    pub resume_offset: u64,
    pub token_digest: Digest32,
}

impl HttpSessionResumeToken {
    #[must_use]
    pub fn new(session_id: SessionId, resume_offset: u64) -> Self {
        let mut hasher_bytes = Vec::new();
        hasher_bytes.extend_from_slice(&session_id.get().to_be_bytes());
        hasher_bytes.extend_from_slice(&resume_offset.to_be_bytes());

        let token_digest = Digest32::of_bytes(&hasher_bytes);
        Self {
            session_id,
            resume_offset,
            token_digest,
        }
    }

    #[must_use]
    pub fn verify_signature(&self) -> bool {
        let expected = Self::new(self.session_id, self.resume_offset);
        self.token_digest == expected.token_digest
    }
}

/// Buffer tracking messages for a single HTTP streaming session.
#[derive(Clone, Debug)]
struct SessionMessageBuffer {
    start_offset: u64,
    messages: VecDeque<String>,
}

/// Streamable HTTP Transport Session Manager.
#[derive(Clone, Debug, Default)]
pub struct HttpTransportSessionManager {
    sessions: BTreeMap<SessionId, SessionMessageBuffer>,
}

impl HttpTransportSessionManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: BTreeMap::new(),
        }
    }

    /// Open or register a new HTTP session.
    pub fn open_session(&mut self, session_id: SessionId) -> HttpSessionResumeToken {
        self.sessions.insert(
            session_id,
            SessionMessageBuffer {
                start_offset: 0,
                messages: VecDeque::with_capacity(64),
            },
        );
        HttpSessionResumeToken::new(session_id, 0)
    }

    /// Buffer an outgoing message for a session.
    pub fn buffer_message(&mut self, session_id: SessionId, message: String) -> Result<u64> {
        let buf = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| DfmcpError::new(ErrorCode::SessionNotFound, "session not found"))?;

        if buf.messages.len() >= MAX_RESUMPTION_BUFFER_SIZE {
            buf.messages.pop_front();
            buf.start_offset = buf.start_offset.saturating_add(1);
        }

        buf.messages.push_back(message);
        let next_offset = buf.start_offset + buf.messages.len() as u64;
        Ok(next_offset)
    }

    /// Resume session from a given token, returning all messages since `resume_offset`.
    pub fn resume_session(&self, token: &HttpSessionResumeToken) -> Result<Vec<String>> {
        if !token.verify_signature() {
            return Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "invalid session resumption token signature",
            ));
        }

        let buf = self.sessions.get(&token.session_id).ok_or_else(|| {
            DfmcpError::new(ErrorCode::SessionNotFound, "session not found or expired")
        })?;

        let current_head = buf.start_offset + buf.messages.len() as u64;
        if token.resume_offset < buf.start_offset {
            return Err(DfmcpError::new(
                ErrorCode::CursorGap,
                format!(
                    "requested offset {} is older than buffer horizon {}; full refresh required",
                    token.resume_offset, buf.start_offset
                ),
            ));
        }

        if token.resume_offset > current_head {
            return Err(DfmcpError::new(
                ErrorCode::CursorGap,
                "requested resume offset is ahead of current server sequence",
            ));
        }

        let skip_count = (token.resume_offset - buf.start_offset) as usize;
        let messages: Vec<String> = buf.messages.iter().skip(skip_count).cloned().collect();
        Ok(messages)
    }

    /// Close and clean up session buffer.
    pub fn close_session(&mut self, session_id: SessionId) {
        self.sessions.remove(&session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_session_buffering_and_resumption() -> Result<()> {
        let mut manager = HttpTransportSessionManager::new();
        let session = SessionId::new(100);

        let initial_token = manager.open_session(session);
        assert_eq!(initial_token.resume_offset, 0);

        // Buffer 3 messages
        manager.buffer_message(session, "msg 1".to_owned())?;
        manager.buffer_message(session, "msg 2".to_owned())?;
        manager.buffer_message(session, "msg 3".to_owned())?;

        // Resume from offset 0 -> returns all 3
        let all_msgs = manager.resume_session(&initial_token)?;
        assert_eq!(all_msgs.len(), 3);

        // Resume from offset 2 -> returns only "msg 3"
        let resume_token = HttpSessionResumeToken::new(session, 2);
        let partial_msgs = manager.resume_session(&resume_token)?;
        assert_eq!(partial_msgs.len(), 1);
        assert_eq!(partial_msgs[0], "msg 3");

        Ok(())
    }
}
