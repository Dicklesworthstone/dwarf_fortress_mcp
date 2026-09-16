//! Operator-only durable monitoring for the citizen-inclusive spatial runtime.
//! The observation archive is opened/replayed first; a watch cannot outlive the
//! evidence universe that gives its entity generations and anchors meaning.

use std::path::{Path, PathBuf};
use super::*;

fn configured_path(value: Option<std::ffi::OsString>, observation_path: Option<&Path>)
    -> Result<Option<PathBuf>> {
    let Some(value) = value else { return Ok(None); };
    let text = value.to_str().filter(|text| !text.is_empty()).ok_or_else(|| {
        error(ErrorCode::InvalidRequest, "DFMCP_SPATIAL_CITIZEN_WATCH_JOURNAL must be nonempty UTF-8")
    })?;
    let observation_path = observation_path.ok_or_else(|| {
        error(ErrorCode::CapabilityDenied, "durable watches require DFMCP_SPATIAL_CITIZEN_JOURNAL; process-local observation generations cannot be recovered")
    })?;
    let path = PathBuf::from(text);
    if path == observation_path || !path.is_absolute() || path.as_os_str().len() > 4096
        || path.components().any(|part| !matches!(part, std::path::Component::RootDir | std::path::Component::Normal(_)))
        || path.file_name().is_none() {
        return Err(error(ErrorCode::InvalidRequest, "watch journal must be a separate absolute normalized operator-configured file"));
    }
    Ok(Some(path))
}

pub(super) fn configuration(observation_path: Option<&Path>) -> Result<Option<PathBuf>> {
    configured_path(std::env::var_os("DFMCP_SPATIAL_CITIZEN_WATCH_JOURNAL"), observation_path)
}

pub(super) fn finish_open(session: &mut Session, context: &OperationContext,
    path: Option<&Path>, value: Value) -> Result<String> {
    let Some(path) = path else {
        return packet(Some(session), Some(context), "fortress.open_session", value);
    };
    let journal = session.journal.as_ref().ok_or_else(|| {
        error(ErrorCode::CorruptLedger, "durable watch bootstrap has no observation archive")
    })?;
    if journal.fenced() || journal.profile() != "spatial/1.8"
        || journal.state().snapshot().map(|snapshot| snapshot.anchor()) != Some(context.anchor) {
        return Err(error(ErrorCode::CorruptLedger, "durable watch bootstrap requires the synced current spatial/1.8 anchor"));
    }
    let archive = journal.id();
    let observations: Vec<_> = journal.entries().iter().map(|entry| entry.anchor).collect();
    let snapshot = session.state.snapshot().ok_or_else(|| {
        error(ErrorCode::InternalInvariantViolation, "durable watch bootstrap snapshot absent")
    })?;
    let (output, owner) = semantic_query::attach_watch_journal(snapshot, context, path,
        archive, &observations, value, |value| {
            packet(Some(session), Some(context), "fortress.open_session", value)
        })?;
    session._watch_journal = Some(owner);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn watch_configuration_requires_separate_archive_and_absolute_path() {
        for value in ["", "relative.bin", "/private/../watches.bin", "/private/observations.bin"] {
            assert!(configured_path(Some(value.into()), Some(Path::new("/private/observations.bin"))).is_err());
        }
        assert!(configured_path(Some("/private/watches.bin".into()), None).is_err());
        assert_eq!(configured_path(None, None).ok(), Some(None));
        assert_eq!(configured_path(Some("/private/watches.bin".into()), Some(Path::new("/private/observations.bin"))).ok(),
            Some(Some(PathBuf::from("/private/watches.bin"))));
    }
    #[cfg(unix)]
    #[test]
    fn non_utf8_watch_path_is_refused_without_environment_mutation() {
        use std::os::unix::ffi::OsStringExt;
        let path = std::ffi::OsString::from_vec(vec![b'/', 0xff]);
        assert!(configured_path(Some(path), Some(Path::new("/private/observations.bin"))).is_err());
    }
}
