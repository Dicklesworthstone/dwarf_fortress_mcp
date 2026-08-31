#![forbid(unsafe_code)]

//! Process-local Streamable HTTP resumption laboratory.
//!
//! Tokens are integrity-sealed records and are accepted only when they exactly
//! match a token issued by this manager. The digest is not a cryptographic
//! signature and this module is not wired into the stdio server.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result, SessionId};

pub const MAX_RESUMPTION_BUFFER_SIZE: usize = 1_000;
pub const MAX_HTTP_SESSIONS: usize = 1_024;
pub const MAX_HTTP_MESSAGE_BYTES: usize = 1_048_576;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct HttpSessionResumeToken {
    pub session_id: SessionId,
    pub resume_offset: u64,
    pub token_digest: Digest32,
}

impl HttpSessionResumeToken {
    #[must_use]
    pub fn new(session_id: SessionId, resume_offset: u64) -> Self {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"dfmcp-http-resume-token-v1");
        bytes.extend_from_slice(&session_id.get().to_be_bytes());
        bytes.extend_from_slice(&resume_offset.to_be_bytes());
        Self {
            session_id,
            resume_offset,
            token_digest: Digest32::of_bytes(&bytes),
        }
    }

    #[must_use]
    pub fn integrity_is_valid(&self) -> bool {
        self.token_digest == Self::new(self.session_id, self.resume_offset).token_digest
    }
}

#[derive(Clone, Debug)]
struct SessionMessageBuffer {
    start_offset: u64,
    messages: VecDeque<String>,
}

#[derive(Clone, Debug, Default)]
pub struct HttpTransportSessionManager {
    sessions: BTreeMap<SessionId, SessionMessageBuffer>,
    issued_tokens: BTreeSet<HttpSessionResumeToken>,
}

impl HttpTransportSessionManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: BTreeMap::new(),
            issued_tokens: BTreeSet::new(),
        }
    }

    pub fn open_session(&mut self, session_id: SessionId) -> Result<HttpSessionResumeToken> {
        if session_id == SessionId::NIL {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "HTTP session identifier zero is reserved",
            ));
        }
        if self.sessions.contains_key(&session_id) {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "HTTP session identifier is already open",
            ));
        }
        if self.sessions.len() >= MAX_HTTP_SESSIONS {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "HTTP session manager reached its explicit session bound",
            ));
        }
        self.sessions.insert(
            session_id,
            SessionMessageBuffer {
                start_offset: 0,
                messages: VecDeque::with_capacity(64),
            },
        );
        let token = HttpSessionResumeToken::new(session_id, 0);
        self.issued_tokens.insert(token.clone());
        Ok(token)
    }

    pub fn buffer_message(&mut self, session_id: SessionId, message: String) -> Result<u64> {
        if message.len() > MAX_HTTP_MESSAGE_BYTES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "HTTP resumption message exceeds its explicit byte bound",
            ));
        }
        let buffer = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| DfmcpError::new(ErrorCode::SessionNotFound, "session not found"))?;

        if buffer.messages.len() >= MAX_RESUMPTION_BUFFER_SIZE {
            let next_start = buffer.start_offset.checked_add(1).ok_or_else(|| {
                DfmcpError::new(ErrorCode::BudgetExceeded, "HTTP message offset exhausted")
            })?;
            buffer.messages.pop_front();
            buffer.start_offset = next_start;
            self.issued_tokens.retain(|token| {
                token.session_id != session_id || token.resume_offset >= next_start
            });
        }

        buffer.messages.push_back(message);
        let buffered_count = u64::try_from(buffer.messages.len()).map_err(|_| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "HTTP resumption buffer length cannot be represented",
            )
        })?;
        buffer.start_offset.checked_add(buffered_count).ok_or_else(|| {
            DfmcpError::new(ErrorCode::BudgetExceeded, "HTTP message offset exhausted")
        })
    }

    pub fn issue_resume_token(
        &mut self,
        session_id: SessionId,
        resume_offset: u64,
    ) -> Result<HttpSessionResumeToken> {
        let buffer = self.sessions.get(&session_id).ok_or_else(|| {
            DfmcpError::new(ErrorCode::SessionNotFound, "session not found or expired")
        })?;
        validate_offset(buffer, resume_offset)?;
        let token = HttpSessionResumeToken::new(session_id, resume_offset);
        self.issued_tokens.insert(token.clone());
        Ok(token)
    }

    pub fn resume_session(&self, token: &HttpSessionResumeToken) -> Result<Vec<String>> {
        if !token.integrity_is_valid() || !self.issued_tokens.contains(token) {
            return Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "session resumption token was not issued by this manager or was modified",
            ));
        }
        let buffer = self.sessions.get(&token.session_id).ok_or_else(|| {
            DfmcpError::new(ErrorCode::SessionNotFound, "session not found or expired")
        })?;
        validate_offset(buffer, token.resume_offset)?;
        let relative_offset = token.resume_offset - buffer.start_offset;
        let skip_count = usize::try_from(relative_offset).map_err(|_| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "HTTP resume offset cannot be represented on this platform",
            )
        })?;
        Ok(buffer.messages.iter().skip(skip_count).cloned().collect())
    }

    pub fn close_session(&mut self, session_id: SessionId) {
        self.sessions.remove(&session_id);
        self.issued_tokens
            .retain(|token| token.session_id != session_id);
    }
}

fn validate_offset(buffer: &SessionMessageBuffer, offset: u64) -> Result<()> {
    let message_count = u64::try_from(buffer.messages.len()).map_err(|_| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "HTTP resumption buffer length cannot be represented",
        )
    })?;
    let current_head = buffer.start_offset.checked_add(message_count).ok_or_else(|| {
        DfmcpError::new(ErrorCode::BudgetExceeded, "HTTP message offset exhausted")
    })?;
    if offset < buffer.start_offset {
        return Err(DfmcpError::new(
            ErrorCode::CursorGap,
            format!(
                "requested offset {offset} is older than buffer horizon {}; full refresh required",
                buffer.start_offset
            ),
        ));
    }
    if offset > current_head {
        return Err(DfmcpError::new(
            ErrorCode::CursorGap,
            "requested resume offset is ahead of current server sequence",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_session_buffering_and_resumption() -> Result<()> {
        let mut manager = HttpTransportSessionManager::new();
        let session = SessionId::new(100);
        let initial_token = manager.open_session(session)?;
        manager.buffer_message(session, "msg 1".to_owned())?;
        manager.buffer_message(session, "msg 2".to_owned())?;
        manager.buffer_message(session, "msg 3".to_owned())?;
        assert_eq!(manager.resume_session(&initial_token)?.len(), 3);

        let resume_token = manager.issue_resume_token(session, 2)?;
        let messages = manager.resume_session(&resume_token)?;
        assert_eq!(messages, vec!["msg 3"]);

        let forged = HttpSessionResumeToken::new(session, 1);
        assert!(manager.resume_session(&forged).is_err());
        Ok(())
    }
}
