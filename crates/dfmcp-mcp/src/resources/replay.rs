//! Export the exact input accepted by the replay CLI. Resource transport
//! metadata belongs outside the closed bundle schema.

use super::{denial, inspect_session, task_resource_bytes, text_content};
use crate::server::{authorize_entry, read_context, resource_replay_bundle};
use dfmcp_core::{Capability, Digest32, RiskTier};
use fastmcp_rust::prelude::{McpError, McpResult, ResourceContent};
use serde_json::json;
use std::time::{Duration, Instant};

const OPERATION: &str = "df://session/replay";
const MAX_PAGE_BYTES: usize = 16 * 1024;

fn cursor(view: &str) -> McpResult<Option<(String, usize)>> {
    if view == "replay" {
        return Ok(None);
    }
    let invalid = || McpError::invalid_params("invalid replay digest or byte offset");
    let (digest, raw) = view
        .strip_prefix("replay-")
        .and_then(|v| v.split_once('~'))
        .ok_or_else(invalid)?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid());
    }
    let offset = raw.parse::<usize>().map_err(|_| invalid())?;
    if offset.to_string() != raw {
        return Err(invalid());
    }
    Ok(Some((digest.to_owned(), offset)))
}

pub(super) fn read(session: &str, uri: &str, view: &str) -> McpResult<Vec<ResourceContent>> {
    let started = Instant::now();
    if uri.len() > 192 || uri != format!("df://session/{session}/{view}") {
        return Err(McpError::invalid_params(
            "replay URI must match its session and view",
        ));
    }
    let cursor = cursor(view)?;
    inspect_session(session, OPERATION, |guard| {
        authorize_entry(
            &read_context(guard),
            Capability::Observe,
            RiskTier::ReadOnly,
        )
        .map_err(|error| denial(OPERATION, error))?;
        let budget = guard.budget;
        let check_wall = || {
            if started.elapsed() >= Duration::from_millis(budget.max_wall_millis) {
                return Err(McpError::invalid_request(
                    "budget_exceeded: replay export exceeded its wall budget",
                ));
            }
            Ok(())
        };
        check_wall()?;
        guard
            .replay
            .admit_export(budget, started)
            .map_err(|error| denial(OPERATION, error))?;
        let bundle = resource_replay_bundle(guard).to_string();
        check_wall()?;
        if bundle.len() as u64 > budget.max_bytes || bundle.len() > crate::replay::MAX_BUNDLE_BYTES
        {
            return Err(McpError::invalid_request(
                "budget_exceeded: complete replay document exceeds the source byte budget",
            ));
        }
        let maximum = budget.max_bytes.min(
            u64::from(budget.max_output_tokens) * crate::output_budget::BYTES_PER_TOKEN as u64,
        );
        let fits = |text: &str| {
            text.len() as u64 <= maximum && task_resource_bytes(uri, text) as u64 <= maximum
        };
        if cursor.is_none() && fits(&bundle) {
            check_wall()?;
            return Ok(text_content(uri, bundle));
        }
        let digest = Digest32::of_bytes(bundle.as_bytes()).to_hex();
        let mut offset = 0;
        if let Some((expected, start)) = &cursor {
            if expected != &digest {
                return Err(McpError::invalid_request(
                    "cursor_gap: replay transcript changed; restart at the session replay resource",
                ));
            }
            offset = *start;
        }
        if offset >= bundle.len() || !bundle.is_char_boundary(offset) {
            return Err(McpError::invalid_params(
                "replay offset must begin a character inside the exact document",
            ));
        }
        let first = bundle[offset..]
            .chars()
            .next()
            .ok_or_else(|| McpError::invalid_params("empty replay page"))?
            .len_utf8();
        let mut end = (offset + MAX_PAGE_BYTES).min(bundle.len());
        loop {
            while !bundle.is_char_boundary(end) {
                end -= 1;
            }
            let next = (end < bundle.len())
                .then(|| format!("df://session/{session}/replay-{digest}~{end}"));
            let page = json!({
                "schema":"dfmcp.replay.page/1", "digest":digest, "encoding":"json_utf8",
                "total_bytes":bundle.len(), "offset":offset, "part":&bundle[offset..end],
                "complete":next.is_none(), "next":next,
            })
            .to_string();
            check_wall()?;
            if fits(&page) {
                check_wall()?;
                return Ok(text_content(uri, page));
            }
            if end == offset + first {
                return Err(McpError::invalid_request(
                    "budget_exceeded: even the smallest replay page exceeds the negotiated output allowance",
                ));
            }
            end = offset + ((end - offset) / 2).max(first);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_facade as facade;
    use serde_json::Value;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    fn open(selector: &str, tokens: u32) -> TestResult<String> {
        let value: Value = serde_json::from_str(&facade::fortress_open_session(
            Some(true),
            Some(selector.to_owned()),
            None,
            Some(60_000),
            None,
            None,
            None,
            Some(tokens),
            None,
            None,
            None,
            None,
        ))?;
        assert_eq!(value["ok"], true, "{value}");
        Ok(value["session_id"]
            .as_str()
            .ok_or("missing session")?
            .to_owned())
    }

    fn fetch(session: &str, uri: &str) -> McpResult<String> {
        let view = uri
            .rsplit('/')
            .next()
            .ok_or_else(|| McpError::invalid_params("missing view"))?;
        read(session, uri, view)?
            .into_iter()
            .next()
            .and_then(|item| item.text)
            .ok_or_else(|| McpError::invalid_params("missing replay text"))
    }

    #[test]
    fn inline_resource_is_an_exact_cli_bundle() -> TestResult {
        let session = open("7893702", 8_192)?;
        let uri = format!("df://session/{session}/replay");
        let text = fetch(&session, &uri)?;
        let bundle: Value = serde_json::from_str(&text)?;
        assert_eq!(bundle, crate::server::replay_bundle_for(&session)?);
        let report = crate::replay::replay_bundle(&bundle);
        assert_eq!(report["ok"], true, "{report}");
        Ok(())
    }

    #[test]
    fn paged_export_replays_and_rechecks_digest_and_authority() -> TestResult {
        let session = open("7893703", 1_500)?;
        for _ in 0..12 {
            let _ = facade::fortress_observe(Some(session.clone()));
        }
        let root = format!("df://session/{session}/replay");
        let mut uri = root.clone();
        let mut document = String::new();
        let expected = crate::server::replay_bundle_for(&session)?.to_string();
        let digest = Digest32::of_bytes(expected.as_bytes()).to_hex();
        let mut pages = 0;
        loop {
            let text = fetch(&session, &uri)?;
            let page: Value = serde_json::from_str(&text)?;
            assert_eq!(page["schema"], "dfmcp.replay.page/1");
            assert!(task_resource_bytes(&uri, &text) <= 6_000);
            assert_eq!(page["digest"], digest);
            assert_eq!(page["offset"].as_u64(), Some(document.len() as u64));
            assert_eq!(page["total_bytes"].as_u64(), Some(expected.len() as u64));
            let part = page["part"].as_str().ok_or("missing replay bytes")?;
            assert!(!part.is_empty());
            document.push_str(part);
            pages += 1;
            if page["next"].is_null() {
                assert_eq!(page["complete"], true);
                break;
            }
            uri = page["next"]
                .as_str()
                .ok_or("missing continuation")?
                .to_owned();
        }
        assert!(pages > 1);
        assert_eq!(document, expected);
        let report = crate::replay::replay_bundle(&serde_json::from_str(&document)?);
        assert_eq!(report["ok"], true, "{report}");
        let _ = facade::fortress_observe(Some(session.clone()));
        let stale = fetch(&session, &uri).expect_err("transcript changed");
        assert!(stale.to_string().contains("cursor_gap"), "{stale}");
        let owner = crate::server::lookup_session_str(&session)?;
        owner.lock().map_err(|_| "session lock")?.grants.clear();
        let denied = fetch(&session, &uri).expect_err("revoked Observe");
        assert!(denied.to_string().contains("capability_denied"), "{denied}");
        Ok(())
    }
}
