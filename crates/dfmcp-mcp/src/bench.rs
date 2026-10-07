//! Laboratory-scale SLO scorecard.
//!
//! Drives the eleven-tool facade on `starter_fortress` and the fault
//! campaigns, then scores every SLO-001..015 row as `pass`, `fail`, `partial`
//! (measured, but below the stated sample size) or `not_applicable` with a
//! reason. Wall-clock figures depend on the host and build profile; the
//! scorecard records both. Laboratory measurements are not live-game evidence.

use std::time::Instant;

use serde_json::{Value, json};

use crate::agent_facade as f;

/// Most measured iterations per latency row.
pub const MAX_ITERATIONS: u32 = 10_000;

fn parsed(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or(Value::Null)
}

fn tokens(raw: &str) -> u64 {
    (raw.len() as u64).div_ceil(crate::output_budget::BYTES_PER_TOKEN as u64)
}

/// Nearest-rank percentile of `samples` in microseconds.
fn percentile(samples: &mut [u128], p: f64) -> u128 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let rank = ((p * samples.len() as f64).ceil() as usize).clamp(1, samples.len());
    samples[rank - 1]
}

/// Latency samples (microseconds), output-token samples, and the last response.
fn timed(mut call: impl FnMut() -> String, iterations: u32) -> (Vec<u128>, Vec<u128>, String) {
    let mut samples = Vec::with_capacity(iterations as usize);
    let mut sizes = Vec::with_capacity(iterations as usize);
    let mut last = String::new();
    for _ in 0..iterations {
        let start = Instant::now();
        last = call();
        samples.push(start.elapsed().as_micros());
        sizes.push(u128::from(tokens(&last)));
    }
    (samples, sizes, last)
}

fn latency_row(slo: &str, what: &str, target_ms: u128, samples: &mut [u128]) -> Value {
    let p50 = percentile(samples, 0.50);
    let p99 = percentile(samples, 0.99);
    json!({
        "slo": slo, "measures": what, "target": format!("p99 <= {target_ms} ms"),
        "p50_us": p50, "p99_us": p99, "samples": samples.len(),
        "status": if p99 <= target_ms * 1_000 { "pass" } else { "fail" },
    })
}

/// Scored on the median response ("ordinary"); the largest is reported too.
fn token_row(slo: &str, what: &str, target: u64, sizes: &mut [u128]) -> Value {
    let median = percentile(sizes, 0.50);
    let max = percentile(sizes, 1.0);
    json!({
        "slo": slo, "measures": what, "target": format!("ordinary response <= {target} output tokens"),
        "median_tokens": median, "max_tokens": max, "estimator": "ceil(bytes/4)",
        "status": if median <= u128::from(target) { "pass" } else { "fail" },
    })
}

fn not_applicable(slo: &str, reason: &str) -> Value {
    json!({"slo": slo, "status": "not_applicable", "reason": reason})
}

fn rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|kib| kib.parse().ok())
}

