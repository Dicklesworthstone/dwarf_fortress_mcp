use super::*;
use crate::build_placement::journal::{BuildJournal, BuildMode};
use crate::build_placement::{BuildItem, BuildPlan, BuildRecord};
use crate::control_effect_journal::EffectJournalStorage;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, GameTick, ObservationCursor, OperationContext,
    RequestId, RiskTier, SessionId, StateAnchor, WorkBudget,
};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;

const GOLDEN_INPUT: &[u8] = br#"{"steps":[{"name":"z","kind":"table","item":44,"target":[20,15,2],"after":["a","b"]},{"name":"b","kind":"chair","item":43,"target":[18,15,2],"after":["a"]},{"name":"a","kind":"bed","item":42,"target":[15,15,2]}],"schema":"dfmcp.furniture-plan/1"}"#;

fn document(steps: &[String]) -> String {
    format!(
        "{{\"schema\":\"{SCHEMA}\",\"steps\":[{}]}}",
        steps.join(",")
    )
}
fn step(name: &str, item: u32, x: u32, after: &[String]) -> String {
    let dependencies = after
        .iter()
        .map(|name| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"name\":\"{name}\",\"kind\":\"bed\",\"item\":{item},\"target\":[{x},15,2],\"after\":[{dependencies}]}}"
    )
}
fn plan(count: usize) -> Result<FurniturePlan> {
    let steps = (0..count)
        .map(|index| {
            step(
                &format!("step-{index:02}"),
                42 + index as u32,
                15 + index as u32,
                &[],
            )
        })
        .collect::<Vec<_>>();
    FurniturePlan::decode(document(&steps).as_bytes())
}

#[test]
fn canonical_plan_matches_independent_python_digest_and_normalization() -> Result<()> {
    let value = FurniturePlan::decode(GOLDEN_INPUT)?;
    // Produced by the existing scripts/furniture_plan.py with hashlib, not this codec.
    assert_eq!(
        value.digest().to_string(),
        "17b6972689f9b20c30023e643360865fc6439999a7e630dc43989f5b9d6d6c44"
    );
    let canonical = br#"{"schema":"dfmcp.furniture-plan/1","steps":[{"after":[],"item":42,"kind":"bed","name":"a","target":[15,15,2]},{"after":["a"],"item":43,"kind":"chair","name":"b","target":[18,15,2]},{"after":["a","b"],"item":44,"kind":"table","name":"z","target":[20,15,2]}]}"#;
    assert_eq!(value.canonical_bytes(), canonical);
    assert_eq!(FurniturePlan::decode(canonical)?, value);
    assert_eq!(
        value
            .ordered_steps()
            .map(|step| step.name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "z"]
    );
    let escaped = String::from_utf8(canonical.to_vec())
        .map_err(|_| invalid("fixture UTF-8"))?
        .replace("\"schema\"", "\"sch\\u0065ma\"")
        .replace("furniture-plan/1", "furniture-plan\\/1")
        .replace("\"a\"", "\"\\u0061\"");
    assert_eq!(FurniturePlan::decode(escaped.as_bytes())?, value);
    Ok(())
}

