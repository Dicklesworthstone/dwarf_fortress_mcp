#![forbid(unsafe_code)]

//! Explicit compatibility policy for the authenticated live bridge.
//!
//! Transport negotiation proves only that both sides speak the bridge wire.
//! Canonical observation additionally requires a policy that admits the exact
//! Dwarf Fortress, DFHack, bridge implementation, protocol, and method set.

use std::collections::BTreeSet;

use dfmcp_core::{DfmcpError, ErrorCode, Result};

use crate::{
    BRIDGE_PROTOCOL_MAJOR, BRIDGE_PROTOCOL_MINOR, BridgeManifest, CompatibilityLevel,
};

const MAX_VERSION_BYTES: usize = 128;
const MAX_METHODS: usize = 32;
const MAX_METHOD_BYTES: usize = 128;

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn validate_token(value: &str, field: &str, maximum: usize) -> Result<()> {
    if value.is_empty() || value.len() > maximum {
        return Err(error(
            ErrorCode::InvalidRequest,
            format!("{field} must contain 1..={maximum} bytes"),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveCompatibilityPolicy {
    pub dwarf_fortress_version: String,
    pub dfhack_version: String,
    pub bridge_version: String,
    pub protocol_major: u32,
    pub protocol_minor: u32,
    pub required_methods: BTreeSet<String>,
    pub allow_version_mismatch_for_diagnostics: bool,
}

impl LiveCompatibilityPolicy {
    pub fn exact(
        dwarf_fortress_version: impl Into<String>,
        dfhack_version: impl Into<String>,
        bridge_version: impl Into<String>,
    ) -> Result<Self> {
        let value = Self {
            dwarf_fortress_version: dwarf_fortress_version.into(),
            dfhack_version: dfhack_version.into(),
            bridge_version: bridge_version.into(),
            protocol_major: BRIDGE_PROTOCOL_MAJOR,
            protocol_minor: BRIDGE_PROTOCOL_MINOR,
            required_methods: BTreeSet::from([
                "Handshake".to_owned(),
                "ReadObservation".to_owned(),
            ]),
            allow_version_mismatch_for_diagnostics: false,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<()> {
        validate_token(
            &self.dwarf_fortress_version,
            "Dwarf Fortress version",
            MAX_VERSION_BYTES,
        )?;
        validate_token(&self.dfhack_version, "DFHack version", MAX_VERSION_BYTES)?;
        validate_token(&self.bridge_version, "bridge version", MAX_VERSION_BYTES)?;
        if self.protocol_major != BRIDGE_PROTOCOL_MAJOR
            || self.protocol_minor != BRIDGE_PROTOCOL_MINOR
        {
            return Err(error(
                ErrorCode::VersionMismatch,
                format!(
                    "compatibility policy protocol {}.{} differs from compiled bridge protocol {}.{}",
                    self.protocol_major,
                    self.protocol_minor,
                    BRIDGE_PROTOCOL_MAJOR,
                    BRIDGE_PROTOCOL_MINOR
                ),
            ));
        }
        if self.required_methods.is_empty() || self.required_methods.len() > MAX_METHODS {
            return Err(error(
                ErrorCode::InvalidRequest,
                format!(
                    "compatibility policy must contain 1..={MAX_METHODS} required methods"
                ),
            ));
        }
        for method in &self.required_methods {
            validate_token(method, "required bridge method", MAX_METHOD_BYTES)?;
        }
        Ok(())
    }

    pub fn evaluate(&self, manifest: &BridgeManifest) -> Result<LiveCompatibilityVerdict> {
        self.validate()?;
        if manifest.supported_methods != self.required_methods {
            return Ok(LiveCompatibilityVerdict {
                level: CompatibilityLevel::Incompatible,
                canonical_observation_allowed: false,
                reasons: vec![format!(
                    "bridge method set {:?} differs from required {:?}",
                    manifest.supported_methods, self.required_methods
                )],
            });
        }
        if manifest.bridge_version != self.bridge_version {
            return Ok(LiveCompatibilityVerdict {
                level: CompatibilityLevel::Incompatible,
                canonical_observation_allowed: false,
                reasons: vec![format!(
                    "bridge version {:?} differs from required {:?}",
                    manifest.bridge_version, self.bridge_version
                )],
            });
        }

        let mut reasons = Vec::new();
        if manifest.df_version != self.dwarf_fortress_version {
            reasons.push(format!(
                "Dwarf Fortress version {:?} differs from required {:?}",
                manifest.df_version, self.dwarf_fortress_version
            ));
        }
        if manifest.dfhack_version != self.dfhack_version {
            reasons.push(format!(
                "DFHack version {:?} differs from required {:?}",
                manifest.dfhack_version, self.dfhack_version
            ));
        }
        if reasons.is_empty() {
            return Ok(LiveCompatibilityVerdict {
                level: CompatibilityLevel::Exact,
                canonical_observation_allowed: true,
                reasons,
            });
        }
        Ok(LiveCompatibilityVerdict {
            level: CompatibilityLevel::DegradedReadOnly,
            canonical_observation_allowed: self.allow_version_mismatch_for_diagnostics,
            reasons,
        })
    }

    pub fn require_canonical_observation(
        &self,
        manifest: &BridgeManifest,
    ) -> Result<LiveCompatibilityVerdict> {
        let verdict = self.evaluate(manifest)?;
        if !verdict.canonical_observation_allowed {
            return Err(error(
                ErrorCode::CompatibilityUnknown,
                format!(
                    "live manifest is not admitted for canonical observation: {}",
                    verdict.reasons.join("; ")
                ),
            ));
        }
        Ok(verdict)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveCompatibilityVerdict {
    pub level: CompatibilityLevel,
    pub canonical_observation_allowed: bool,
    pub reasons: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> BridgeManifest {
        BridgeManifest {
            bridge_version: "0.1.0".to_owned(),
            dfhack_version: "0.51.11-r1".to_owned(),
            df_version: "0.51.11".to_owned(),
            world_loaded: true,
            fortress_mode: true,
            bridge_generation: 42,
            supported_methods: BTreeSet::from([
                "Handshake".to_owned(),
                "ReadObservation".to_owned(),
            ]),
        }
    }

    #[test]
    fn exact_manifest_is_admitted() -> Result<()> {
        let policy = LiveCompatibilityPolicy::exact(
            "0.51.11",
            "0.51.11-r1",
            "0.1.0",
        )?;
        let verdict = policy.require_canonical_observation(&manifest())?;
        assert_eq!(verdict.level, CompatibilityLevel::Exact);
        assert!(verdict.canonical_observation_allowed);
        Ok(())
    }

    #[test]
    fn unknown_game_version_fails_closed_by_default() -> Result<()> {
        let policy = LiveCompatibilityPolicy::exact(
            "0.51.12",
            "0.51.11-r1",
            "0.1.0",
        )?;
        assert!(policy.require_canonical_observation(&manifest()).is_err());
        let verdict = policy.evaluate(&manifest())?;
        assert_eq!(verdict.level, CompatibilityLevel::DegradedReadOnly);
        assert!(!verdict.canonical_observation_allowed);
        Ok(())
    }

    #[test]
    fn bridge_or_method_mismatch_is_incompatible_even_for_diagnostics() -> Result<()> {
        let mut policy = LiveCompatibilityPolicy::exact(
            "0.51.11",
            "0.51.11-r1",
            "0.2.0",
        )?;
        policy.allow_version_mismatch_for_diagnostics = true;
        let verdict = policy.evaluate(&manifest())?;
        assert_eq!(verdict.level, CompatibilityLevel::Incompatible);
        assert!(!verdict.canonical_observation_allowed);

        policy.bridge_version = "0.1.0".to_owned();
        policy.required_methods.insert("Mutate".to_owned());
        let verdict = policy.evaluate(&manifest())?;
        assert_eq!(verdict.level, CompatibilityLevel::Incompatible);
        assert!(!verdict.canonical_observation_allowed);
        Ok(())
    }
}
