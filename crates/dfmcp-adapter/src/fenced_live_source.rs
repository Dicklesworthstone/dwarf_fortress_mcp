#![forbid(unsafe_code)]

//! Fail-closed lifecycle fence for long-lived live observation sources.
//!
//! A failed DFHack call can be a clean semantic rejection, a complete but
//! malformed reply, or a transport failure that leaves framing uncertain. The
//! generic [`LiveObservationSource`] contract cannot distinguish those cases.
//! Reusing the stream optimistically would let a later plausible-looking frame
//! be interpreted against unknown framing or bridge state. This wrapper takes
//! the conservative rule: one failed page read poisons the source permanently;
//! the owning region must close it and negotiate a fresh connection.

use dfmcp_core::{DfmcpError, ErrorCode, Result};

use crate::{BridgeManifest, LiveObservationSource, ObservationPage};

const MAX_POISON_REASON_BYTES: usize = 4_096;

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn bounded_utf8_prefix(value: &str, maximum: usize) -> String {
    if value.len() <= maximum {
        return value.to_owned();
    }
    let mut boundary = 0usize;
    for (index, character) in value.char_indices() {
        let next = index.saturating_add(character.len_utf8());
        if next > maximum {
            break;
        }
        boundary = next;
    }
    value[..boundary].to_owned()
}

pub struct FencedLiveSource<T> {
    source: T,
    manifest: BridgeManifest,
    poisoned_reason: Option<String>,
}

impl<T: LiveObservationSource> FencedLiveSource<T> {
    pub fn new(source: T) -> Result<Self> {
        let manifest = source.bridge_manifest();
        manifest.validate()?;
        Ok(Self {
            source,
            manifest,
            poisoned_reason: None,
        })
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

    pub fn into_inner(self) -> T {
        self.source
    }

    fn record_failure(&mut self, failure: &DfmcpError) {
        let rendered = format!("{}: {}", failure.code.as_str(), failure.message);
        self.poisoned_reason = Some(bounded_utf8_prefix(
            &rendered,
            MAX_POISON_REASON_BYTES,
        ));
    }
}

impl<T: LiveObservationSource> LiveObservationSource for FencedLiveSource<T> {
    fn bridge_manifest(&self) -> BridgeManifest {
        self.manifest.clone()
    }

    fn read_observation_page(
        &mut self,
        offset: u32,
        maximum: u32,
        include_names: bool,
    ) -> Result<ObservationPage> {
        if let Some(reason) = self.poisoned_reason.as_ref() {
            return Err(error(
                ErrorCode::AdapterUnavailable,
                "live observation source is poisoned; negotiate a fresh bridge connection",
            )
            .with_detail("poisoned_by", reason.clone()));
        }
        match self
            .source
            .read_observation_page(offset, maximum, include_names)
        {
            Ok(page) => Ok(page),
            Err(failure) => {
                self.record_failure(&failure);
                Err(failure)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, VecDeque};

    use super::*;
    use crate::CitizenRecord;

    struct ScriptedSource {
        manifest: BridgeManifest,
        replies: VecDeque<Result<ObservationPage>>,
        calls: usize,
    }

    impl LiveObservationSource for ScriptedSource {
        fn bridge_manifest(&self) -> BridgeManifest {
            self.manifest.clone()
        }

        fn read_observation_page(
            &mut self,
            _offset: u32,
            _maximum: u32,
            _include_names: bool,
        ) -> Result<ObservationPage> {
            self.calls = self.calls.saturating_add(1);
            self.replies.pop_front().ok_or_else(|| {
                error(
                    ErrorCode::AdapterFailure,
                    "scripted source exhausted its replies",
                )
            })?
        }
    }

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

    fn page() -> ObservationPage {
        ObservationPage {
            bridge_generation: 42,
            world_loaded: true,
            fortress_mode: true,
            paused: true,
            current_year: 105,
            current_year_tick: 12_345,
            world_name: "The Balanced Realm".to_owned(),
            world_folder: "region1".to_owned(),
            site_id: 7,
            citizen_count_total: 1,
            citizen_offset: 0,
            complete: true,
            citizens: vec![CitizenRecord {
                unit_id: 7,
                name: "Urist".to_owned(),
                race: "dwarf".to_owned(),
                profession: 4,
                x: 1,
                y: 2,
                z: 3,
                alive: true,
                sane: true,
                active: true,
                visible: true,
                citizen: true,
                resident: false,
                baby: false,
                child: false,
                adult: true,
            }],
        }
    }

    #[test]
    fn one_failure_permanently_fences_the_source() -> Result<()> {
        let scripted = ScriptedSource {
            manifest: manifest(),
            replies: VecDeque::from([
                Err(error(ErrorCode::AdapterFailure, "truncated reply")),
                Ok(page()),
            ]),
            calls: 0,
        };
        let mut source = FencedLiveSource::new(scripted)?;
        assert!(source.read_observation_page(0, 1, true).is_err());
        assert!(source.is_poisoned());
        assert_eq!(source.poisoned_reason(), Some("adapter_failure: truncated reply"));
        assert!(source.read_observation_page(0, 1, true).is_err());
        assert_eq!(source.source().calls, 1);
        Ok(())
    }

    #[test]
    fn successful_source_remains_usable() -> Result<()> {
        let scripted = ScriptedSource {
            manifest: manifest(),
            replies: VecDeque::from([Ok(page()), Ok(page())]),
            calls: 0,
        };
        let mut source = FencedLiveSource::new(scripted)?;
        assert!(source.read_observation_page(0, 1, true).is_ok());
        assert!(source.read_observation_page(0, 1, true).is_ok());
        assert!(!source.is_poisoned());
        assert_eq!(source.source().calls, 2);
        Ok(())
    }

    #[test]
    fn poison_reason_truncation_preserves_utf8() {
        let value = "é".repeat(MAX_POISON_REASON_BYTES);
        let bounded = bounded_utf8_prefix(&value, MAX_POISON_REASON_BYTES);
        assert!(bounded.len() <= MAX_POISON_REASON_BYTES);
        assert!(bounded.is_char_boundary(bounded.len()));
    }
}