#[test]
fn closed_parser_rejects_substitutions_duplicates_types_and_bounds() -> Result<()> {
    let base = document(&[step("a", 42, 15, &[])]);
    let cases = [
        base.replace(
            "\"schema\":",
            "\"schema\":\"dfmcp.furniture-plan/1\",\"schema\":",
        ),
        base.replace("\"name\":\"a\"", "\"name\":\"a\",\"na\\u006de\":\"a\""),
        base.replace("\"item\":42", "\"item\":42,\"extra\":0"),
        base.replace("\"item\":42", "\"item\":42,\"item\":43"),
        base.replace("\"item\":42", "\"item\":null"),
        base.replace("\"item\":42", "\"item\":true"),
        base.replace("\"item\":42", "\"item\":\"42\""),
        base.replace("\"item\":42", "\"item\":42.0"),
        base.replace("\"item\":42", "\"item\":42e0"),
        base.replace("\"item\":42", "\"item\":042"),
        base.replace("\"item\":42", "\"item\":-1"),
        base.replace("\"item\":42", "\"item\":4294967296"),
        base.replace("\"item\":42", "\"item\":2147483647"),
        base.replace("\"after\":[]", "\"after\":null"),
        base.replace("\"after\":[]", "\"after\":[\"a\"]"),
        base.replace("\"after\":[]", "\"after\":[\"other\"]"),
        base.replace("\"name\":\"a\"", "\"name\":\"\""),
        base.replace("\"name\":\"a\"", "\"name\":\"bad name\""),
        base.replace("\"name\":\"a\"", "\"name\":\"\\u00e9\""),
        base.replace("\"name\":\"a\"", "\"name\":\"\\u0000\""),
        base.replace("\"bed\"", "\"door\""),
        base.replace("[15,15,2]", "[15,15,2,0]"),
        base.replace("[15,15,2]", "[0,15,2]"),
        base.replace("[15,15,2]", "[32767,15,2]"),
        base.replace("[15,15,2]", "[15,15,32768]"),
        base.replace("[15,15,2]", "[15,15,2.0]"),
        base.replace("[15,15,2]", "[15,15]"),
        base.replace("\"item\":42,", ""),
        base.replace("\"after\":[]", "\"after\":[[[[[[[[[0]]]]]]]]]"),
        format!("{base},"),
        base.replace("}]}", "},]}"),
        base.replace("\"after\":[]}", "\"after\":[],}"),
    ];
    for raw in cases {
        assert!(FurniturePlan::decode(raw.as_bytes()).is_err(), "{raw}");
    }
    assert!(FurniturePlan::decode(b"{}").is_err());
    assert!(FurniturePlan::decode(document(&[]).as_bytes()).is_err());
    assert!(FurniturePlan::decode(&vec![b' '; MAX_PLAN_BYTES + 1]).is_err());
    assert!(FurniturePlan::decode(&[0xff]).is_err());
    let duplicate = step("a", 42, 15, &[]);
    assert!(FurniturePlan::decode(document(&[duplicate.clone(), duplicate]).as_bytes()).is_err());
    assert!(
        FurniturePlan::decode(
            document(&[step("a", 42, 15, &[]), step("b", 42, 16, &[])]).as_bytes()
        )
        .is_err()
    );
    assert!(
        FurniturePlan::decode(
            document(&[step("a", 42, 15, &[]), step("b", 43, 15, &[])]).as_bytes()
        )
        .is_err()
    );
    assert!(
        FurniturePlan::decode(
            document(&[
                step("a", 42, 15, &[]),
                step("b", 43, 16, &["a".to_owned(), "a".to_owned()])
            ])
            .as_bytes()
        )
        .is_err()
    );
    assert!(plan(33).is_err());
    assert!(
        FurniturePlan::decode(document(&[step(&"x".repeat(49), 42, 15, &[])]).as_bytes()).is_err()
    );
    let maximum = FurniturePlan::decode(
        document(&[step(&"x".repeat(48), 2_147_483_646, 32_766, &[])]).as_bytes(),
    )?;
    assert_eq!(maximum.steps()[0].selection.item_id(), 2_147_483_646);
    // Every cut through a valid complete object remains invalid.
    for end in 0..base.len() {
        assert!(FurniturePlan::decode(&base.as_bytes()[..end]).is_err());
    }
    Ok(())
}

