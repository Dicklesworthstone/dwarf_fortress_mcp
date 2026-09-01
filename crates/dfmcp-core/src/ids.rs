use core::fmt;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

const LIVE_SESSION_NAMESPACE: u128 = 1u128 << 127;
const PROCESS_SCOPED_SESSION_MARKER: u128 = 1u128 << 126;
const LIVE_SESSION_SEQUENCE_BITS: u32 = 62;
const LIVE_SESSION_SEQUENCE_MASK: u128 = (1u128 << LIVE_SESSION_SEQUENCE_BITS) - 1;

static PROCESS_SESSION_SCOPE: LazyLock<u64> = LazyLock::new(|| {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write(b"dfmcp-live-session-process-scope-v1\0");
    hasher.write_u32(std::process::id());
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    hasher.write_u128(nanos);
    hasher.write(env!("CARGO_PKG_VERSION").as_bytes());
    let value = hasher.finish();
    if value == 0 { 1 } else { value }
});

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SessionId(u128);

impl SessionId {
    pub const NIL: Self = Self(0);

    /// Construct a session identity.
    ///
    /// Ordinary IDs are preserved exactly. Values in the raw live-server
    /// namespace (bit 127 set, bit 126 clear) are encoded with one process
    /// incarnation and a 62-bit monotonic sequence. Encoded values are
    /// idempotent, so parsing a displayed live session ID cannot scope it a
    /// second time. This prevents an old client handle from aliasing the first
    /// session allocated after a process restart.
    #[must_use]
    pub fn new(value: u128) -> Self {
        if is_raw_live_session_id(value) {
            Self(encode_process_scoped_live_session(
                *PROCESS_SESSION_SCOPE,
                value,
            ))
        } else {
            Self(value)
        }
    }

    #[must_use]
    pub const fn get(self) -> u128 {
        self.0
    }

    #[must_use]
    pub const fn is_process_scoped_live(self) -> bool {
        self.0 & (LIVE_SESSION_NAMESPACE | PROCESS_SCOPED_SESSION_MARKER)
            == (LIVE_SESSION_NAMESPACE | PROCESS_SCOPED_SESSION_MARKER)
    }
}

impl fmt::Debug for SessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "SessionId({:032x})", self.0)
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:032x}", self.0)
    }
}

const fn is_raw_live_session_id(value: u128) -> bool {
    value & LIVE_SESSION_NAMESPACE != 0 && value & PROCESS_SCOPED_SESSION_MARKER == 0
}

const fn encode_process_scoped_live_session(process_scope: u64, raw: u128) -> u128 {
    let sequence = raw & LIVE_SESSION_SEQUENCE_MASK;
    LIVE_SESSION_NAMESPACE
        | PROCESS_SCOPED_SESSION_MARKER
        | (u128::from(process_scope) << LIVE_SESSION_SEQUENCE_BITS)
        | sequence
}

macro_rules! id_u128 {
    ($name:ident) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        pub struct $name(u128);

        impl $name {
            pub const NIL: Self = Self(0);

            #[must_use]
            pub const fn new(value: u128) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn get(self) -> u128 {
                self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}({:032x})", stringify!($name), self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{:032x}", self.0)
            }
        }
    };
}

macro_rules! id_u64 {
    ($name:ident) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        pub struct $name(u64);

        impl $name {
            pub const NIL: Self = Self(0);

            #[must_use]
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}({})", stringify!($name), self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}", self.0)
            }
        }
    };
}

id_u128!(RequestId);
id_u128!(IntentId);
id_u128!(PlanId);
id_u128!(ActionId);
id_u128!(CheckpointId);
id_u128!(EvidenceId);
id_u128!(LeaseId);
id_u128!(EdgeId);
id_u128!(EventId);
id_u128!(ObjectiveId);
id_u128!(AttentionId);
id_u128!(AffordanceId);
id_u128!(RecommendationId);
id_u128!(SurpriseId);
id_u128!(MemoryId);
id_u128!(HandoffId);
id_u64!(FortressId);
id_u64!(EntityId);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct StepId(u32);

impl StepId {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Debug for StepId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "StepId({})", self.0)
    }
}

impl fmt::Display for StepId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_session_ids_preserve_their_raw_value() {
        assert_eq!(SessionId::new(7).get(), 7);
        assert!(!SessionId::new(7).is_process_scoped_live());
    }

    #[test]
    fn live_namespace_is_process_scoped_and_idempotently_parseable() {
        let raw = LIVE_SESSION_NAMESPACE | 41;
        let encoded = SessionId::new(raw);
        assert!(encoded.is_process_scoped_live());
        assert_eq!(encoded.get() & LIVE_SESSION_SEQUENCE_MASK, 41);
        assert_eq!(SessionId::new(encoded.get()), encoded);
    }

    #[test]
    fn consecutive_live_allocations_preserve_monotonic_order() {
        let first = SessionId::new(LIVE_SESSION_NAMESPACE | 10);
        let second = SessionId::new(LIVE_SESSION_NAMESPACE | 11);
        assert_eq!(second.get(), first.get() + 1);
    }

    #[test]
    fn different_process_scopes_cannot_alias_the_same_raw_live_id() {
        let raw = LIVE_SESSION_NAMESPACE | 99;
        let first = encode_process_scoped_live_session(1, raw);
        let second = encode_process_scoped_live_session(2, raw);
        assert_ne!(first, second);
        assert_eq!(first & LIVE_SESSION_SEQUENCE_MASK, 99);
        assert_eq!(second & LIVE_SESSION_SEQUENCE_MASK, 99);
    }
}
