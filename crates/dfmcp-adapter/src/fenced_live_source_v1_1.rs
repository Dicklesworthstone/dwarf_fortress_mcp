#![forbid(unsafe_code)]

//! Permanent transport fence for protocol-1.1 live reads.
//!
//! Once a DFHack call fails, the stream outcome may be ambiguous: bytes can
//! have been written while the reply was truncated or lost. Reusing that
//! transport would risk interpreting a later reply against the wrong request.
//! The wrapper therefore poisons on the first failed read and requires a fresh
//! negotiated session.

use dfmcp_core::{DfmcpError, ErrorCode, Result};

use crate::{BridgeManifest, LiveObservationSourceV1_1, ObservationPageV1_1};

const MAX_POISON_REASON_BYTES: usize = 512;

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn bounded_reason(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if output.len() >= MAX_POISON_REASON_BYTES {
            break;
        }
        let sanitized = if character.is_control() { ' ' } else { character };
        if output.len() + sanitized.len_utf8() > MAX_POISON_REASON_BYTES {
            break;
        }
        output.push(sanitized);
    }
    if output.is_empty() {
        "protocol-1.1 source failed without a printable reason".to_owned()
    } else {
        output
    }
}

#[derive(Debug)]
pub struct FencedLiveSourceV1_1<T> {
    source: T,
    poisoned_reason: Option<String>,
}

impl<T> FencedLiveSourceV1_1<T> {
    #[must_use]
    pub const fn new(source: T) -> Self {
        Self {
            source,
            poisoned_reason: None,
        }
    }

    #[must_use]
    pub const fn source(&self) -> &T {
        &self.source
    }

    #[must_use]
    pub const fn is_poisoned(&self) -> bool {
        self.poisoned_reason.is_some()
    }

    #[must_use]
    pub fn poisoned_reason(&self) -> Option<&str> {
        self.poisoned_reason.as_deref()
    }

    pub fn into_inner(self) -> Result<T> {
        if let Some(reason) = self.poisoned_reason {
            return Err(error(
                ErrorCode::PreconditionsFailed,
                format!(
                    "cannot extract a protocol-1.1 source after transport poisoning: {reason}"
                ),
            ));
        }
        Ok(self.source)
    }

    fn ensure_usable(&self) -> Result<()> {
        if let Some(reason) = self.poisoned_reason.as_deref() {
            return Err(error(
                ErrorCode::AdapterUnavailable,
                format!(
                    "protocol-1.1 live source is permanently fenced after failure: {reason}"
                ),
            ));
        }
        Ok(())
    }
}

impl<T: LiveObservationSourceV1_1> LiveObservationSourceV1_1 for FencedLiveSourceV1_1<T> {
    fn bridge_manifest_v1_1(&self) -> BridgeManifest {
        self.source.bridge_manifest_v1_1()
    }

    fn read_observation_page_v1_1(
        &mut self,
        offset: u32,
        maximum: u32,
        include_names: bool,
        announcement_after_id: i32,
        max_announcements: u32,
    ) -> Result<ObservationPageV1_1> {
        self.ensure_usable()?;
        match self.source.read_observation_page_v1_1(
            offset,
            maximum,
            include_names,
            announcement_after_id,
            max_announcements,
        ) {
            Ok(page) => Ok(page),
            Err(failure) => {
                self.poisoned_reason = Some(bounded_reason(&failure.message));
                Err(failure)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, VecDeque};

    use super::*;
    use crate::{AnnouncementContinuity, AnnouncementCoverage, LiveAnnouncementBatch};

    struct ScriptedSource {
        manifest: BridgeManifest,
        outcomes: VecDeque<Result<ObservationPageV1_1>>,
        calls: usize,
    }

    impl LiveObservationSourceV1_1 for ScriptedSource {
        fn bridge_manifest_v1_1(&self) -> BridgeManifest {
            self.manifest.clone()
        }

        fn read_observation_page_v1_1(
            &mut self,
            _offset: u32,
            _maximum: u32,
            _include_names: bool,
            _announcement_after_id: i32,
            _max_announcements: u32,
        ) -> Result<ObservationPageV1_1> {
            self.calls = self.calls.saturating_add(1);
            self.outcomes.pop_front().unwrap_or_else(|| {
                Err(error(
                    ErrorCode::AdapterFailure,
                    "scripted source exhausted",
                ))
            })
        }
    }

    fn manifest() -> BridgeManifest {
        BridgeManifest {
            bridge_version: "0.2.0".to_owned(),
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

    fn page() -> Result<ObservationPageV1_1> {
        Ok(ObservationPageV1_1 {
            bridge_generation: 42,
            world_loaded: true,
            fortress_mode: true,
            paused: true,
            current_year: 105,
            current_year_tick: 12_345,
            world_name: "The Balanced Realm".to_owned(),
            world_folder: "region1".to_owned(),
            site_id: 7,
            citizen_count_total: 0,
            citizen_offset: 0,
            complete: true,
            citizens: Vec::new(),
            announcement_batch: LiveAnnouncementBatch::new(
                42,
                true,
                105,
                12_345,
                7,
                AnnouncementCoverage {
                    requested_after_id: -1,
                    oldest_available_id: -1,
                    latest_available_id: -1,
                    returned: 0,
                    complete_through_latest: true,
                    continuity: AnnouncementContinuity::CompleteSuffix,
                    next_after_id: -1,
                },
                Vec::new(),
            )?,
        })
    }

    #[test]
    fn first_failure_permanently_fences_the_transport() {
        let source = ScriptedSource {
            manifest: manifest(),
            outcomes: VecDeque::from([
                Err(error(ErrorCode::AdapterFailure, "truncated reply")),
                page(),
            ]),
            calls: 0,
        };
        let mut fenced = FencedLiveSourceV1_1::new(source);
        assert!(
            fenced
                .read_observation_page_v1_1(0, 1, true, -1, 128)
                .is_err()
        );
        assert!(fenced.is_poisoned());
        assert!(
            fenced
                .read_observation_page_v1_1(0, 1, true, -1, 128)
                .is_err()
        );
        assert_eq!(fenced.source().calls, 1);
    }

    #[test]
    fn successful_source_remains_extractable() -> Result<()> {
        let source = ScriptedSource {
            manifest: manifest(),
            outcomes: VecDeque::from([page()]),
            calls: 0,
        };
        let mut fenced = FencedLiveSourceV1_1::new(source);
        let _page = fenced.read_observation_page_v1_1(0, 1, true, -1, 128)?;
        assert!(!fenced.is_poisoned());
        assert_eq!(fenced.into_inner()?.calls, 1);
        Ok(())
    }

    #[test]
    fn poison_reason_is_bounded_and_sanitized() {
        let hostile = format!("{}\n{}", "x".repeat(1_000), "secret-control");
        let source = ScriptedSource {
            manifest: manifest(),
            outcomes: VecDeque::from([Err(error(ErrorCode::AdapterFailure, hostile))]),
            calls: 0,
        };
        let mut fenced = FencedLiveSourceV1_1::new(source);
        let _failure = fenced.read_observation_page_v1_1(0, 1, true, -1, 128);
        let reason = fenced.poisoned_reason().unwrap_or("");
        assert!(reason.len() <= MAX_POISON_REASON_BYTES);
        assert!(!reason.chars().any(char::is_control));
    }
}
