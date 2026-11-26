//! Parent-held original and source-frontier checks before child custody opens.
//!
//! This is recovery metadata only. It restores no preparation, current sample,
//! worker choice, inventory proof, clock permission or replacement original.
use super::super::project::ProjectStockGoal;
use super::*;
use serde_json::Value;

const SCHEMA: &str = "dfmcp.project-production-recovery/1";
const MAX_REFERENCE_BYTES: usize = 32 * 1024;
const INTENT_SCHEMA: &str = "dfmcp.project-production-intent/1";

/// An exact prospective original, acknowledged by the parent before the child
/// starts its local publication. This is never a current proof or a permit.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectProductionIntent {
    schema: String,
    goal: ProjectStockGoal,
    recovery_reference: Value,
    input_policy_witness: Option<String>,
}

impl ProjectProductionIntent {
    pub(crate) fn new(
        goal: ProjectStockGoal,
        recovery_reference: Value,
        input_policy_witness: Option<String>,
    ) -> Result<Self> {
        Self::from_json(&serde_json::json!({
            "schema": INTENT_SCHEMA,
            "goal": goal,
            "recovery_reference": recovery_reference,
            "input_policy_witness": input_policy_witness,
        }))
    }

    pub(crate) fn from_json(value: &Value) -> Result<Self> {
        let bytes = serde_json::to_vec(value)
            .map_err(|_| invalid("project publication intent is not serializable"))?;
        if bytes.len() > MAX_REFERENCE_BYTES {
            return Err(invalid(
                "project publication intent exceeds its compact bound",
            ));
        }
        let intent: Self = serde_json::from_slice(&bytes)
            .map_err(|_| invalid("invalid closed project publication intent"))?;
        if intent.schema != INTENT_SCHEMA {
            return Err(invalid("invalid project publication intent schema"));
        }
        intent.goal.validate()?;
        let expected = retained(&intent.recovery_reference)?;
        if let Some(watch) = &expected.stock_watch {
            watch.validate_goal(&intent.goal)?;
            watch.require_unfinished()?;
        }
        let recovery = reference_recovery(&intent.recovery_reference)?;
        validate_reference_sources(&expected, recovery)?;
        if expected.portfolio_clocks.as_ref().is_some_and(|slots| {
            slots.iter().any(|slot| {
                [slot.clock.as_ref(), slot.inventory_clock.as_ref()]
                    .into_iter()
                    .flatten()
                    .any(|pin| pin.head != pin.journal_id)
            })
        }) {
            return Err(denied(
                "prospective portfolio clocks must retain their exact unexecuted headers",
            ));
        }
        if recovery.goal_history_present
            || intent.goal.is_portfolio() != recovery.portfolio.is_some()
            || recovery.portfolio.is_some_and(|slots| {
                slots[0].is_none_or(|first| first.step_id != u32::from(intent.goal.assign_labor()))
                    || slots
                        .into_iter()
                        .flatten()
                        .any(|slot| slot.goal_history_present)
            })
            || recovery.frontier.head != recovery.frontier.journal_id
            || recovery.anchor.tick.get() >= intent.goal.deadline_tick()
            || expected.workshop_prerequisite.is_none()
            || expected
                .workshop_prerequisite
                .as_ref()
                .and_then(|s| s.checkpoint)
                .is_none_or(|checkpoint| checkpoint.floor.fenced)
            || recovery
                .manager_prerequisite_floor
                .is_some_and(|floor| floor.fenced)
            || recovery
                .input_supply_floor
                .is_some_and(|floor| floor.fenced)
            || intent.input_policy_witness.is_some() != expected.input_supply.is_some()
        {
            return Err(denied(
                "project intent must describe one qualified, unexecuted exact original",
            ));
        }
        intent
            .input_policy_witness
            .as_deref()
            .map(digest)
            .transpose()?;
        Ok(intent)
    }

    pub(crate) fn to_json(&self) -> Value {
        serde_json::json!({
            "schema": self.schema,
            "goal": self.goal,
            "recovery_reference": self.recovery_reference,
            "input_policy_witness": self.input_policy_witness,
        })
    }

    pub(crate) fn authority_tick_floor(&self) -> u64 {
        // Construction checked this field and the immutable Value is private.
        self.recovery_reference["observation_tick_floor"]
            .as_u64()
            .unwrap_or(u64::MAX)
    }

    pub(crate) fn validate_goal(&self, goal: &ProjectStockGoal) -> Result<()> {
        goal.validate()?;
        if goal != &self.goal {
            return Err(denied(
                "publication intent changed the immutable original project goal",
            ));
        }
        Ok(())
    }

    /// Exact new-slot identities for the currently publishing original.
    /// The caller may use these only after its parent ACK callback succeeds;
    /// recovered originals always reopen with their ordinary retained pins.
    pub(in crate::live_production_runtime) fn portfolio_clocks_for_publication(
        &self,
        original: &dfmcp_intent::PreparedPlan,
    ) -> Result<Option<[Option<ProjectPortfolioClockRecovery>; 4]>> {
        let reference = retained(&self.recovery_reference)?;
        let recovery = reference_recovery(&self.recovery_reference)?;
        if recovery.seal.plan_digest != original.digest || recovery.anchor != original.anchor {
            return Err(denied(
                "project clock publication differs from its acknowledged original",
            ));
        }
        let clocks = portfolio_clock_pins(&reference, recovery)?;
        if clocks.as_ref().is_some_and(|slots| {
            slots.iter().skip(1).flatten().any(|slot| {
                [slot.clock, slot.inventory_clock]
                    .into_iter()
                    .flatten()
                    .any(|pin| {
                        pin.head != pin.journal_id
                            || pin.authority_tick_floor != 0
                            || pin.high_source_sequence != 0
                    })
            })
        }) {
            return Err(denied(
                "new project clock slots require untouched prospective headers",
            ));
        }
        Ok(clocks)
    }