fn permutations(values: &mut [usize], start: usize, out: &mut Vec<Vec<usize>>) {
    if start == values.len() {
        out.push(values.to_vec());
        return;
    }
    for index in start..values.len() {
        values.swap(start, index);
        permutations(values, start + 1, out);
        values.swap(start, index);
    }
}
#[test]
fn all_4096_four_node_graphs_match_independent_permutation_oracle() -> Result<()> {
    let mut orders = Vec::new();
    permutations(&mut [0, 1, 2, 3], 0, &mut orders);
    orders.sort();
    let edges = (0..4)
        .flat_map(|from| {
            (0..4)
                .filter(move |to| *to != from)
                .map(move |to| (from, to))
        })
        .collect::<Vec<_>>();
    let names = ["a", "b", "c", "d"];
    for mask in 0u32..4096 {
        let selected = edges
            .iter()
            .enumerate()
            .filter(|(bit, _)| mask & (1 << bit) != 0)
            .map(|(_, edge)| *edge)
            .collect::<Vec<_>>();
        let oracle = orders.iter().find(|order| {
            selected.iter().all(|(from, to)| {
                order.iter().position(|node| node == from)
                    < order.iter().position(|node| node == to)
            })
        });
        let steps = (0..4)
            .rev()
            .map(|node| {
                let after = selected
                    .iter()
                    .filter(|(_, to)| *to == node)
                    .map(|(from, _)| names[*from].to_owned())
                    .collect::<Vec<_>>();
                step(names[node], 42 + node as u32, 15 + node as u32, &after)
            })
            .collect::<Vec<_>>();
        let decoded = FurniturePlan::decode(document(&steps).as_bytes());
        match oracle {
            None => assert!(decoded.is_err(), "cycle {mask}"),
            Some(order) => {
                let actual = decoded?
                    .ordered_steps()
                    .map(|step| step.name.clone())
                    .collect::<Vec<_>>();
                let expected = order
                    .iter()
                    .map(|node| names[*node].to_owned())
                    .collect::<Vec<_>>();
                assert_eq!(actual, expected, "graph {mask}");
            }
        }
    }
    Ok(())
}

fn fixture(name: &str) -> Result<Vec<u8>> {
    let source = include_str!("../../../bridge/common/tests/fixtures/build_placement_v1_19.json");
    let prefix = format!("\"{name}\": \"");
    let value = source
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))
        .and_then(|value| value.split('"').next())
        .ok_or_else(|| invalid("native fixture absent"))?;
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            u8::from_str_radix(
                std::str::from_utf8(pair).map_err(|_| invalid("fixture encoding"))?,
                16,
            )
            .map_err(|_| invalid("fixture hex"))
        })
        .collect()
}
fn capture(index: usize) -> Result<BuildCapture> {
    let mut raw = fixture("capture")?;
    let selection = BuildSelection::new(
        BuildKind::Bed,
        42 + index as u32,
        [15 + index as u32, 15, 2],
    )?;
    // Frozen independent native fixture offsets: source, clock, horizons, selection.
    raw[16..24].copy_from_slice(&(index as u64).to_be_bytes());
    raw[24..32].copy_from_slice(&(806_500 + index as u64).to_be_bytes());
    raw[48..52].copy_from_slice(&(70 + index as u32).to_be_bytes());
    raw[52..56].copy_from_slice(&(90 + index as u32).to_be_bytes());
    raw[56..60].copy_from_slice(&(4 + index as u32).to_be_bytes());
    raw[72..89].copy_from_slice(&selection.canonical_bytes());
    BuildCapture::decode(&raw)
}
fn binding() -> Result<BuildBinding> {
    BuildBinding::new(
        SocketAddr::from(([127, 0, 0, 1], 5000)),
        "df",
        "dfhack",
        &capture(0)?,
    )
}
fn append_field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}
fn header(binding: &BuildBinding) -> (Vec<u8>, Digest32) {
    let mut raw = b"DFMBJ019".to_vec();
    append_field(&mut raw, &binding.encode());
    raw.extend_from_slice(&[7; 32]);
    let id = hash(b"dfmcp-build-journal/1", &raw);
    raw.extend_from_slice(id.as_bytes());
    (raw, id)
}
fn definition(count: usize) -> Result<BatchDefinition> {
    let binding = binding()?;
    let (_, id) = header(&binding);
    BatchDefinition::new(plan(count)?, binding, id)
}

