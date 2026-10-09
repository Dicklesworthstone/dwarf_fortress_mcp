//! Partial-order exploration of two agents sharing one laboratory fortress.
//!
//! Every interleaving of two short tool-call scripts is enumerated and grouped
//! into Mazurkiewicz trace classes under a declared independence relation:
//! operations of one agent are ordered; across agents, only reads of the
//! shared world (`plan`, `query`) commute, while `commit` (world + leases) and
//! `wait` (the shared clock) conflict with everything that reads the world.
//! Classes are identified by Foata normal form.
//!
//! Executing two distinct linearizations of each class and comparing every
//! outcome and the final world tests the independence relation itself: if a
//! "commuting" pair did not commute, the class members would disagree. Each
//! representative also re-runs byte-identically (TEST-021 replay equality),
//! and the lease invariant is checked on every interleaving.
use super::*;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Plan([i32; 3], [i32; 3]),
    Commit,
    Wait(u64),
    Query,
}

impl Op {
    const fn reads_only(self) -> bool {
        matches!(self, Self::Plan(..) | Self::Query)
    }
}

/// An event is (agent, position in that agent's script).
type Event = (usize, usize);

fn dependent(scripts: &[Vec<Op>; 2], a: Event, b: Event) -> bool {
    a.0 == b.0 || !(scripts[a.0][a.1].reads_only() && scripts[b.0][b.1].reads_only())
}

/// Foata normal form: events grouped by causal level, each level sorted.
fn foata(scripts: &[Vec<Op>; 2], word: &[Event]) -> Vec<Vec<Event>> {
    let mut levels: Vec<usize> = Vec::with_capacity(word.len());
    for (i, event) in word.iter().enumerate() {
        let level = (0..i)
            .filter(|j| dependent(scripts, word[*j], *event))
            .map(|j| levels[j] + 1)
            .max()
            .unwrap_or(0);
        levels.push(level);
    }
    let depth = levels.iter().copied().max().map_or(0, |d| d + 1);
    let mut form = vec![Vec::new(); depth];
    for (event, level) in word.iter().zip(levels) {
        form[level].push(*event);
    }
    form.iter_mut().for_each(|level| level.sort_unstable());
    form
}

/// Every interleaving of two scripts, in lexicographic order of choices.
fn interleavings(n: usize, m: usize) -> Vec<Vec<Event>> {
    fn go(
        i: usize,
        j: usize,
        n: usize,
        m: usize,
        prefix: &mut Vec<Event>,
        out: &mut Vec<Vec<Event>>,
    ) {
        if i == n && j == m {
            out.push(prefix.clone());
            return;
        }
        if i < n {
            prefix.push((0, i));
            go(i + 1, j, n, m, prefix, out);
            prefix.pop();
        }
        if j < m {
            prefix.push((1, j));
            go(i, j + 1, n, m, prefix, out);
            prefix.pop();
        }
    }
    let mut out = Vec::new();
    go(0, 0, n, m, &mut Vec::new(), &mut out);
    out
}

static NEXT_SELECTOR: AtomicU64 = AtomicU64::new(0);

fn parsed(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or(Value::Null)
}

/// What one execution produced: each agent's (ok, error code) per op, and
/// the final world identity and designation count.
#[derive(Debug, PartialEq, Eq)]
struct Run {
    outcomes: [Vec<(bool, String)>; 2],
    commits_ok: [bool; 2],
    final_state: String,
    designations: u64,
}

fn execute(scripts: &[Vec<Op>; 2], word: &[Event]) -> std::result::Result<Run, String> {
    let selector = format!("76{:05}", NEXT_SELECTOR.fetch_add(1, Ordering::Relaxed));
    let caps: Vec<(String, String)> = [
        ("observe", "read_only"),
        ("query", "read_only"),
        ("plan", "reversible"),
        ("control_clock", "reversible"),
        ("checkpoint", "guarded"),
        ("designate", "guarded"),
    ]
    .iter()
    .map(|(c, r)| ((*c).to_owned(), (*r).to_owned()))
    .collect();
    let mut sessions = Vec::new();
    for scenario in [Some("starter_fortress".to_owned()), None] {
        let opened = parsed(&fortress_open_session(
            Some(false),
            Some(selector.clone()),
            Some(caps.clone()),
            None,
            Some(10_000),
            None,
            None,
            Some(8_192),
            None,
            scenario,
            Some(true),
            None,
        ));
        sessions.push(
            opened["session_id"]
                .as_str()
                .ok_or_else(|| format!("open failed: {opened}"))?
                .to_owned(),
        );
    }
    let fortress = selector
        .parse::<u64>()
        .map(dfmcp_core::FortressId::new)
        .map_err(|_| "selector is not numeric".to_owned())?;
    let run = drive(scripts, word, &sessions);
    crate::server::release_shared_world(fortress);
    run
}