    pub(crate) fn validate_initial_frontier(&self, frontier: durable::Recovery) -> Result<()> {
        let expected = reference_recovery(&self.recovery_reference)?.frontier;
        if frontier.journal_id != expected.journal_id
            || frontier.head != expected.head
            || frontier.authority_tick_floor < expected.authority_tick_floor
        {
            return Err(denied(
                "unacknowledged project original has advanced beyond its exact initial frontier",
            ));
        }
        Ok(())
    }

    /// Pending publication can recover only the exact, still unexecuted
    /// original. Per-order history is checked even for an inactive portfolio
    /// leaf; no selected-leaf summary can hide already started goal custody.
    pub(crate) fn validate_pending_checkpoint(&self, checkpoint: &Value) -> Result<()> {
        self.validate_checkpoint(checkpoint)?;
        let actual = retained(checkpoint)?;
        let expected = retained(&self.recovery_reference)?;
        // Pending publication can adopt initialized headers, never a clock
        // prepare or dispatch that advanced before the parent acknowledged it.
        compare_clock_sources(actual.clock.as_ref(), expected.clock.as_ref(), false)?;
        compare_clock_sources(
            actual.inventory_clock.as_ref(),
            expected.inventory_clock.as_ref(),
            false,
        )?;
        compare_portfolio_clock_sources(&actual, &expected, false)?;
        self.validate_pending_recovery(reference_recovery(checkpoint)?)
    }

    fn validate_pending_recovery(&self, current: Recovery) -> Result<()> {
        self.validate_initial_frontier(current.frontier)?;
        let expected = retained(&self.recovery_reference)?;
        let prior = expected
            .recovery
            .as_ref()
            .ok_or_else(|| invalid("project intent original is absent"))?;
        original(current, prior, expected.observation_tick_floor)?;
        if current.goal_history_present
            || current.portfolio.is_some_and(|slots| {
                slots
                    .into_iter()
                    .flatten()
                    .any(|slot| slot.goal_history_present)
            })
        {
            return Err(denied(
                "unacknowledged project original already has goal history",
            ));
        }
        Ok(())
    }