#[test]
fn batch_definition_seals_every_original_boundary_and_checks_all_targets() -> Result<()> {
    let value = definition(32)?;
    let raw = value.canonical_bytes();
    assert!(raw.len() <= MAX_DEFINITION_BYTES);
    assert_eq!(BatchDefinition::decode(&raw)?, value);
    for end in 0..raw.len() {
        assert!(BatchDefinition::decode(&raw[..end]).is_err());
    }
    let mut trailing = raw.clone();
    trailing.push(0);
    assert!(BatchDefinition::decode(&trailing).is_err());
    let different_journal = BatchDefinition::new(
        value.plan().clone(),
        value.binding().clone(),
        Digest32::of_bytes(b"other"),
    )?;
    assert_ne!(different_journal.id(), value.id());
    let other_endpoint = BuildBinding::new(
        SocketAddr::from(([127, 0, 0, 1], 5001)),
        "df",
        "dfhack",
        &capture(0)?,
    )?;
    assert_ne!(
        BatchDefinition::new(value.plan().clone(), other_endpoint, value.journal_id())?.id(),
        value.id()
    );
    let other_software = BuildBinding::new(
        SocketAddr::from(([127, 0, 0, 1], 5000)),
        "other-df",
        "dfhack",
        &capture(0)?,
    )?;
    assert_ne!(
        BatchDefinition::new(value.plan().clone(), other_software, value.journal_id())?.id(),
        value.id()
    );
    let reordered_dependencies = FurniturePlan::decode(
        document(&[
            step("step-00", 42, 15, &[]),
            step("step-01", 43, 16, &["step-00".into()]),
        ])
        .as_bytes(),
    )?;
    assert_ne!(
        BatchDefinition::new(reordered_dependencies, binding()?, value.journal_id())?.id(),
        definition(2)?.id()
    );
    let late_invalid = FurniturePlan::decode(
        document(&[step("a", 42, 15, &[]), step("z", 43, 63, &[])]).as_bytes(),
    )?;
    assert!(late_invalid.validate_binding(value.binding()).is_err());
    let maximum_name =
        FurniturePlan::decode(document(&[step(&"x".repeat(48), 42, 15, &[])]).as_bytes())?;
    let named = BatchDefinition::new(maximum_name, binding()?, value.journal_id())?;
    assert_eq!(named.key(&named.plan().steps()[0]).len(), 116);
    assert!(BatchDefinition::new(plan(1)?, binding()?, Digest32::ZERO).is_err());
    // Even valid JSON must already be normalized inside persisted definition bytes.
    let mut noncanonical = DEFINITION_MAGIC.to_vec();
    append_field(&mut noncanonical, GOLDEN_INPUT);
    append_field(&mut noncanonical, &binding()?.encode());
    noncanonical.extend_from_slice(value.journal_id().as_bytes());
    assert!(BatchDefinition::decode(&noncanonical).is_err());
    Ok(())
}

fn record(plan: &BuildPlan, phase: BuildPhase) -> Result<BuildRecord> {
    fn native_field(out: &mut Vec<u8>, bytes: &[u8]) {
        out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        out.extend_from_slice(bytes);
    }
    let mut raw = b"DFMBR019".to_vec();
    native_field(&mut raw, plan.key().as_bytes());
    native_field(&mut raw, plan.before().canonical_bytes());
    raw.extend_from_slice(plan.digest().as_bytes());
    raw.extend_from_slice(plan.token());
    let reason = match phase {
        BuildPhase::Prepared | BuildPhase::Placed => 0,
        BuildPhase::Indeterminate => 5,
        BuildPhase::Refused => 2,
        BuildPhase::Cancelled => 4,
    };
    raw.extend_from_slice(&[
        phase as u8,
        reason,
        u8::from(matches!(
            phase,
            BuildPhase::Placed | BuildPhase::Indeterminate
        )),
        u8::from(phase == BuildPhase::Placed),
    ]);
    if phase == BuildPhase::Placed {
        let before = plan.before();
        native_field(&mut raw, before.expected_after()?.canonical_bytes());
        let BuildItem::Visible(item) = before.item() else {
            return Err(invalid("fixture item unavailable"));
        };
        let mut insertion = b"DFMBI019".to_vec();
        for number in [
            before.next_building_id(),
            before.next_job_id(),
            before.selection().item_id(),
        ] {
            insertion.extend_from_slice(&number.to_be_bytes());
        }
        insertion.push(before.selection().kind() as u8);
        for number in before.selection().target() {
            insertion.extend_from_slice(&number.to_be_bytes());
        }
        for number in [item.material(), item.material_index(), 0, 1] {
            insertion.extend_from_slice(&number.to_be_bytes());
        }
        insertion.extend_from_slice(&[1, 1, 1, 0]);
        native_field(&mut raw, &insertion);
    }
    raw.extend_from_slice(hash(b"dfmcp-build-receipt/1", &raw).as_bytes());
    BuildRecord::decode(&raw)
}

