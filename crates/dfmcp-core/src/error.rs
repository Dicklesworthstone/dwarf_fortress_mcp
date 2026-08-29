use core::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ErrorCode {
    VersionMismatch,
    SessionNotFound,
    FortressNotLoaded,
    AdapterUnavailable,
    CursorGap,
    StaleAnchor,
    InvalidRequest,
    InvalidIntent,
    InvalidPlan,
    CapabilityDenied,
    RiskCeilingExceeded,
    BudgetExceeded,
    PreconditionsFailed,
    Conflict,
    LeaseDenied,
    CheckpointRequired,
    AdapterRejected,
    AdapterFailure,
    EffectIndeterminate,
    VerificationTimeout,
    CancellationRequested,
    CancellationIncomplete,
    RestoreRequired,
    CorruptLedger,
    CompatibilityUnknown,
    InternalInvariantViolation,
}

impl ErrorCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::VersionMismatch => "version_mismatch",
            Self::SessionNotFound => "session_not_found",
            Self::FortressNotLoaded => "fortress_not_loaded",
            Self::AdapterUnavailable => "adapter_unavailable",
            Self::CursorGap => "cursor_gap",
            Self::StaleAnchor => "stale_anchor",
            Self::InvalidRequest => "invalid_request",
            Self::InvalidIntent => "invalid_intent",
            Self::InvalidPlan => "invalid_plan",
            Self::CapabilityDenied => "capability_denied",
            Self::RiskCeilingExceeded => "risk_ceiling_exceeded",
            Self::BudgetExceeded => "budget_exceeded",
            Self::PreconditionsFailed => "preconditions_failed",
            Self::Conflict => "conflict",
            Self::LeaseDenied => "lease_denied",
            Self::CheckpointRequired => "checkpoint_required",
            Self::AdapterRejected => "adapter_rejected",
            Self::AdapterFailure => "adapter_failure",
            Self::EffectIndeterminate => "effect_indeterminate",
            Self::VerificationTimeout => "verification_timeout",
            Self::CancellationRequested => "cancellation_requested",
            Self::CancellationIncomplete => "cancellation_incomplete",
            Self::RestoreRequired => "restore_required",
            Self::CorruptLedger => "corrupt_ledger",
            Self::CompatibilityUnknown => "compatibility_unknown",
            Self::InternalInvariantViolation => "internal_invariant_violation",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DfmcpError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    pub details: Vec<(String, String)>,
}

impl DfmcpError {
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: false,
            details: Vec::new(),
        }
    }

    #[must_use]
    pub const fn retryable(mut self, value: bool) -> Self {
        self.retryable = value;
        self
    }

    #[must_use]
    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.details.push((key.into(), value.into()));
        self
    }
}

impl fmt::Display for DfmcpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: {}{}",
            self.code.as_str(),
            self.message,
            if self.retryable { " (retryable)" } else { "" }
        )
    }
}

impl std::error::Error for DfmcpError {}

pub type Result<T> = core::result::Result<T, DfmcpError>;