    /// Native custody and original identities remain fixed. Before the child
    /// publishes, the parent can retain either older qualified floors or newer
    /// failed-call rejection floors; recovery joins those floors with the
    /// immutable intent. After publication every original floor is monotone.
    pub(crate) fn validate_checkpoint(&self, checkpoint: &Value) -> Result<()> {
        super::super::project_stock_watch::verify_intent_watch(
            &self.recovery_reference,
            checkpoint,
        )?;
        let expected = retained(&self.recovery_reference)?;
        let actual = retained(checkpoint)?;
        same_digest(
            digest(&actual.native_custody.workforce_journal_id)?,
            &expected.native_custody.workforce_journal_id,
        )?;
        same_digest(
            digest(&actual.native_custody.order_journal_id)?,
            &expected.native_custody.order_journal_id,
        )?;
        let planned = actual.recovery.is_some();
        compare_clock_sources(actual.clock.as_ref(), expected.clock.as_ref(), planned)?;
        compare_clock_sources(
            actual.inventory_clock.as_ref(),
            expected.inventory_clock.as_ref(),
            planned,
        )?;
        if planned {
            compare_portfolio_clock_sources(&actual, &expected, true)?;
        }
        compare_workshop_sources(&actual, &expected, planned)?;
        compare_manager_sources(&actual, &expected, planned)?;
        compare_input_sources(
            &actual,
            &expected,
            planned,
            self.input_policy_witness.as_deref(),
        )?;
        if planned {
            let mut current = reference_recovery(checkpoint)?;
            validate_reference_sources(&actual, current)?;
            let expected_original = expected
                .recovery
                .as_ref()
                .ok_or_else(|| invalid("project intent original is absent"))?;
            // Journal ancestry belongs to the actual coordinator. Only its
            // immutable journal identity is compared by this metadata seam.
            current.frontier.head = digest(&expected_original.frontier.head)?;
            original(current, expected_original, expected.observation_tick_floor)?;
            if actual.observation_tick_floor < expected.observation_tick_floor {
                return Err(denied(
                    "project publication regressed its original observation floor",
                ));
            }
        } else {
            validate_preplan_sources(&actual)?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Retained {
    schema: String,
    recovery: Option<RecoveryInput>,
    native_custody: NativeCustody,
    observation_tick_floor: u64,
    clock: Option<ClockPin>,
    inventory_clock: Option<ClockPin>,
    portfolio_clocks: Option<Vec<PortfolioClockPin>>,
    stock_watch: Option<super::super::project_stock_watch::Checkpoint>,
    workshop_prerequisite: Option<WorkshopSource>,
    manager_prerequisite: Option<ManagerSource>,
    input_supply: Option<InputSource>,
    original_goal_proven: bool,
    preparation_restored: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeCustody {
    workforce_journal_id: String,
    order_journal_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ClockPin {
    journal_id: String,
    head: String,
    authority_tick_floor: u64,
    high_source_sequence: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PortfolioClockPin {
    step_id: u32,
    clock: Option<ClockPin>,
    inventory_clock: Option<ClockPin>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkshopSource {
    #[serde(deserialize_with = "workshop_checkpoint")]
    checkpoint: Option<WorkshopPrerequisiteCheckpoint>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagerSource {
    #[serde(deserialize_with = "manager_config::checkpoint")]
    checkpoint: Option<ManagerPrerequisiteCheckpoint>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputSource {
    #[serde(deserialize_with = "input_config::checkpoint")]
    checkpoint: Option<InputCheckpoint>,
}

fn retained(value: &Value) -> Result<Retained> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| invalid("project production recovery is not serializable"))?;
    if bytes.len() > MAX_REFERENCE_BYTES {
        return Err(invalid(
            "project production recovery exceeds its compact bound",
        ));
    }
    let retained: Retained = serde_json::from_slice(&bytes)
        .map_err(|_| invalid("invalid closed project production recovery reference"))?;
    if retained.schema != SCHEMA
        || retained.original_goal_proven
        || retained.preparation_restored
        || retained.observation_tick_floor > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
    {
        return Err(denied(
            "project recovery is historical custody, not restored authority or proof",
        ));
    }
    super::super::project_stock_watch::from_reference(value)?;
    Ok(retained)
}

fn reference_recovery(reference: &Value) -> Result<Recovery> {
    let input: RecoveryInput = serde_json::from_value(reference["recovery"].clone())
        .map_err(|_| invalid("project publication requires a complete exact original selector"))?;
    decode_recovery(
        input,
        !reference["clock"].is_null(),
        !reference["inventory_clock"].is_null(),
    )
}

fn validate_preplan_sources(reference: &Retained) -> Result<()> {
    if reference.recovery.is_none() && reference.portfolio_clocks.is_some() {
        return Err(denied(
            "pre-plan custody cannot adopt original portfolio clocks",
        ));
    }
    digest(&reference.native_custody.workforce_journal_id)?;
    digest(&reference.native_custody.order_journal_id)?;
    let floor = reference.observation_tick_floor;
    for clock in [reference.clock.as_ref(), reference.inventory_clock.as_ref()]
        .into_iter()
        .flatten()
    {
        digest(&clock.journal_id)?;
        digest(&clock.head)?;
        if clock.authority_tick_floor > floor {
            return Err(denied(
                "project source checkpoint exceeds its retained authority floor",
            ));
        }
    }
    if let Some(source) = &reference.workshop_prerequisite {
        let checkpoint = source
            .checkpoint
            .ok_or_else(|| denied("project Workshop checkpoint is absent"))?;
        checkpoint
            .floor
            .validate(checkpoint.floor.generation, floor)?;
    }
    if let Some(source) = &reference.manager_prerequisite {
        let checkpoint = source
            .checkpoint
            .ok_or_else(|| denied("project Manager checkpoint is absent"))?;
        checkpoint
            .floor
            .validate(checkpoint.floor.generation, floor)?;
    }
    if let Some(source) = &reference.input_supply {
        let checkpoint = source
            .checkpoint
            .ok_or_else(|| denied("project input checkpoint is absent"))?;
        checkpoint.floor.validate(0, floor)?;
    }
    Ok(())
}

fn validate_reference_sources(reference: &Retained, recovery: Recovery) -> Result<()> {
    validate_preplan_sources(reference)?;
    validate_portfolio(recovery)?;
    portfolio_clock_pins(reference, recovery)?;
    if recovery.anchor.tick.get() > reference.observation_tick_floor
        || recovery.frontier.authority_tick_floor < reference.observation_tick_floor
    {
        return Err(denied(
            "project original or source evidence exceeds its retained frontier floor",
        ));
    }
    for (clock, original) in [
        (reference.clock.as_ref(), recovery.clock_journal_id),
        (
            reference.inventory_clock.as_ref(),
            recovery.inventory_clock_journal_id,
        ),
    ] {
        same_optional_digest(original, clock.map(|pin| pin.journal_id.as_str()))?;
    }
    let workshop = reference
        .workshop_prerequisite
        .as_ref()
        .and_then(|source| source.checkpoint);
    if workshop.map(|checkpoint| checkpoint.floor) != recovery.workshop_prerequisite_floor
        || workshop.is_some() != recovery.workshop_prerequisite_digest.is_some()
    {
        return Err(denied(
            "project original lost its exact Workshop association or source floors",
        ));
    }
    let manager = reference
        .manager_prerequisite
        .as_ref()
        .and_then(|source| source.checkpoint);
    if manager.map(|checkpoint| checkpoint.floor) != recovery.manager_prerequisite_floor
        || manager.is_some() != recovery.manager_prerequisite_digest.is_some()
    {
        return Err(denied(
            "project original lost its exact Manager association or source floors",
        ));
    }
    let input = reference
        .input_supply
        .as_ref()
        .and_then(|source| source.checkpoint);
    if input.map(|checkpoint| checkpoint.floor) != recovery.input_supply_floor
        || input.map(|checkpoint| checkpoint.origin_witness) != recovery.input_supply_digest
    {
        return Err(denied(
            "project original lost its exact input binding or source floors",
        ));
    }
    Ok(())
}

fn compare_clock_sources(
    actual: Option<&ClockPin>,
    expected: Option<&ClockPin>,
    planned: bool,
) -> Result<()> {
    match (actual, expected) {
        (Some(actual), Some(expected)) => {
            same_digest(digest(&actual.journal_id)?, &expected.journal_id)?;
            digest(&actual.head)?;
            if planned {
                monotone(
                    &[actual.authority_tick_floor, actual.high_source_sequence],
                    &[expected.authority_tick_floor, expected.high_source_sequence],
                    false,
                    false,
                )?;
            } else {
                same_digest(digest(&actual.head)?, &expected.head)?;
            }
            Ok(())
        }
        (None, None) => Ok(()),
        _ => Err(denied("project publication changed its clock custodians")),
    }
}

fn compare_workshop_sources(actual: &Retained, expected: &Retained, planned: bool) -> Result<()> {
    let checkpoint = |value: &Retained| {
        value
            .workshop_prerequisite
            .as_ref()
            .and_then(|source| source.checkpoint)
    };
    match (checkpoint(actual), checkpoint(expected)) {
        (Some(actual), Some(expected)) if actual.origin_witness == expected.origin_witness => {
            if planned {
                workshop_floor_at_least(actual.floor, expected.floor)?;
            }
            Ok(())
        }
        _ => Err(denied(
            "project publication changed its acknowledged Workshop donor",
        )),
    }
}

fn compare_manager_sources(actual: &Retained, expected: &Retained, planned: bool) -> Result<()> {
    let checkpoint = |value: &Retained| {
        value
            .manager_prerequisite
            .as_ref()
            .and_then(|source| source.checkpoint)
    };
    match (checkpoint(actual), checkpoint(expected)) {
        (Some(actual), Some(expected)) if actual.origin_witness == expected.origin_witness => {
            if planned {
                manager_floor_at_least(actual.floor, expected.floor)?;
            }
            Ok(())
        }
        (None, None)
            if actual.manager_prerequisite.is_none() && expected.manager_prerequisite.is_none() =>
        {
            Ok(())
        }
        _ => Err(denied(
            "project publication changed its acknowledged Manager donor",
        )),
    }
}

fn compare_input_sources(
    actual: &Retained,
    expected: &Retained,
    planned: bool,
    policy: Option<&str>,
) -> Result<()> {
    let checkpoint = |value: &Retained| {
        value
            .input_supply
            .as_ref()
            .and_then(|source| source.checkpoint)
    };
    match (checkpoint(actual), checkpoint(expected)) {
        (Some(actual), Some(expected)) => {
            let origin = if planned {
                expected.origin_witness
            } else {
                digest(policy.ok_or_else(|| denied("project intent lost its input policy"))?)?
            };
            if actual.origin_witness != origin {
                return Err(denied(
                    "project publication changed its original input policy or binding",
                ));
            }
            if planned {
                input_floor_at_least(actual.floor, expected.floor)?;
            }
            Ok(())
        }
        (None, None)
            if actual.input_supply.is_none()
                && expected.input_supply.is_none()
                && policy.is_none() =>
        {
            Ok(())
        }
        _ => Err(denied("project publication changed its input source owner")),
    }
}

/// Join only closed rejection counters. The original origin witness remains
/// fixed, and a retained fence can never become a fresh readiness assertion.
fn join_source_floor(
    reference: &mut Value,
    source: &str,
    recovery_field: &str,
    incoming: &Value,
) -> Result<()> {
    if incoming.is_null() {
        return Ok(());
    }
    let current = reference[source]["checkpoint"]
        .as_object()
        .ok_or_else(|| denied("project publication cannot add a source owner"))?;
    let mut joined = current.clone();
    let incoming = incoming
        .as_object()
        .ok_or_else(|| invalid("invalid project source checkpoint"))?;
    for (field, value) in incoming {
        if field == "origin_witness" {
            continue;
        }
        let previous = joined
            .get(field)
            .ok_or_else(|| invalid("unknown project rejection floor"))?;
        let value = if field == "fenced" {
            Value::Bool(
                previous
                    .as_bool()
                    .ok_or_else(|| invalid("invalid project source fence"))?
                    || value
                        .as_bool()
                        .ok_or_else(|| invalid("invalid project source fence"))?,
            )
        } else {
            serde_json::json!(
                previous
                    .as_u64()
                    .ok_or_else(|| invalid("invalid project source counter"))?
                    .max(
                        value
                            .as_u64()
                            .ok_or_else(|| invalid("invalid project source counter"))?
                    )
            )
        };
        joined.insert(field.clone(), value);
    }
    let tick = joined
        .get("tick")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("project source lost its rejection tick"))?;
    reference[source]["checkpoint"] = Value::Object(joined.clone());
    joined.remove("origin_witness");
    reference["recovery"][recovery_field] = Value::Object(joined);
    raise_reference_floor(reference, tick)
}

fn raise_reference_floor(reference: &mut Value, incoming: u64) -> Result<()> {
    let floor = reference["observation_tick_floor"]
        .as_u64()
        .ok_or_else(|| invalid("project publication lost its authority floor"))?
        .max(incoming);
    reference["observation_tick_floor"] = serde_json::json!(floor);
    let frontier = reference["recovery"]["frontier"]["authority_tick_floor"]
        .as_u64()
        .ok_or_else(|| invalid("project publication lost its frontier floor"))?
        .max(floor);
    reference["recovery"]["frontier"]["authority_tick_floor"] = serde_json::json!(frontier);
    Ok(())
}

fn publication_reference(prior: &Value, intent: &ProjectProductionIntent) -> Result<Value> {
    intent.validate_checkpoint(prior)?;
    if !prior["recovery"].is_null() {
        return Ok(prior.clone());
    }
    let mut reference = intent.recovery_reference.clone();
    raise_reference_floor(
        &mut reference,
        prior["observation_tick_floor"]
            .as_u64()
            .ok_or_else(|| invalid("parent lost its production authority floor"))?,
    )?;
    for (source, recovery) in [
        ("workshop_prerequisite", "workshop_prerequisite_floor"),
        ("manager_prerequisite", "manager_prerequisite_floor"),
        ("input_supply", "input_supply_floor"),
    ] {
        join_source_floor(
            &mut reference,
            source,
            recovery,
            &prior[source]["checkpoint"],
        )?;
    }
    for clock in ["clock", "inventory_clock"] {
        if !prior[clock].is_null() {
            for counter in ["authority_tick_floor", "high_source_sequence"] {
                let value = reference[clock][counter]
                    .as_u64()
                    .ok_or_else(|| invalid("intent clock rejection floor is absent"))?
                    .max(
                        prior[clock][counter]
                            .as_u64()
                            .ok_or_else(|| invalid("parent clock rejection floor is absent"))?,
                    );
                reference[clock][counter] = serde_json::json!(value);
            }
            if reference["portfolio_clocks"].is_array() {
                reference["portfolio_clocks"][0][clock] = reference[clock].clone();
            }
        }
    }
    super::super::project_stock_watch::join_intent_watch(&mut reference, prior)?;
    Ok(reference)
}

fn same_digest(current: Digest32, prior: &str) -> Result<()> {
    if current == digest(prior)? {
        Ok(())
    } else {
        Err(denied(
            "project recovery changed an acknowledged original identity",
        ))
    }
}

fn same_optional_digest(current: Option<Digest32>, prior: Option<&str>) -> Result<()> {
    if current == prior.map(digest).transpose()? {
        Ok(())
    } else {
        Err(denied(
            "project recovery changed an acknowledged original association",
        ))
    }
}

fn monotone(current: &[u64], prior: &[u64], fenced: bool, prior_fenced: bool) -> Result<()> {
    if current.len() != prior.len()
        || current.iter().zip(prior).any(|(now, before)| now < before)
        || (prior_fenced && !fenced)
    {
        return Err(denied(
            "project recovery regressed a source floor or cleared a sticky fence",
        ));
    }
    Ok(())
}

fn workshop_floor_at_least(
    current: WorkshopRecoveryFloor,
    prior: WorkshopRecoveryFloor,
) -> Result<()> {
    monotone(
        &[
            current.tick,
            current.generation,
            u64::from(current.next_building_id),
        ],
        &[
            prior.tick,
            prior.generation,
            u64::from(prior.next_building_id),
        ],
        current.fenced,
        prior.fenced,
    )
}

fn manager_floor_at_least(
    current: ManagerRecoveryFloor,
    prior: ManagerRecoveryFloor,
) -> Result<()> {
    let counters = |floor: ManagerRecoveryFloor| {
        [
            floor.tick,
            floor.generation,
            floor.sequence,
            u64::from(floor.next_event),
            u64::from(floor.history_count),
            u64::from(floor.group_next_position),
            u64::from(floor.group_next_assignment),
            u64::from(floor.civ_next_position),
            u64::from(floor.civ_next_assignment),
        ]
    };
    monotone(
        &counters(current),
        &counters(prior),
        current.fenced,
        prior.fenced,
    )
}

fn input_floor_at_least(current: InputFloor, prior: InputFloor) -> Result<()> {
    let counters = |floor: InputFloor| {
        [
            floor.tick,
            floor.generation,
            u64::from(floor.next_job_id),
            u64::from(floor.next_building_id),
            u64::from(floor.next_item_id),
        ]
    };
    monotone(
        &counters(current),
        &counters(prior),
        current.fenced,
        prior.fenced,
    )
}

fn optional_floor<T: Copy>(
    current: Option<T>,
    prior: Option<T>,
    check: impl FnOnce(T, T) -> Result<()>,
) -> Result<()> {
    match (current, prior) {
        (Some(current), Some(prior)) => check(current, prior),
        (None, None) => Ok(()),
        _ => Err(denied(
            "project recovery added or removed an original source owner",
        )),
    }
}

/// Preserve the complete immutable slot set. Clock presence is a property of
/// the configured family and is identical for every original order.
fn validate_portfolio(recovery: Recovery) -> Result<()> {
    let Some(slots) = recovery.portfolio else {
        return Ok(());
    };
    let first =
        slots[0].ok_or_else(|| denied("project portfolio lost its first original order"))?;
    if first.step_id > 1
        || recovery.goal_history_present != first.goal_history_present
        || recovery.clock_journal_id != first.clock_journal_id
        || recovery.inventory_clock_journal_id != first.inventory_clock_journal_id
    {
        return Err(denied(
            "project portfolio changed its first-order summary or clock custody",
        ));
    }
    let mut ended = false;
    let mut identities = std::collections::BTreeSet::new();
    for (index, slot) in slots.into_iter().enumerate() {
        match slot {
            Some(slot)
                if !ended
                    && slot.step_id == first.step_id + index as u32
                    && slot.clock_journal_id.is_some() == first.clock_journal_id.is_some()
                    && slot.inventory_clock_journal_id.is_some()
                        == first.inventory_clock_journal_id.is_some() =>
            {
                for id in [slot.clock_journal_id, slot.inventory_clock_journal_id]
                    .into_iter()
                    .flatten()
                {
                    if id == Digest32::ZERO || !identities.insert(id) {
                        return Err(denied(
                            "original portfolio clocks require distinct custodians",
                        ));
                    }
                }
            }
            None => ended = true,
            Some(_) => {
                return Err(denied(
                    "project portfolio requires every original order and configured clock in order",
                ));
            }
        }
    }
    Ok(())
}

/// Decode the complete source-frontier vector before any private file opens.
/// Legacy scalar/clockless references keep their absent field and exact bytes.
fn portfolio_clock_pins(
    reference: &Retained,
    recovery: Recovery,
) -> Result<Option<[Option<ProjectPortfolioClockRecovery>; 4]>> {
    validate_portfolio(recovery)?;
    let Some(slots) = recovery.portfolio else {
        return if reference.portfolio_clocks.is_none() {
            Ok(None)
        } else {
            Err(denied(
                "scalar original cannot adopt portfolio clock custody",
            ))
        };
    };
    let first = slots[0].ok_or_else(|| denied("project portfolio lost its first order"))?;
    let configured = first.clock_journal_id.is_some() || first.inventory_clock_journal_id.is_some();
    if !configured {
        return if reference.portfolio_clocks.is_none() {
            Ok(None)
        } else {
            Err(denied(
                "clockless original cannot add portfolio clock metadata",
            ))
        };
    }
    let pins = reference.portfolio_clocks.as_deref().ok_or_else(|| {
        denied("project recovery requires every original clock head and source floor")
    })?;
    if pins.len() != slots.iter().flatten().count()
        || pins.is_empty()
        || pins.len() > 4
        || pins[0].clock != reference.clock
        || pins[0].inventory_clock != reference.inventory_clock
    {
        return Err(denied(
            "project clock vector changed its complete first-order summary",
        ));
    }
    let mut result = [None; 4];
    for (index, (pin, slot)) in pins.iter().zip(slots.into_iter().flatten()).enumerate() {
        if pin.step_id != slot.step_id {
            return Err(denied(
                "project clock vector replaced or reordered an original slot",
            ));
        }
        let decode = |pin: Option<&ClockPin>, expected: Option<Digest32>| match (pin, expected) {
            (Some(pin), Some(expected))
                if digest(&pin.journal_id)? == expected
                    && pin.authority_tick_floor <= reference.observation_tick_floor =>
            {
                Ok(Some(ProjectClockRecovery {
                    journal_id: expected,
                    head: digest(&pin.head)?,
                    authority_tick_floor: pin.authority_tick_floor,
                    high_source_sequence: pin.high_source_sequence,
                }))
            }
            (None, None) => Ok(None),
            _ => Err(denied(
                "project clock vector lost a custodian or its consumed source floor",
            )),
        };
        result[index] = Some(ProjectPortfolioClockRecovery {
            step_id: pin.step_id,
            clock: decode(pin.clock.as_ref(), slot.clock_journal_id)?,
            inventory_clock: decode(
                pin.inventory_clock.as_ref(),
                slot.inventory_clock_journal_id,
            )?,
        });
    }
    Ok(Some(result))
}

fn compare_portfolio_clock_sources(
    actual: &Retained,
    expected: &Retained,
    descendant: bool,
) -> Result<()> {
    match (&actual.portfolio_clocks, &expected.portfolio_clocks) {
        (None, None) => Ok(()),
        (Some(actual), Some(expected)) if actual.len() == expected.len() => {
            for (actual, expected) in actual.iter().zip(expected) {
                if actual.step_id != expected.step_id {
                    return Err(denied("project publication changed an original clock slot"));
                }
                compare_clock_sources(actual.clock.as_ref(), expected.clock.as_ref(), descendant)?;
                compare_clock_sources(
                    actual.inventory_clock.as_ref(),
                    expected.inventory_clock.as_ref(),
                    descendant,
                )?;
            }
            Ok(())
        }
        _ => Err(denied(
            "project publication changed its complete clock source vector",
        )),
    }
}

fn same_portfolio(current: Recovery, prior: &RecoveryInput) -> Result<()> {
    validate_portfolio(current)?;
    match (current.portfolio, prior.portfolio.as_deref()) {
        (None, None) => Ok(()),
        (Some(current), Some(prior_slots)) => {
            let count = current.iter().flatten().count();
            if count != prior_slots.len()
                || prior_slots
                    .first()
                    .is_none_or(|first| prior.goal_history_present != first.goal_history_present)
            {
                return Err(denied(
                    "project recovery changed its complete original portfolio",
                ));
            }
            for (now, before) in current.into_iter().flatten().zip(prior_slots) {
                if now.step_id != before.step_id
                    || (before.goal_history_present && !now.goal_history_present)
                {
                    return Err(denied(
                        "project recovery replaced an order or erased its retained goal history",
                    ));
                }
                same_optional_digest(now.clock_journal_id, before.clock_journal_id.as_deref())?;
                same_optional_digest(
                    now.inventory_clock_journal_id,
                    before.inventory_clock_journal_id.as_deref(),
                )?;
            }
            Ok(())
        }
        _ => Err(denied(
            "project recovery cannot add or discard an original portfolio",
        )),
    }
}

fn original(current: Recovery, prior: &RecoveryInput, observation_tick_floor: u64) -> Result<()> {
    same_portfolio(current, prior)?;
    same_digest(current.seal.plan_digest, &prior.plan_digest)?;
    same_digest(current.seal.object_digest, &prior.object_digest)?;
    same_digest(current.identity_digest, &prior.identity_digest)?;
    same_digest(current.anchor.state_hash, &prior.anchor.state_hash)?;
    if current.anchor.fortress_id.get().to_string() != prior.anchor.fortress_id
        || current.anchor.cursor.epoch != prior.anchor.epoch
        || current.anchor.cursor.sequence != prior.anchor.sequence
        || current.anchor.tick.get() != prior.anchor.tick
        || (prior.goal_history_present && !current.goal_history_present)
        || current.frontier.authority_tick_floor < prior.frontier.authority_tick_floor
        || current.frontier.authority_tick_floor < observation_tick_floor
    {
        return Err(denied(
            "project recovery changed its original anchor, goal history or authority floor",
        ));
    }
    // Exact parent head is the replay starting point. The actual coordinator
    // subsequently verifies that its durable file is a descendant of this pin.
    same_digest(current.frontier.journal_id, &prior.frontier.journal_id)?;
    same_digest(current.frontier.head, &prior.frontier.head)?;
    same_optional_digest(current.clock_journal_id, prior.clock_journal_id.as_deref())?;
    same_optional_digest(
        current.inventory_clock_journal_id,
        prior.inventory_clock_journal_id.as_deref(),
    )?;
    same_optional_digest(
        current.workshop_prerequisite_digest,
        prior.workshop_prerequisite_digest.as_deref(),
    )?;
    same_optional_digest(
        current.manager_prerequisite_digest,
        prior.manager_prerequisite_digest.as_deref(),
    )?;
    same_optional_digest(
        current.input_supply_digest,
        prior.input_supply_digest.as_deref(),
    )?;
    optional_floor(
        current.workshop_prerequisite_floor,
        prior.workshop_prerequisite_floor,
        workshop_floor_at_least,
    )?;
    optional_floor(
        current.manager_prerequisite_floor,
        prior.manager_prerequisite_floor,
        manager_floor_at_least,
    )?;
    optional_floor(
        current.input_supply_floor,
        prior.input_supply_floor,
        input_floor_at_least,
    )
}

fn clock_pin(
    current: Option<ClockConfig>,
    prior: Option<&ClockPin>,
) -> Result<Option<ProjectClockRecovery>> {
    match (current, prior) {
        (Some(current), Some(prior)) => {
            let journal_id = digest(&prior.journal_id)?;
            let head = digest(&prior.head)?;
            if current.journal_id != Some(journal_id)
                || prior.authority_tick_floor > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
            {
                return Err(denied(
                    "project recovery changed its clock custodian or source floor",
                ));
            }
            Ok(Some(ProjectClockRecovery {
                journal_id,
                head,
                authority_tick_floor: prior.authority_tick_floor,
                high_source_sequence: prior.high_source_sequence,
            }))
        }
        (None, None) => Ok(None),
        _ => Err(denied(
            "project recovery cannot hide or add a clock custodian",
        )),
    }
}

impl Config {
    /// Recover only the original whose complete compiled identity the parent
    /// ACKed before publication. This supplies lost reply selectors internally;
    /// Runtime::open still opens every existing file without initialization.
    pub(crate) fn recover_project_publication(
        &mut self,
        prior: &Value,
        intent: &ProjectProductionIntent,
    ) -> Result<()> {
        self.current()?;
        let binding = self
            .project_binding
            .ok_or_else(|| denied("project publication requires its completed Workshop handoff"))?;
        if self.initialize
            || intent.goal.deadline_tick() != binding.deadline_tick
            || intent.goal.deadline_tick() > binding.authority_expires_tick
        {
            return Err(denied(
                "project publication cannot initialize custody or change its original deadline",
            ));
        }
        let expected = retained(&intent.recovery_reference)?;
        let origin = expected
            .workshop_prerequisite
            .as_ref()
            .and_then(|source| source.checkpoint)
            .ok_or_else(|| denied("project publication lost its Workshop origin"))?
            .origin_witness;
        if origin != binding.workshop_origin {
            return Err(denied(
                "project publication belongs to another completed Workshop",
            ));
        }
        let mut reference = publication_reference(prior, intent)?;
        let pending = prior["recovery"].is_null();
        if self.recovery.is_none() {
            // Optional operator-retained rejection floors can only strengthen
            // the independently ACKed parent pins; they supply no original.
            if let Some(checkpoint) = self.workshop_prerequisite_checkpoint {
                if checkpoint.origin_witness != origin {
                    return Err(denied(
                        "project publication changed its pre-plan Workshop donor",
                    ));
                }
                let f = checkpoint.floor;
                join_source_floor(
                    &mut reference,
                    "workshop_prerequisite",
                    "workshop_prerequisite_floor",
                    &serde_json::json!({"tick":f.tick,"generation":f.generation,
                        "next_building_id":f.next_building_id,"fenced":f.fenced}),
                )?;
            }
            if let Some(checkpoint) = self.manager_prerequisite_checkpoint {
                let expected_origin = expected
                    .manager_prerequisite
                    .as_ref()
                    .and_then(|source| source.checkpoint)
                    .ok_or_else(|| denied("project publication cannot add a Manager donor"))?
                    .origin_witness;
                if checkpoint.origin_witness != expected_origin {
                    return Err(denied(
                        "project publication changed its pre-plan Manager donor",
                    ));
                }
                let f = checkpoint.floor;
                join_source_floor(
                    &mut reference,
                    "manager_prerequisite",
                    "manager_prerequisite_floor",
                    &serde_json::json!({"tick":f.tick,"generation":f.generation,"sequence":f.sequence,
                        "next_event":f.next_event,"history_count":f.history_count,
                        "group_next_position":f.group_next_position,"group_next_assignment":f.group_next_assignment,
                        "civ_next_position":f.civ_next_position,"civ_next_assignment":f.civ_next_assignment,
                        "fenced":f.fenced}),
                )?;
            }
            if let Some(checkpoint) = self.input_supply_checkpoint {
                if Some(checkpoint.origin_witness)
                    != intent
                        .input_policy_witness
                        .as_deref()
                        .map(digest)
                        .transpose()?
                {
                    return Err(denied(
                        "project publication changed its pre-plan input policy",
                    ));
                }
                let f = checkpoint.floor;
                join_source_floor(
                    &mut reference,
                    "input_supply",
                    "input_supply_floor",
                    &serde_json::json!({"tick":f.tick,"generation":f.generation,"next_job_id":f.next_job_id,
                        "next_building_id":f.next_building_id,"next_item_id":f.next_item_id,"fenced":f.fenced}),
                )?;
            }
            let recovery = reference_recovery(&reference)?;
            validate_reference_sources(&retained(&reference)?, recovery)?;
            self.recovery = Some(recovery);
            self.workshop_prerequisite_checkpoint = None;
            self.manager_prerequisite_checkpoint = None;
            self.input_supply_checkpoint = None;
        }
        self.validate_project_recovery(&reference)?;
        if pending {
            intent.validate_pending_recovery(
                self.recovery
                    .ok_or_else(|| denied("project publication lost its exact original"))?,
            )?;
        }
        let expected = self
            .project_recovery
            .as_mut()
            .ok_or_else(|| denied("project publication lost its authenticated recovery pins"))?;
        expected.publication_pending = pending;
        Ok(())
    }

    /// Validate all caller-selected recovery identities before Runtime::open
    /// can touch child files or native sources. Actual file ancestry is checked
    /// by each durable owner after it acquires its original exclusive custody.
    pub(crate) fn validate_project_recovery(&mut self, reference: &Value) -> Result<()> {
        self.current()?;
        let binding = self
            .project_binding
            .ok_or_else(|| denied("project recovery requires the verified Workshop handoff"))?;
        if self.initialize {
            return Err(denied(
                "acknowledged project child custody cannot be initialized again",
            ));
        }
        let prior = retained(reference)?;
        if let Some(watch) = &prior.stock_watch {
            let terms = watch.terms()?;
            if terms.deadline_tick.get() != binding.deadline_tick
                || terms.original_anchor.fortress_id != self.workforce.acceptance().fortress()
            {
                return Err(denied(
                    "project stock watch changed its original fortress or deadline",
                ));
            }
        }
        let portfolio_clocks = if prior.recovery.is_some() {
            let recovery = reference_recovery(reference)?;
            validate_reference_sources(&prior, recovery)?;
            portfolio_clock_pins(&prior, recovery)?
        } else {
            validate_preplan_sources(&prior)?;
            None
        };
        match (self.recovery, prior.recovery.as_ref()) {
            (Some(current), Some(prior_original)) => {
                original(current, prior_original, prior.observation_tick_floor)?;
            }
            (None, None) => {}
            _ => {
                return Err(denied(
                    "project recovery cannot adopt or discard an unacknowledged original",
                ));
            }
        }
        let workforce_journal = digest(&prior.native_custody.workforce_journal_id)?;
        let order_journal = digest(&prior.native_custody.order_journal_id)?;
        // Native workforce configuration has an independently retained journal
        // identity. The order owner exposes its identity only after local open,
        // so retain that expectation privately for the actual-owner check.
        let raw = family_environment()?;
        let workforce: Value = serde_json::from_str(&raw[1])
            .map_err(|_| invalid("invalid retained workforce configuration"))?;
        let configured_workforce = workforce
            .get("journal_id")
            .and_then(Value::as_str)
            .ok_or_else(|| denied("project recovery lost its workforce journal identity"))?;
        same_digest(workforce_journal, configured_workforce)?;
        let clock = clock_pin(self.clock, prior.clock.as_ref())?;
        let inventory_clock = clock_pin(self.inventory_clock, prior.inventory_clock.as_ref())?;
        let mut observation_tick_floor = prior.observation_tick_floor.max(
            self.recovery
                .map_or(0, |recovery| recovery.frontier.authority_tick_floor),
        );
        let mut source_floor = clock
            .map_or(0, |pin| pin.authority_tick_floor)
            .max(inventory_clock.map_or(0, |pin| pin.authority_tick_floor));
        if let Some(slots) = portfolio_clocks {
            for slot in slots.into_iter().flatten() {
                source_floor = source_floor
                    .max(slot.clock.map_or(0, |pin| pin.authority_tick_floor))
                    .max(
                        slot.inventory_clock
                            .map_or(0, |pin| pin.authority_tick_floor),
                    );
            }
        }

        let workshop = prior
            .workshop_prerequisite
            .as_ref()
            .and_then(|source| source.checkpoint);
        if self.workshop_prerequisite.is_some() != workshop.is_some() {
            return Err(denied(
                "project recovery lost its Workshop source checkpoint",
            ));
        }
        if let Some(previous) = workshop {
            if previous.origin_witness != binding.workshop_origin {
                return Err(denied(
                    "project recovery changed its verified Workshop donor",
                ));
            }
            let current = if let Some(recovery) = self.recovery {
                recovery
                    .workshop_prerequisite_floor
                    .ok_or_else(|| denied("project recovery lost Workshop rejection floors"))?
            } else {
                let checkpoint = self.workshop_prerequisite_checkpoint.ok_or_else(|| {
                    denied("project recovery requires its latest pre-plan Workshop checkpoint")
                })?;
                if checkpoint.origin_witness != previous.origin_witness {
                    return Err(denied("pre-plan Workshop checkpoint changed its donor"));
                }
                checkpoint.floor
            };
            workshop_floor_at_least(current, previous.floor)?;
            source_floor = source_floor.max(previous.floor.tick);
            observation_tick_floor = observation_tick_floor.max(current.tick);
        }

        let manager = prior
            .manager_prerequisite
            .as_ref()
            .and_then(|source| source.checkpoint);
        if self.manager_prerequisite.is_some() != manager.is_some() {
            return Err(denied(
                "project recovery lost its Manager source checkpoint",
            ));
        }
        if let Some(previous) = manager {
            let current = if let Some(recovery) = self.recovery {
                recovery
                    .manager_prerequisite_floor
                    .ok_or_else(|| denied("project recovery lost Manager rejection floors"))?
            } else {
                let checkpoint = self.manager_prerequisite_checkpoint.ok_or_else(|| {
                    denied("project recovery requires its latest pre-plan Manager checkpoint")
                })?;
                if checkpoint.origin_witness != previous.origin_witness {
                    return Err(denied("pre-plan Manager checkpoint changed its donor"));
                }
                checkpoint.floor
            };
            manager_floor_at_least(current, previous.floor)?;
            source_floor = source_floor.max(previous.floor.tick);
            observation_tick_floor = observation_tick_floor.max(current.tick);
        }

        let input = prior
            .input_supply
            .as_ref()
            .and_then(|source| source.checkpoint);
        if self.input_supply.is_some() != input.is_some() {
            return Err(denied(
                "project recovery lost its input-supply source checkpoint",
            ));
        }
        if let Some(previous) = input {
            let current = if let Some(recovery) = self.recovery {
                // The planned input origin is its exact original binding; the
                // pre-plan origin is the independently pinned reserve policy.
                if Some(previous.origin_witness) != recovery.input_supply_digest {
                    return Err(denied(
                        "project recovery changed its original input binding",
                    ));
                }
                recovery
                    .input_supply_floor
                    .ok_or_else(|| denied("project recovery lost input rejection floors"))?
            } else {
                let checkpoint = self.input_supply_checkpoint.ok_or_else(|| {
                    denied("project recovery requires its latest pre-plan input checkpoint")
                })?;
                if checkpoint.origin_witness != previous.origin_witness {
                    return Err(denied("pre-plan input checkpoint changed its policy"));
                }
                checkpoint.floor
            };
            input_floor_at_least(current, previous.floor)?;
            source_floor = source_floor.max(previous.floor.tick);
            observation_tick_floor = observation_tick_floor.max(current.tick);
        }
        if prior.observation_tick_floor < source_floor {
            return Err(denied(
                "project recovery observation floor precedes retained source evidence",
            ));
        }
        self.current()?;
        self.project_stock_watch = prior.stock_watch;
        self.project_recovery = Some(ProjectRecovery {
            workforce_journal,
            order_journal,
            observation_tick_floor,
            clock,
            inventory_clock,
            portfolio_clocks,
            manager_origin: manager.map(|checkpoint| checkpoint.origin_witness),
            publication_pending: false,
        });
        Ok(())
    }
}

#[cfg(test)]
#[path = "project_recovery_tests.rs"]
mod tests;