struct ReadOnly(Cursor<Vec<u8>>);
impl Read for ReadOnly {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.0.read(out)
    }
}
impl Seek for ReadOnly {
    fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
        self.0.seek(offset)
    }
}
impl Write for ReadOnly {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("offline write"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("offline flush"))
    }
}
impl EffectJournalStorage for ReadOnly {
    fn sync(&mut self) -> io::Result<()> {
        Err(io::Error::other("offline sync"))
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("offline truncate"))
    }
}
fn context() -> Result<OperationContext> {
    let source = capture(0)?;
    Ok(OperationContext {
        session_id: SessionId::new(5),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: source.fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(source.tick()),
            state_hash: source.witness(),
        },
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: 64 * 1024 * 1024,
            max_entities: 256,
            ..WorkBudget::CONSERVATIVE_DEFAULT
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            max_risk: RiskTier::ReadOnly,
            scope: CapabilityScope {
                fortress_id: Some(source.fortress_id()),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    })
}
fn add_frame(
    raw: &mut Vec<u8>,
    sequence: &mut u64,
    head: &mut Digest32,
    plan: &BuildPlan,
    state: BuildState,
    native: Option<&BuildRecord>,
) {
    let mut body = vec![
        state as u8,
        u8::from(state == BuildState::DispatchStarted),
        u8::from(state == BuildState::CancelRequested),
    ];
    append_field(&mut body, &plan.canonical_bytes());
    append_field(&mut body, native.map_or(&[], BuildRecord::canonical_bytes));
    let mut frame = b"DFMBJF19".to_vec();
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    *sequence += 1;
    frame.extend_from_slice(&sequence.to_be_bytes());
    frame.extend_from_slice(head.as_bytes());
    frame.extend_from_slice(&body);
    *head = hash(b"dfmcp-build-journal-frame/1", &frame);
    frame.extend_from_slice(head.as_bytes());
    frame.extend_from_slice(b"DFMBJEND");
    raw.extend_from_slice(&frame);
}
fn inventory(
    definition: &BatchDefinition,
    entries: &[(BuildPlan, BuildState, Option<BuildPhase>)],
) -> Result<BuildInventory> {
    let (mut raw, mut head) = header(definition.binding());
    let mut sequence = 0;
    for (plan, state, phase) in entries {
        add_frame(
            &mut raw,
            &mut sequence,
            &mut head,
            plan,
            BuildState::Intent,
            None,
        );
        if *state == BuildState::Intent {
            continue;
        }
        let native = phase.map(|phase| record(plan, phase)).transpose()?;
        if *state == BuildState::DispatchStarted {
            add_frame(
                &mut raw,
                &mut sequence,
                &mut head,
                plan,
                BuildState::Prepared,
                native.as_ref(),
            );
        }
        add_frame(
            &mut raw,
            &mut sequence,
            &mut head,
            plan,
            *state,
            native.as_ref(),
        );
    }
    let context = context()?;
    BuildJournal::open(
        ReadOnly(Cursor::new(raw)),
        &context,
        BuildMode::Offline,
        Some(definition.binding().clone()),
        None,
    )?
    .inventory(&context)
}
fn child(definition: &BatchDefinition, index: usize) -> Result<BuildPlan> {
    BuildPlan::new(
        &definition.key(&definition.plan().steps()[index]),
        capture(index)?,
    )
}

#[test]
fn independent_native_record_builder_preserves_unchanged_golden_vectors() -> Result<()> {
    let plan = BuildPlan::new("golden", capture(0)?)?;
    for (name, phase) in [
        ("prepared", BuildPhase::Prepared),
        ("placed", BuildPhase::Placed),
        ("indeterminate", BuildPhase::Indeterminate),
        ("expired", BuildPhase::Refused),
        ("cancelled", BuildPhase::Cancelled),
    ] {
        assert_eq!(record(&plan, phase)?.canonical_bytes(), fixture(name)?);
    }
    Ok(())
}

