//! Read-only, operator-selected excavation evidence in the mining handoff.
//! No transport, sampler, subprocess, journal writer or mutation authority.
use dfmcp_adapter::dig_designation::journal::DigBinding;
use dfmcp_adapter::excavation_goal::FloorStatus;
use dfmcp_core::{
    Capability, Digest32, ErrorCode, GameTick, MapCoord, MapCuboid, OperationContext, Result,
    RiskTier,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

mod archive;
mod private_file;
use archive::Archive;
use private_file::Snapshot;

pub(super) const ENVIRONMENT: &str = "DFMCP_DIG_GOAL_JOURNALS";
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_GOALS: usize = 4;
const WORK_PER_GOAL: u64 = 16 * MAX_BYTES as u64;
const OUTPUT_RESERVE: usize = 8 * 1024;

fn denied() -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(
        ErrorCode::CorruptLedger,
        "excavation evidence is unavailable or inconsistent; retain the original journal",
    )
}
fn budget_error() -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(
        ErrorCode::BudgetExceeded,
        "complete mining and excavation inventory exceed the request allowance",
    )
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileSpec {
    label: String,
    goal_id: String,
    journal: PathBuf,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Files(Vec<FileSpec>);
impl Files {
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 20 * 1024 {
            return Err(denied());
        }
        let mut files: Vec<FileSpec> = serde_json::from_str(raw).map_err(|_| denied())?;
        if files.len() > MAX_GOALS {
            return Err(denied());
        }
        let mut ids = BTreeSet::new();
        let mut labels = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for file in &files {
            if file.label.is_empty()
                || file.label.len() > 48
                || !file
                    .label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
                || archive::digest(&file.goal_id)? == Digest32::ZERO
            {
                return Err(denied());
            }
            let path = file.journal.to_str().ok_or_else(denied)?;
            if !path.starts_with('/')
                || path.len() > 4096
                || path[1..]
                    .split('/')
                    .any(|s| s.is_empty() || s == "." || s == ".." || s.contains('\0'))
                || !ids.insert(file.goal_id.clone())
                || !labels.insert(file.label.clone())
                || !paths.insert(file.journal.clone())
            {
                return Err(denied());
            }
        }
        files.sort_unstable_by(|a, b| a.goal_id.cmp(&b.goal_id));
        Ok(Self(files))
    }
    pub fn environment() -> Result<Self> {
        match std::env::var(ENVIRONMENT) {
            Ok(raw) => Self::parse(&raw),
            Err(std::env::VarError::NotPresent) => Ok(Self::default()),
            _ => Err(denied()),
        }
    }
}
#[derive(Clone)]
struct Pin {
    identity: (u64, u64, u64, u64),
    length: usize,
    digest: Digest32,
    summary: Value,
}
pub(super) struct Inventory {
    files: Files,
    pins: Vec<Option<Pin>>,
    high_tick: u64,
}
impl Inventory {
    pub fn new(files: Files) -> Self {
        Self {
            pins: vec![None; files.0.len()],
            files,
            high_tick: 0,
        }
    }
    pub fn release(&self, packet: String, maximum: usize) -> Result<String> {
        // Closing local custody neither samples nor cancels the independently
        // owned goals, and does not perform I/O after runtime revocation.
        attach(
            packet,
            self.files.0.iter().map(|s| unavailable(s, None)).collect(),
            maximum,
        )
    }
    pub fn narrow(&self, c: &mut OperationContext) {
        c.anchor.tick = GameTick(c.anchor.tick.get().max(self.high_tick));
    }
    /// Reserve reads, canonical reconstruction, complete replay and final
    /// re-verification before either goal-file or native recovery I/O.
    pub fn reserve(&self, c: &OperationContext) -> Result<OperationContext> {
        let mut out = c.clone();
        self.narrow(&mut out);
        out.budget.max_bytes = out
            .budget
            .max_bytes
            .checked_sub(WORK_PER_GOAL * self.files.0.len() as u64)
            .ok_or_else(budget_error)?;
        Ok(out)
    }
    pub fn load(
        &mut self,
        c: &OperationContext,
        b: &DigBinding,
        started: Instant,
        mut boundary: impl FnMut() -> Result<()>,
    ) -> Result<ReadSet> {
        let deadline = started
            .checked_add(Duration::from_millis(c.budget.max_wall_millis))
            .ok_or_else(budget_error)?;
        let mut check = || -> Result<()> {
            boundary()?;
            if c.cancellation_requested || Instant::now() >= deadline {
                return Err(budget_error());
            }
            Ok(())
        };
        check()?;
        if c.anchor.fortress_id != b.fortress_id() {
            return Err(denied());
        }
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], Some(b.scope()))?;
        let mut entries = Vec::with_capacity(self.files.0.len());
        for (index, spec) in self.files.0.iter().enumerate() {
            check()?;
            let candidate = (|| -> Result<(Snapshot, Pin, u64)> {
                let snapshot = Snapshot::open(&spec.journal, &mut check)?;
                if let Some(old) = &self.pins[index] {
                    if snapshot.identity() != old.identity
                        || snapshot.bytes().len() < old.length
                        || Digest32::of_bytes(&snapshot.bytes()[..old.length]) != old.digest
                    {
                        return Err(denied());
                    }
                }
                let archive = Archive::decode(snapshot.bytes(), &mut check)?;
                if archive.id != archive::digest(&spec.goal_id)? {
                    return Err(denied());
                }
                let goal = archive.progress.goal();
                let r = goal.region();
                let max = [
                    r.origin[0] + r.size[0] - 1,
                    r.origin[1] + r.size[1] - 1,
                    r.origin[2],
                ];
                let area = MapCuboid::new(
                    MapCoord::new(r.origin[0] as i32, r.origin[1] as i32, r.origin[2] as i32),
                    MapCoord::new(max[0] as i32, max[1] as i32, max[2] as i32),
                )?;
                if goal.folder() != b.folder()
                    || goal.site() != b.site()
                    || !b.scope().contains_cuboid(area)
                {
                    return Err(denied());
                }
                let tick = archive.progress.latest().tick().get();
                let summary = summary(spec, &archive)?;
                let pin = Pin {
                    identity: snapshot.identity(),
                    length: snapshot.bytes().len(),
                    digest: Digest32::of_bytes(snapshot.bytes()),
                    summary,
                };
                Ok((snapshot, pin, tick))
            })();
            match candidate {
                Ok((snapshot, pin, tick)) => {
                    // A verified historical observation may narrow expiry, never
                    // lower a later call's floor even if final publication fails.
                    self.high_tick = self.high_tick.max(tick);
                    entries.push(Entry {
                        snapshot: Some(snapshot),
                        candidate: Some(pin.clone()),
                        value: pin.summary,
                    });
                }
                Err(_) => entries.push(Entry {
                    snapshot: None,
                    candidate: None,
                    value: unavailable(spec, self.pins[index].as_ref()),
                }),
            }
        }
        let mut current = c.clone();
        self.narrow(&mut current);
        current.authorize(Capability::Query, RiskTier::ReadOnly, &[], Some(b.scope()))?;
        check()?;
        Ok(ReadSet { entries })
    }
}
fn summary(spec: &FileSpec, a: &Archive) -> Result<Value> {
    let p = &a.progress;
    let g = p.goal();
    let counts = p.counts()?;
    let source = p.latest();
    Ok(
        json!({"label":spec.label,"goal_id":a.id.to_string(),"journal_head":a.head.to_string(),
        "verification":"verified_snapshot","goal_status":if a.pending_read{"unknown"}else{p.status().as_str()},
        "terminal":p.status().terminal(),"unfinished_read":a.pending_read,"read_attempts":a.attempts,"events":a.events,
        "region":{"origin":g.region().origin,"size":g.region().size},"deadline_tick":g.deadline(),
        "stable_ticks":g.stable_ticks(),"required_samples":g.required_samples(),"max_gap_ticks":g.max_gap_ticks(),
        "sample_tick":source.tick().get(),"matching_samples":if a.pending_read{0}else{p.streak()},
        "matching_since_tick":if a.pending_read{None}else{p.since_tick()},"observations":p.observations(),
        "interruption":if a.pending_read{Some("unfinished_read")}else{p.interruption().map(|i|i.as_str())},
        "counts":{"floor_goal":counts.floor_goal,"wall":counts.wall,"other_shape":counts.other_shape,
            "wet_floor":counts.wet_floor,"designated_floor":counts.designated_floor,"hidden":counts.hidden,
            "missing":counts.missing,"active_designations":counts.active_designations},
        "map_generation":source.bridge_generation,"map_source_digest":source.source_digest()?.to_string(),
        "floor_goal_satisfied_at_sample":!a.pending_read&&p.status()==FloorStatus::Satisfied,
        "historical_evidence_only":true,"current_terrain_proven":false,"continuous_stability_proven":false,
        "mining_action_completed_proven":false,"retry_designation_permitted":false}),
    )
}
fn unavailable(spec: &FileSpec, prior: Option<&Pin>) -> Value {
    json!({"label":spec.label,"goal_id":spec.goal_id,"verification":"unavailable","goal_status":"unknown",
        "terminal":false,"pending_absence_proven":false,"historical_prior":prior.map(|p|&p.summary),
        "mining_action_completed_proven":false,"retry_designation_permitted":false,
        "recovery":"Retain the original goal journal; missing, busy, corrupt or replaced evidence is not absence."})
}
struct Entry {
    snapshot: Option<Snapshot>,
    candidate: Option<Pin>,
    value: Value,
}
pub(super) struct ReadSet {
    entries: Vec<Entry>,
}
impl ReadSet {
    /// Final custody checks happen before adding evidence to the complete packet.
    /// Projection failure never clears native pending work or changes its receipt.
    pub fn finish(
        mut self,
        inventory: &mut Inventory,
        packet: String,
        maximum: usize,
        mut boundary: impl FnMut() -> Result<()>,
    ) -> Result<String> {
        for (index, entry) in self.entries.iter_mut().enumerate() {
            boundary()?;
            if entry
                .snapshot
                .as_mut()
                .is_some_and(|s| s.verify(&mut boundary).is_err())
            {
                entry.candidate = None;
                entry.value =
                    unavailable(&inventory.files.0[index], inventory.pins[index].as_ref());
            }
        }
        let values = self
            .entries
            .iter()
            .map(|e| e.value.clone())
            .collect::<Vec<_>>();
        let output = attach(packet, values, maximum)?;
        boundary()?;
        for (index, entry) in self.entries.into_iter().enumerate() {
            if let Some(pin) = entry.candidate {
                inventory.pins[index] = Some(pin);
            }
        }
        Ok(output)
    }
}
fn attach(packet: String, rows: Vec<Value>, maximum: usize) -> Result<String> {
    if rows.is_empty() {
        return Ok(packet);
    }
    if serde_json::to_vec(&rows).map_err(|_| denied())?.len() > OUTPUT_RESERVE {
        return Err(budget_error());
    }
    let mut value: Value = serde_json::from_str(&packet).map_err(|_| denied())?;
    let active = value
        .get_mut("agent_turn")
        .and_then(|t| t.get_mut("active_work"))
        .and_then(Value::as_object_mut)
        .ok_or_else(denied)?;
    let verified = rows
        .iter()
        .all(|r| r["verification"] == "verified_snapshot");
    let pending = rows.iter().filter(|r| r["terminal"] != true).count();
    // Do not overwrite the dig journal's indeterminate-effects or its own
    // inventory_verified bit: the two inventories have independent evidence.
    active.insert(
        "excavation_goals".into(),
        json!({"schema":"dfmcp.excavation-inventory/1",
        "scope":"operator_configured_goal_journals_only","configured_count":rows.len(),
        "inventory_verified":verified,"pending_count":pending,
        "pending_absence_proven":verified&&pending==0,"goals":rows,
        "native_calls":0,"sampling_owner":"standalone_track_excavation",
        "game_effect_obligations_changed":false,"map_and_dig_generations_joined":false}),
    );
    let mut encoded = serde_json::to_string(&value).map_err(|_| denied())?;
    if encoded.len() > maximum {
        // Whole optional tool details are omitted, never safety/active-work
        // fields. A caller can request those native details again by exact key.
        value["result"] = json!({"ok":false,"error":{"code":"budget_exceeded",
            "message":"Optional tool details omitted to preserve complete native and goal recovery inventories."},
            "native_outcome_not_inferred":true,"retry_designation_permitted":false,"native_calls_not_retried":true});
        encoded = serde_json::to_string(&value).map_err(|_| denied())?;
    }
    if encoded.len() > maximum {
        return Err(budget_error());
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests;