fn drive(
    scripts: &[Vec<Op>; 2],
    word: &[Event],
    sessions: &[String],
) -> std::result::Result<Run, String> {
    let mut pending: [Option<String>; 2] = [None, None];
    let mut outcomes: [Vec<(bool, String)>; 2] = [Vec::new(), Vec::new()];
    let mut commits_ok = [false, false];
    for &(agent, index) in word {
        let session = Some(sessions[agent].clone());
        let response = match scripts[agent][index] {
            Op::Plan(min, max) => {
                let actions = json!([{"action": {"kind": "designate_dig", "min": min, "max": max, "mode": "mine"}}]);
                let planned = parsed(&fortress_plan(
                    session,
                    None,
                    None,
                    Some(actions.to_string()),
                    None,
                ));
                pending[agent] = planned["plan_digest"].as_str().map(str::to_owned);
                planned
            }
            Op::Commit => {
                let digest = pending[agent].clone().unwrap_or_else(|| "00".repeat(32));
                let committed = parsed(&fortress_commit(session, digest));
                commits_ok[agent] |= committed["ok"] == true;
                committed
            }
            Op::Wait(ticks) => parsed(&fortress_wait(session, Some(ticks))),
            Op::Query => parsed(&fortress_query(session, None)),
        };
        if response.is_null() {
            return Err("a tool returned non-JSON".to_owned());
        }
        outcomes[agent].push((
            response["ok"] == true,
            response["error"]["code"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        ));
    }
    // The world modulo fortress identity (each run is a fresh fortress):
    // game tick, terrain over the work area, and entity counts by kind.
    let observed = parsed(&fortress_observe(Some(sessions[0].clone())));
    let terrain = parsed(&fortress_query(
        Some(sessions[0].clone()),
        Some(r#"{"mode":"terrain","min":[0,0,10],"max":[9,6,10]}"#.to_owned()),
    ));
    let mut kinds = BTreeMap::new();
    for kind in ["unit", "dig_designation", "building", "work_order"] {
        let total = parsed(&fortress_query(
            Some(sessions[0].clone()),
            Some(json!({"mode": "entities", "kind": kind}).to_string()),
        ))["total"]
            .as_u64()
            .unwrap_or(u64::MAX);
        kinds.insert(kind, total);
    }
    Ok(Run {
        outcomes,
        commits_ok,
        final_state: json!({
            "game_tick": observed["agent_turn"]["anchor"]["game_tick"],
            "terrain": terrain["levels"],
            "entities": kinds,
        })
        .to_string(),
        designations: kinds["dig_designation"],
    })
}

struct Report {
    interleavings: usize,
    classes: usize,
    executions: usize,
}

/// Explores every class of `scripts`, checking `invariant` on every
/// interleaving and class agreement plus re-run determinism per class.
fn explore(
    scripts: &[Vec<Op>; 2],
    invariant: impl Fn(&Run) -> std::result::Result<(), String>,
) -> std::result::Result<Report, String> {
    let words = interleavings(scripts[0].len(), scripts[1].len());
    let mut classes: BTreeMap<Vec<Vec<Event>>, Vec<Vec<Event>>> = BTreeMap::new();
    for word in &words {
        classes
            .entry(foata(scripts, word))
            .or_default()
            .push(word.clone());
    }
    let mut executions = 0;
    for (form, members) in &classes {
        let first = execute(scripts, &members[0])?;
        executions += 1;
        invariant(&first).map_err(|why| format!("{why}: {:?} -> {first:?}", members[0]))?;
        // Replay equality: the same trace re-runs to the same world.
        let again = execute(scripts, &members[0])?;
        executions += 1;
        if again != first {
            return Err(format!(
                "trace {:?} is not deterministic: {first:?} vs {again:?}",
                members[0]
            ));
        }
        // Soundness of the independence relation: another linearization of
        // the same class must agree on every outcome and the final world.
        if let Some(other) = members.last().filter(|_| members.len() > 1) {
            let run = execute(scripts, other)?;
            executions += 1;
            invariant(&run).map_err(|why| format!("{why}: {other:?} -> {run:?}"))?;
            if run != first {
                return Err(format!(
                    "class {form:?} is not a trace class: {:?} -> {first:?} but {other:?} -> {run:?}",
                    members[0]
                ));
            }
        }
    }
    Ok(Report {
        interleavings: words.len(),
        classes: classes.len(),
        executions,
    })
}

#[test]
fn foata_classes_are_insertion_order_independent_and_reduce_the_space() {
    let scripts = [
        vec![Op::Plan([1, 3, 10], [2, 3, 10]), Op::Query, Op::Commit],
        vec![Op::Plan([5, 3, 10], [6, 3, 10]), Op::Query, Op::Commit],
    ];
    let words = interleavings(3, 3);
    assert_eq!(words.len(), 20);
    let classes: std::collections::BTreeSet<_> = words.iter().map(|w| foata(&scripts, w)).collect();
    // Reads commute, so all 3x3 read prefixes collapse; commits do not.
    assert!(classes.len() < words.len(), "{}", classes.len());
    // Both orders of two independent reads have one normal form.
    let ab = foata(&scripts, &[(0, 0), (1, 0), (0, 1), (0, 2), (1, 1), (1, 2)]);
    let ba = foata(&scripts, &[(1, 0), (0, 0), (0, 1), (0, 2), (1, 1), (1, 2)]);
    assert_eq!(ab, ba);
}

#[test]
fn overlapping_excavations_never_both_commit_in_any_interleaving() -> TestResult {
    let _serial = crate::test_serial();
    // Both agents want tile x=3; neither dig can finish within the run.
    let scripts = [
        vec![Op::Plan([0, 3, 10], [3, 4, 10]), Op::Commit, Op::Wait(30)],
        vec![Op::Plan([3, 3, 10], [6, 4, 10]), Op::Commit, Op::Wait(30)],
    ];
    let report = explore(&scripts, |run| {
        if run.commits_ok == [true, true] {
            return Err("two leases covered one tile".to_owned());
        }
        if !run.commits_ok.contains(&true) {
            return Err("neither agent could commit".to_owned());
        }
        if run.designations != 1 {
            return Err(format!(
                "{} designations for one committed dig",
                run.designations
            ));
        }
        Ok(())
    })?;
    assert_eq!(report.interleavings, 20);
    assert!(report.classes < report.interleavings);
    eprintln!(
        "overlap: {} interleavings, {} trace classes, {} executions",
        report.interleavings, report.classes, report.executions
    );
    Ok(())
}

#[test]
fn disjoint_excavations_always_both_commit_and_classes_agree() -> TestResult {
    let _serial = crate::test_serial();
    let scripts = [
        vec![
            Op::Query,
            Op::Plan([0, 3, 10], [1, 4, 10]),
            Op::Commit,
            Op::Wait(20),
        ],
        vec![
            Op::Plan([5, 3, 10], [6, 4, 10]),
            Op::Query,
            Op::Commit,
            Op::Wait(20),
        ],
    ];
    let report = explore(&scripts, |run| {
        if run.commits_ok != [true, true] {
            return Err("a disjoint dig was refused".to_owned());
        }
        if run.designations > 2 {
            return Err(format!("{} designations for two digs", run.designations));
        }
        Ok(())
    })?;
    assert_eq!(report.interleavings, 70);
    assert!(report.classes < report.interleavings);
    eprintln!(
        "disjoint: {} interleavings, {} trace classes, {} executions",
        report.interleavings, report.classes, report.executions
    );
    Ok(())
}

/// Write-skew probe (bead wji): adjacent digs each read the other's region
/// through the one-tile hazard halo but lease only their own tiles. Commit
/// revalidates every read, so a dig whose halo changed underneath it is
/// refused as stale (first committer wins) rather than committed on an
/// outdated read; when both commit, both rooms really are dug.
#[test]
fn adjacent_excavations_serialize_without_write_skew() -> TestResult {
    let _serial = crate::test_serial();
    let scripts = [
        vec![Op::Plan([0, 3, 10], [2, 4, 10]), Op::Commit, Op::Wait(200)],
        vec![Op::Plan([3, 3, 10], [5, 4, 10]), Op::Commit, Op::Wait(200)],
    ];
    let both_dug = ".".repeat(6);
    let mut refused = 0;
    let report = explore(&scripts, |run| {
        for (agent, outcomes) in run.outcomes.iter().enumerate() {
            let (ok, code) = &outcomes[1];
            if !ok && code != "stale_anchor" && code != "conflict" {
                return Err(format!("agent {agent} commit failed untyped: {code}"));
            }
        }
        if !run.commits_ok.contains(&true) {
            return Err("neither adjacent dig committed".to_owned());
        }
        if run.commits_ok == [true, true] && !run.final_state.contains(&both_dug) {
            return Err("both committed but the rooms were not both dug".to_owned());
        }
        Ok(())
    })?;
    for word in interleavings(3, 3) {
        refused += usize::from(execute(&scripts, &word)?.commits_ok != [true, true]);
    }
    eprintln!(
        "adjacent: {} interleavings, {} trace classes, {refused} with a stale read refused",
        report.interleavings, report.classes
    );
    Ok(())
}