#[test]
fn exact_32_step_batch_keeps_every_row_and_unlocks_only_verified_prefixes() -> Result<()> {
    let definition = definition(32)?;
    let mut entries = Vec::new();
    for next in 0..=32 {
        let view = inventory(&definition, &entries)?;
        let progress = definition.audit(&view, false)?;
        assert_eq!(progress.rows.len(), 32);
        assert_eq!(progress.placed, next);
        assert_eq!(progress.total, 32);
        assert_eq!(progress.journal_id, view.journal_id);
        assert_eq!(progress.head, view.head);
        assert_eq!(progress.pending_step, None);
        assert_eq!(
            progress.status,
            if next == 32 { "all_placed" } else { "ready" }
        );
        let stopped = definition.audit(&view, true)?;
        assert_eq!(
            stopped.status,
            if next == 32 { "all_placed" } else { "stopped" }
        );
        assert_eq!(stopped.next_step, None);
        if next == 32 {
            assert_eq!(progress.next_step, None);
            break;
        }
        let step = &definition.plan().steps()[next];
        assert_eq!(progress.next_step.as_deref(), Some(step.name.as_str()));
        definition.validate_next(
            &view,
            false,
            &definition.key(step),
            step.selection,
            Some(&capture(next)?),
        )?;
        assert!(
            definition
                .validate_next(&view, true, &definition.key(step), step.selection, None)
                .is_err()
        );
        entries.push((
            child(&definition, next)?,
            BuildState::Terminal,
            Some(BuildPhase::Placed),
        ));
    }
    Ok(())
}

#[test]
fn every_nonplaced_outcome_blocks_all_successors_even_after_stop() -> Result<()> {
    let definition = definition(3)?;
    let first = (
        child(&definition, 0)?,
        BuildState::Terminal,
        Some(BuildPhase::Placed),
    );
    let cases = [
        (BuildState::Intent, None, "pending_recovery"),
        (
            BuildState::Prepared,
            Some(BuildPhase::Prepared),
            "pending_recovery",
        ),
        (
            BuildState::DispatchStarted,
            Some(BuildPhase::Prepared),
            "pending_recovery",
        ),
        (
            BuildState::Tracking,
            Some(BuildPhase::Prepared),
            "pending_recovery",
        ),
        (BuildState::CancelRequested, None, "pending_recovery"),
        (
            BuildState::Terminal,
            Some(BuildPhase::Indeterminate),
            "pending_recovery",
        ),
        (
            BuildState::Terminal,
            Some(BuildPhase::Refused),
            "halted_refused",
        ),
        (
            BuildState::Terminal,
            Some(BuildPhase::Cancelled),
            "halted_cancelled",
        ),
    ];
    for (state, phase, expected) in cases {
        let rows = [first.clone(), (child(&definition, 1)?, state, phase)];
        let view = inventory(&definition, &rows)?;
        for stopped in [false, true] {
            let progress = definition.audit(&view, stopped)?;
            assert_eq!(progress.status, expected);
            assert_eq!(progress.placed, 1);
            assert_eq!(progress.next_step, None);
            assert_eq!(progress.stopped, stopped);
            assert_eq!(
                progress.pending_step.as_deref(),
                (expected == "pending_recovery").then_some("step-01")
            );
            assert_eq!(progress.rows[2].state, "not_started");
        }
        if matches!(
            state,
            BuildState::Tracking | BuildState::CancelRequested | BuildState::Terminal
        ) {
            let step = &definition.plan().steps()[1];
            assert!(
                definition
                    .validate_next(&view, false, &definition.key(step), step.selection, None)
                    .is_err()
            );
        }
    }
    Ok(())
}