/// Runs the scorecard with `iterations` samples per latency row.
#[must_use]
pub fn scorecard(iterations: u32, selector: &str) -> Value {
    let iterations = iterations.clamp(1, MAX_ITERATIONS);
    let caps: Vec<(String, String)> = [
        ("observe", "read_only"),
        ("query", "read_only"),
        ("plan", "reversible"),
        ("control_clock", "reversible"),
        ("checkpoint", "guarded"),
        ("restore", "guarded"),
        ("designate", "guarded"),
        ("doctor", "read_only"),
    ]
    .iter()
    .map(|(c, r)| ((*c).to_owned(), (*r).to_owned()))
    .collect();
    let opened = parsed(&f::fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(caps),
        None,
        Some(u64::from(iterations) * 100 + 10_000),
        None,
        None,
        Some(8_192),
        None,
        Some("starter_fortress".to_owned()),
        None,
        None,
    ));
    let Some(session) = opened["session_id"].as_str().map(str::to_owned) else {
        return json!({"ok": false, "error": "could not open the benchmark session", "opened": opened});
    };
    let s = || Some(session.clone());

    // A heartbeat is a one-tick pulse; a delta is a pulse over ten ticks.
    let (mut heartbeat, mut heartbeat_tokens, _) =
        timed(|| f::fortress_wait(s(), Some(1)), iterations);
    let (mut briefing, mut briefing_tokens, _) = timed(|| f::fortress_observe(s()), iterations);
    let (mut delta, mut delta_tokens, _) = timed(|| f::fortress_wait(s(), Some(10)), iterations);
    let (mut query, _, _) = timed(
        || {
            f::fortress_query(
                s(),
                Some(r#"{"mode":"entities","kind":"unit","limit":8}"#.to_owned()),
            )
        },
        iterations,
    );
    // A 10-step plan, well inside SLO-004's 64-step envelope.
    let steps: Vec<String> = (0..10)
        .map(|i| {
            format!(
                r#"{{"action":{{"kind":"designate_dig","min":[{x},3,10],"max":[{x},3,10],"mode":"mine"}}}}"#,
                x = i % 10
            )
        })
        .collect();
    let plan_actions = format!("[{}]", steps.join(","));
    let (mut plan, _, plan_raw) = timed(
        || f::fortress_plan(s(), None, None, Some(plan_actions.clone()), None),
        iterations,
    );

    // SLO-014: every mutating response names its plan/action or evidence.
    let digest = parsed(&plan_raw)["plan_digest"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let committed = parsed(&f::fortress_commit(s(), digest));
    let checkpoint = parsed(&f::fortress_checkpoint(s(), Some("bench".to_owned())));
    let restored = parsed(&f::fortress_restore(
        s(),
        checkpoint["checkpoint_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
    ));
    let identified = [&committed, &checkpoint, &restored].iter().all(|r| {
        r["ok"] == true
            && (r["plan_digest"].is_string()
                || r["action_id"].is_string()
                || r["checkpoint_id"].is_string()
                || r["evidence"].is_array()
                || r["restore_certificate"].is_object())
    });

    // SLO-008 / SLO-010: deterministic fault campaigns.
    let mut schedules = 0u64;
    let mut duplicate_or_diverged = 0u64;
    let clean = dfmcp_lab::FaultSchedule::new(0, [])
        .and_then(|schedule| dfmcp_lab::faults::run_publication_campaign(&schedule, 20));
    let publication = [
        dfmcp_lab::Boundary::PublicationReserve,
        dfmcp_lab::Boundary::PublicationMaterialize,
        dfmcp_lab::Boundary::PublicationPublish,
    ];
    for seed in 1..=u64::from(iterations.min(500)) {
        schedules += 1;
        let outcome = dfmcp_lab::FaultSchedule::seeded(seed, &publication, 20, 6)
            .and_then(|schedule| dfmcp_lab::faults::run_publication_campaign(&schedule, 20));
        let same = match (&clean, outcome) {
            (Ok(clean), Ok(run)) => {
                clean.final_anchor == run.final_anchor && run.completed == clean.completed
            }
            _ => false,
        };
        duplicate_or_diverged += u64::from(!same);
    }
    let journal_dir = std::env::temp_dir().join(format!("dfmcp-bench-{}", std::process::id()));
    let mut crash_points = 0u64;
    let mut recovery_failures = 0u64;
    for seed in 1..=u64::from(iterations.min(20)) {
        let _ = std::fs::remove_dir_all(&journal_dir);
        match dfmcp_lab::FaultSchedule::seeded(seed, &[dfmcp_lab::Boundary::JournalAppend], 16, 6)
            .and_then(|schedule| {
                dfmcp_lab::faults::run_journal_campaign(&journal_dir, &schedule, 10)
            }) {
            Ok(report) => crash_points += report.transcript.len() as u64,
            Err(_) => recovery_failures += 1,
        }
    }
    let _ = std::fs::remove_dir_all(&journal_dir);

    let briefing_p99 = percentile(&mut briefing, 0.99);
    let rows = vec![
        latency_row(
            "SLO-001",
            "fortress.wait(0) pulse heartbeat",
            10,
            &mut heartbeat,
        ),
        latency_row(
            "SLO-002",
            "fortress.wait(10) semantic delta",
            25,
            &mut delta,
        ),
        latency_row("SLO-003", "entities query", 50, &mut query),
        latency_row("SLO-004", "fortress.plan with 10 steps", 100, &mut plan),
        token_row(
            "SLO-005",
            "pulse heartbeat response",
            150,
            &mut heartbeat_tokens,
        ),
        token_row("SLO-006", "pulse delta response", 500, &mut delta_tokens),
        token_row(
            "SLO-007",
            "fortress.observe briefing",
            1_500,
            &mut briefing_tokens,
        ),
        json!({
            "slo": "SLO-008", "target": "no duplicate verified effect under 10,000 schedules",
            "schedules": schedules, "duplicate_or_diverged": duplicate_or_diverged,
            "status": if duplicate_or_diverged > 0 { "fail" } else if schedules >= 10_000 { "pass" } else { "partial" },
        }),
        not_applicable(
            "SLO-009",
            "the compatibility corpus is live bridge data; the lab has none",
        ),
        json!({
            "slo": "SLO-010", "target": "deterministic recovery across 1,000 injected crash points",
            "crash_points": crash_points, "recovery_failures": recovery_failures,
            "status": if recovery_failures > 0 { "fail" } else if crash_points >= 1_000 { "pass" } else { "partial" },
        }),
        match rss_kib() {
            Some(kib) => json!({
                "slo": "SLO-011", "target": "idle server <= 150 MiB RSS",
                "measured_mib": kib / 1024,
                "note": "process RSS after the benchmark, an upper bound on idle",
                "status": if kib <= 150 * 1024 { "pass" } else { "fail" },
            }),
            None => not_applicable("SLO-011", "RSS is read from /proc, absent on this host"),
        },
        json!({
            "slo": "SLO-012", "target": "canonical storage grows sublinearly with observations",
            "observations": u64::from(iterations) * 2,
            "status": "partial",
            "note": "lab sessions retain a bounded window plus live roots (retention tests prove the bound); durable journal compaction is tested separately",
        }),
        not_applicable(
            "SLO-013",
            "bridge payload limits are proven by the wire decoder tests, not this lab run",
        ),
        json!({
            "slo": "SLO-014", "target": "mutating responses carry plan/action identity or evidence",
            "status": if identified { "pass" } else { "fail" },
        }),
        not_applicable(
            "SLO-015",
            "doctor divergence is scored on the certified compatibility corpus",
        ),
    ];
    let failed = rows.iter().filter(|r| r["status"] == "fail").count();
    json!({
        "ok": failed == 0,
        "schema": "dfmcp.lab-slo-scorecard/1",
        "scope": "deterministic laboratory, starter_fortress; not live-game evidence",
        "server_version": env!("CARGO_PKG_VERSION"),
        "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "target": format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
        "iterations": iterations,
        "failed": failed,
        "briefing_p99_us": briefing_p99,
        "rows": rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scorecard_scores_every_slo_row() {
        let card = scorecard(5, "79001");
        let rows = card["rows"].as_array().cloned().unwrap_or_default();
        let ids: Vec<_> = rows.iter().filter_map(|r| r["slo"].as_str()).collect();
        let expected: Vec<String> = (1..=15).map(|n| format!("SLO-{n:03}")).collect();
        assert_eq!(ids, expected, "{card}");
        for row in &rows {
            let status = row["status"].as_str().unwrap_or_default();
            assert!(
                ["pass", "fail", "partial", "not_applicable"].contains(&status),
                "{row}"
            );
            if status == "not_applicable" {
                assert!(row["reason"].is_string(), "{row}");
            }
        }
        // Correctness rows never fail, whatever the host speed.
        for slo in ["SLO-008", "SLO-010", "SLO-014"] {
            let row = rows.iter().find(|r| r["slo"] == slo);
            assert!(row.is_some_and(|r| r["status"] != "fail"), "{slo}: {card}");
        }
    }
}