#[test]
fn audit_rejects_foreign_keys_wrong_selections_missing_prefix_and_halted_bypass() -> Result<()> {
    let definition = definition(3)?;
    let cases = vec![
        vec![(
            BuildPlan::new("foreign", capture(0)?)?,
            BuildState::Intent,
            None,
        )],
        vec![(
            child(&definition, 1)?,
            BuildState::Terminal,
            Some(BuildPhase::Placed),
        )],
        vec![(
            BuildPlan::new(&definition.key(&definition.plan().steps()[0]), capture(1)?)?,
            BuildState::Intent,
            None,
        )],
        vec![
            (
                child(&definition, 0)?,
                BuildState::Terminal,
                Some(BuildPhase::Refused),
            ),
            (
                child(&definition, 1)?,
                BuildState::Terminal,
                Some(BuildPhase::Placed),
            ),
        ],
        vec![
            (
                child(&definition, 0)?,
                BuildState::Terminal,
                Some(BuildPhase::Cancelled),
            ),
            (child(&definition, 1)?, BuildState::Intent, None),
        ],
    ];
    for entries in cases {
        let view = inventory(&definition, &entries)?;
        assert!(definition.audit(&view, false).is_err());
    }
    let mut empty = inventory(&definition, &[])?;
    empty.journal_id = Digest32::of_bytes(b"substituted journal");
    assert!(definition.audit(&empty, false).is_err());
    // A different binding remains refused even with an internally valid journal.
    let mut source = capture(0)?.canonical_bytes().to_vec();
    source[8..16].copy_from_slice(&42u64.to_be_bytes());
    let changed = BuildBinding::new(
        SocketAddr::from(([127, 0, 0, 1], 5000)),
        "df",
        "dfhack",
        &BuildCapture::decode(&source)?,
    )?;
    let different = BatchDefinition::new(plan(3)?, changed, definition.journal_id())?;
    assert!(
        different
            .audit(
                &inventory(
                    &definition,
                    &[(child(&definition, 0)?, BuildState::Intent, None)]
                )?,
                false
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn next_child_rechecks_all_predecessor_horizons_and_exact_pending_capture() -> Result<()> {
    let definition = definition(2)?;
    let first = (
        child(&definition, 0)?,
        BuildState::Terminal,
        Some(BuildPhase::Placed),
    );
    let view = inventory(&definition, std::slice::from_ref(&first))?;
    let second = &definition.plan().steps()[1];
    let key = definition.key(second);
    for (start, bytes) in [
        (24, 806_499u64.to_be_bytes().to_vec()),
        (16, 0u64.to_be_bytes().to_vec()),
        (48, 70u32.to_be_bytes().to_vec()),
        (52, 90u32.to_be_bytes().to_vec()),
        (8, 42u64.to_be_bytes().to_vec()),
    ] {
        let mut raw = capture(1)?.canonical_bytes().to_vec();
        raw[start..start + bytes.len()].copy_from_slice(&bytes);
        assert!(
            definition
                .validate_next(
                    &view,
                    false,
                    &key,
                    second.selection,
                    Some(&BuildCapture::decode(&raw)?)
                )
                .is_err()
        );
    }
    let wrong = BuildSelection::new(BuildKind::Bed, 44, second.selection.target())?;
    assert!(
        definition
            .validate_next(&view, false, &key, wrong, None)
            .is_err()
    );
    assert!(
        definition
            .validate_next(&view, false, "foreign", second.selection, None)
            .is_err()
    );
    let prepared = inventory(
        &definition,
        &[
            first,
            (
                child(&definition, 1)?,
                BuildState::Prepared,
                Some(BuildPhase::Prepared),
            ),
        ],
    )?;
    definition.validate_next(&prepared, false, &key, second.selection, Some(&capture(1)?))?;
    let mut changed = capture(1)?.canonical_bytes().to_vec();
    changed[24..32].copy_from_slice(&806_502u64.to_be_bytes());
    assert!(
        definition
            .validate_next(
                &prepared,
                false,
                &key,
                second.selection,
                Some(&BuildCapture::decode(&changed)?)
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn audit_checks_native_id_horizons_that_original_journal_does_not_order() -> Result<()> {
    let definition = definition(2)?;
    for range in [48..52, 52..56] {
        let mut raw = capture(1)?.canonical_bytes().to_vec();
        raw[range.clone()].copy_from_slice(&capture(0)?.canonical_bytes()[range]);
        let second = BuildPlan::new(
            &definition.key(&definition.plan().steps()[1]),
            BuildCapture::decode(&raw)?,
        )?;
        let view = inventory(
            &definition,
            &[
                (
                    child(&definition, 0)?,
                    BuildState::Terminal,
                    Some(BuildPhase::Placed),
                ),
                (second, BuildState::Intent, None),
            ],
        )?;
        assert!(definition.audit(&view, false).is_err());
    }
    Ok(())
}
