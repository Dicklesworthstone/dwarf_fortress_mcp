//! Exact furnishing plans and original-journal batch progress.
//!
//! This model grants no effect authority. Only a complete original prefix of
//! verified, durably terminal Placed records unlocks another exact selection.
use std::collections::BTreeSet;

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};

use crate::build_placement::journal::{BuildInventory, BuildState};
use crate::build_placement::{BuildBinding, BuildCapture, BuildKind, BuildPhase, BuildSelection};
use crate::furniture_handoff::{Handoff, MAX_HANDOFF_BYTES};

pub mod store;

pub const SCHEMA: &str = "dfmcp.furniture-plan/1";
pub const MAX_STEPS: usize = 32;
pub const MAX_PLAN_BYTES: usize = 16_384;
pub const MAX_LEGACY_DEFINITION_BYTES: usize = MAX_PLAN_BYTES + 1024 + 48;
pub const MAX_DEFINITION_BYTES: usize = MAX_LEGACY_DEFINITION_BYTES + 4 + MAX_HANDOFF_BYTES;
const DEFINITION_MAGIC: &[u8; 8] = b"DFMFBD01";
const HANDOFF_DEFINITION_MAGIC: &[u8; 8] = b"DFMFBD02";

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}
fn corrupt(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::CorruptLedger, message)
}
fn require(ok: bool, message: &str) -> Result<()> {
    if ok { Ok(()) } else { Err(invalid(message)) }
}
fn hash(domain: &[u8], bytes: &[u8]) -> Digest32 {
    let mut input = domain.to_vec();
    input.push(0);
    input.extend_from_slice(bytes);
    Digest32::of_bytes(&input)
}
fn label(value: &str) -> Result<()> {
    require(
        (1..=48).contains(&value.len())
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')),
        "furniture step names need 1..48 ASCII letters, digits, dot, underscore or hyphen",
    )
}

/// A fixed selection; dependencies concern placement registration, not construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FurnitureStep {
    pub name: String,
    pub selection: BuildSelection,
    pub after: Vec<String>,
}

/// Immutable normalized request with Python-compatible canonical JSON and digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FurniturePlan {
    steps: Vec<FurnitureStep>,
    order: Vec<usize>,
    bytes: Vec<u8>,
    digest: Digest32,
}
impl FurniturePlan {
    pub fn decode(raw: &[u8]) -> Result<Self> {
        require(
            (1..=MAX_PLAN_BYTES).contains(&raw.len()),
            "furniture plan exceeds 16 KiB",
        )?;
        check_depth(raw)?;
        let mut parser = Json(raw);
        parser.symbol(b'{')?;
        let mut schema = None;
        let mut steps = None;
        if !parser.consume(b'}') {
            loop {
                let field = parser.string(16)?;
                parser.symbol(b':')?;
                match field.as_str() {
                    "schema" if schema.is_none() => schema = Some(parser.string(32)?),
                    "steps" if steps.is_none() => steps = Some(parser.steps()?),
                    _ => return Err(invalid("unknown or duplicate furniture plan field")),
                }
                if parser.consume(b'}') {
                    break;
                }
                parser.symbol(b',')?;
            }
        }
        parser.whitespace();
        require(parser.0.is_empty(), "trailing furniture plan data")?;
        require(
            schema.as_deref() == Some(SCHEMA),
            "unsupported furniture plan schema",
        )?;
        Self::normalize(steps.ok_or_else(|| invalid("furniture plan lacks steps"))?)
    }

    pub(crate) fn normalize(mut steps: Vec<FurnitureStep>) -> Result<Self> {
        require(
            (1..=MAX_STEPS).contains(&steps.len()),
            "furniture plan requires 1..32 steps",
        )?;
        steps.sort_by(|a, b| a.name.cmp(&b.name));
        let mut names = BTreeSet::new();
        let mut items = BTreeSet::new();
        let mut targets = BTreeSet::new();
        for step in &mut steps {
            label(&step.name)?;
            require(
                names.insert(step.name.clone()),
                "duplicate furniture step name",
            )?;
            require(
                items.insert(step.selection.item_id()),
                "one item cannot furnish two targets",
            )?;
            require(
                targets.insert(step.selection.target()),
                "two furnishings share a target",
            )?;
            require(
                step.after.len() < MAX_STEPS,
                "furniture dependency bound exceeded",
            )?;
            step.after.sort();
            for (index, dependency) in step.after.iter().enumerate() {
                label(dependency)?;
                require(
                    dependency != &step.name
                        && (index == 0 || step.after[index - 1] != *dependency),
                    "duplicate or self furniture dependency",
                )?;
            }
        }
        require(
            steps
                .iter()
                .all(|step| step.after.iter().all(|name| names.contains(name))),
            "unresolved furniture dependency",
        )?;
        let mut done = BTreeSet::new();
        let mut order = Vec::with_capacity(steps.len());
        while order.len() < steps.len() {
            let next = steps
                .iter()
                .enumerate()
                .find(|(_, step)| {
                    !done.contains(&step.name) && step.after.iter().all(|name| done.contains(name))
                })
                .ok_or_else(|| invalid("cyclic furniture dependencies"))?;
            order.push(next.0);
            done.insert(next.1.name.clone());
        }
        // All interpolated strings were restricted to their exact ASCII catalog.
        let mut canonical = format!("{{\"schema\":\"{SCHEMA}\",\"steps\":[");
        for (index, step) in steps.iter().enumerate() {
            if index > 0 {
                canonical.push(',');
            }
            canonical.push_str("{\"after\":[");
            for (index, dependency) in step.after.iter().enumerate() {
                if index > 0 {
                    canonical.push(',');
                }
                canonical.push('"');
                canonical.push_str(dependency);
                canonical.push('"');
            }
            let [x, y, z] = step.selection.target();
            canonical.push_str(&format!(
                "],\"item\":{},\"kind\":\"{}\",\"name\":\"{}\",\"target\":[{x},{y},{z}]}}",
                step.selection.item_id(),
                step.selection.kind().as_str(),
                step.name
            ));
            require(
                canonical.len() + 2 <= MAX_PLAN_BYTES,
                "normalized furniture plan exceeds 16 KiB",
            )?;
        }
        canonical.push_str("]}");
        let bytes = canonical.into_bytes();
        let digest = hash(b"dfmcp-furniture-plan/1", &bytes);
        Ok(Self {
            steps,
            order,
            bytes,
            digest,
        })
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn digest(&self) -> Digest32 {
        self.digest
    }
    /// Steps sorted by exact ASCII name, independent of input ordering.
    pub fn steps(&self) -> &[FurnitureStep] {
        &self.steps
    }
    /// Lexically first ready step at every point in the dependency order.
    pub fn ordered_steps(&self) -> impl Iterator<Item = &FurnitureStep> {
        self.order.iter().map(|index| &self.steps[*index])
    }
    pub fn step(&self, name: &str) -> Option<&FurnitureStep> {
        self.steps.iter().find(|step| step.name == name)
    }
    pub fn validate_binding(&self, binding: &BuildBinding) -> Result<()> {
        let [width, height, depth] = binding.dimensions();
        require(
            self.steps.iter().all(|step| {
                let [x, y, z] = step.selection.target();
                x + 1 < width && y + 1 < height && z < depth
            }),
            "furniture target or its complete 3x3 context is outside the bound map",
        )
    }
}

/// Original complete intent tied to one native source and one placement journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchDefinition {
    plan: FurniturePlan,
    handoff: Option<Handoff>,
    binding: BuildBinding,
    journal_id: Digest32,
    id: Digest32,
}
impl BatchDefinition {
    pub fn new(plan: FurniturePlan, binding: BuildBinding, journal_id: Digest32) -> Result<Self> {
        plan.validate_binding(&binding)?;
        require(
            journal_id.as_bytes() != &[0; 32],
            "furniture batch needs an original journal identity",
        )?;
        let mut out = Self {
            plan,
            handoff: None,
            binding,
            journal_id,
            id: Digest32::from_bytes([0; 32]),
        };
        out.id = hash(b"dfmcp-furniture-batch-rust/1", &out.canonical_bytes());
        Ok(out)
    }
    /// A source-bound allocation retains its original constraints and selected
    /// evidence. The legacy constructor remains byte-for-byte DFMFBD01.
    pub fn from_handoff(
        handoff: Handoff,
        binding: BuildBinding,
        journal_id: Digest32,
    ) -> Result<Self> {
        handoff.validate_binding(&binding)?;
        let mut out = Self::new(handoff.plan().clone(), binding, journal_id)?;
        out.handoff = Some(handoff);
        out.id = hash(b"dfmcp-furniture-batch-rust/2", &out.canonical_bytes());
        Ok(out)
    }
    pub fn plan(&self) -> &FurniturePlan {
        &self.plan
    }
    pub fn binding(&self) -> &BuildBinding {
        &self.binding
    }
    pub fn handoff(&self) -> Option<&Handoff> {
        self.handoff.as_ref()
    }
    pub fn journal_id(&self) -> Digest32 {
        self.journal_id
    }
    pub fn id(&self) -> Digest32 {
        self.id
    }
    pub fn key(&self, step: &FurnitureStep) -> String {
        format!("fb-{}-{}", self.id, step.name)
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let binding = self.binding.encode();
        let mut out = if self.handoff.is_some() {
            HANDOFF_DEFINITION_MAGIC
        } else {
            DEFINITION_MAGIC
        }
        .to_vec();
        for field in [self.plan.canonical_bytes(), binding.as_slice()] {
            out.extend_from_slice(&(field.len() as u32).to_be_bytes());
            out.extend_from_slice(field);
        }
        out.extend_from_slice(self.journal_id.as_bytes());
        if let Some(handoff) = &self.handoff {
            out.extend_from_slice(&(handoff.canonical_bytes().len() as u32).to_be_bytes());
            out.extend_from_slice(handoff.canonical_bytes());
        }
        out
    }
    pub fn decode(raw: &[u8]) -> Result<Self> {
        require(
            raw.len() <= MAX_DEFINITION_BYTES,
            "furniture batch definition exceeds bound",
        )?;
        let mut reader = Bytes(raw);
        let magic = reader.take(8)?;
        require(
            magic == DEFINITION_MAGIC || magic == HANDOFF_DEFINITION_MAGIC,
            "invalid furniture batch definition",
        )?;
        let plan = FurniturePlan::decode(reader.field(MAX_PLAN_BYTES)?)?;
        let binding = BuildBinding::decode(reader.field(1024)?)?;
        let journal_id = Digest32::from_bytes(
            reader
                .take(32)?
                .try_into()
                .map_err(|_| invalid("invalid batch journal identity"))?,
        );
        let handoff = if magic == HANDOFF_DEFINITION_MAGIC {
            Some(Handoff::decode(reader.field(MAX_HANDOFF_BYTES)?)?)
        } else {
            None
        };
        require(
            reader.0.is_empty(),
            "trailing furniture batch definition bytes",
        )?;
        let out = if let Some(handoff) = handoff {
            require(
                handoff.plan() == &plan,
                "batch plan differs from original allocation",
            )?;
            Self::from_handoff(handoff, binding, journal_id)?
        } else {
            Self::new(plan, binding, journal_id)?
        };
        require(
            out.canonical_bytes() == raw,
            "noncanonical furniture batch definition",
        )?;
        Ok(out)
    }

    /// Validate complete retained original history; this does not perform I/O.
    /// The caller must first obtain the inventory under current authority/custody.
    pub fn audit(&self, inventory: &BuildInventory, stopped: bool) -> Result<BatchProgress> {
        if inventory.journal_id != self.journal_id
            || inventory.entries().len() > self.plan.steps.len()
        {
            return Err(corrupt(
                "furniture batch placement journal or record count differs",
            ));
        }
        let expected: BTreeSet<String> =
            self.plan.steps.iter().map(|step| self.key(step)).collect();
        let mut seen = BTreeSet::new();
        for entry in inventory.entries() {
            if !expected.contains(entry.plan().key()) || !seen.insert(entry.plan().key()) {
                return Err(corrupt(
                    "unplanned or duplicate placement in furniture batch journal",
                ));
            }
        }
        let mut out = BatchProgress {
            status: "ready",
            rows: Vec::with_capacity(self.plan.steps.len()),
            next_step: None,
            pending_step: None,
            placed: 0,
            total: self.plan.steps.len(),
            stopped,
            journal_id: inventory.journal_id,
            head: inventory.head,
        };
        let mut prefix_open = true;
        let mut last_after = None;
        for step in self.plan.ordered_steps() {
            let key = self.key(step);
            let entry = inventory.entry(&key);
            if entry.is_some() && !prefix_open {
                return Err(corrupt(
                    "furniture records are not the complete original ordered prefix",
                ));
            }
            let mut phase = None;
            if let Some(entry) = entry {
                self.check_capture(step.selection, entry.plan().before(), last_after)?;
                if let Some(record) = entry.native() {
                    record.verify_plan(entry.plan())?;
                    phase = Some(record.phase());
                }
            }
            let placed = entry.is_some_and(|entry| entry.state() == BuildState::Terminal)
                && phase == Some(BuildPhase::Placed);
            if placed {
                last_after = entry
                    .and_then(|entry| entry.native())
                    .and_then(|record| record.after());
                if last_after.is_none() {
                    return Err(corrupt(
                        "placed furniture child lacks its complete post-state",
                    ));
                }
                out.placed += 1;
            } else if prefix_open {
                prefix_open = false;
                match entry {
                    None if !stopped => out.next_step = Some(step.name.clone()),
                    None => {}
                    Some(entry)
                        if entry.state() == BuildState::Terminal
                            && phase == Some(BuildPhase::Refused) =>
                    {
                        out.status = "halted_refused";
                    }
                    Some(entry)
                        if entry.state() == BuildState::Terminal
                            && phase == Some(BuildPhase::Cancelled) =>
                    {
                        out.status = "halted_cancelled";
                    }
                    Some(_) => {
                        out.status = "pending_recovery";
                        out.pending_step = Some(step.name.clone());
                    }
                }
            }
            out.rows.push(BatchRow {
                step: step.name.clone(),
                key,
                native_plan_digest: entry.map(|entry| entry.plan().digest()),
                state: entry.map_or("not_started", |entry| entry.state().as_str()),
                outcome: phase.map(BuildPhase::as_str),
            });
        }
        if prefix_open {
            out.status = "all_placed";
        } else if stopped && out.status == "ready" {
            out.status = "stopped";
        }
        Ok(out)
    }

    fn check_capture(
        &self,
        selection: BuildSelection,
        capture: &BuildCapture,
        last_after: Option<&BuildCapture>,
    ) -> Result<()> {
        if !self.binding.capture_matches(capture) || capture.selection() != selection {
            return Err(corrupt(
                "furniture child source or exact selection differs from original batch",
            ));
        }
        if let Some(handoff) = &self.handoff {
            handoff
                .validate_capture(&self.binding, capture)
                .map_err(|_| {
                    corrupt("furniture child violates original allocation source or constraints")
                })?;
        }
        if let Some(prior) = last_after
            && (capture.tick() < prior.tick()
                || capture.sequence() < prior.sequence()
                || capture.next_building_id() < prior.next_building_id()
                || capture.next_job_id() < prior.next_job_id())
        {
            return Err(corrupt(
                "furniture child clock, sequence or native ID horizon regressed",
            ));
        }
        Ok(())
    }

    /// Additional next-child constraints, never a dispatch permit. A still-owned
    /// preparation may pass, but terminal or retired work cannot be advanced.
    pub fn validate_next(
        &self,
        inventory: &BuildInventory,
        stopped: bool,
        key: &str,
        selection: BuildSelection,
        capture: Option<&BuildCapture>,
    ) -> Result<()> {
        let progress = self.audit(inventory, stopped)?;
        require(!stopped, "furniture batch is permanently stopped")?;
        let name = progress
            .next_step
            .as_ref()
            .or(progress.pending_step.as_ref())
            .ok_or_else(|| invalid("furniture batch has no next placement"))?;
        let step = self
            .plan
            .step(name)
            .ok_or_else(|| corrupt("furniture next step is absent"))?;
        require(
            self.key(step) == key && step.selection == selection,
            "furniture request substitutes the original next step",
        )?;
        if let Some(entry) = inventory.entry(key) {
            require(
                matches!(
                    entry.state(),
                    BuildState::Intent | BuildState::Prepared | BuildState::DispatchStarted
                ) && !entry.cancel_requested(),
                "furniture child requires original-key recovery",
            )?;
            if let Some(capture) = capture {
                require(
                    capture == entry.plan().before(),
                    "prepared furniture child capture changed",
                )?;
            }
        }
        if let Some(capture) = capture {
            let mut previous = None;
            for original in self.plan.ordered_steps() {
                if original.name == step.name {
                    break;
                }
                previous = inventory
                    .entry(&self.key(original))
                    .and_then(|entry| entry.native())
                    .and_then(|record| record.after());
            }
            self.check_capture(selection, capture, previous)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchRow {
    pub step: String,
    pub key: String,
    pub native_plan_digest: Option<Digest32>,
    pub state: &'static str,
    pub outcome: Option<&'static str>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchProgress {
    pub status: &'static str,
    pub rows: Vec<BatchRow>,
    pub next_step: Option<String>,
    pub pending_step: Option<String>,
    pub placed: usize,
    pub total: usize,
    pub stopped: bool,
    pub journal_id: Digest32,
    pub head: Digest32,
}

struct Bytes<'a>(&'a [u8]);
impl<'a> Bytes<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        require(
            length <= self.0.len(),
            "truncated furniture batch definition",
        )?;
        let (value, remaining) = self.0.split_at(length);
        self.0 = remaining;
        Ok(value)
    }
    fn field(&mut self, maximum: usize) -> Result<&'a [u8]> {
        let length = u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| invalid("invalid furniture batch field length"))?,
        ) as usize;
        require(length <= maximum, "furniture batch field exceeds bound")?;
        self.take(length)
    }
}

fn check_depth(raw: &[u8]) -> Result<()> {
    let (mut depth, mut quoted, mut escaped) = (0u8, false, false);
    for &byte in raw {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'[' | b'{' => {
                    depth += 1;
                    require(depth <= 8, "furniture plan nesting exceeds eight levels")?;
                }
                b']' | b'}' => {
                    depth = depth
                        .checked_sub(1)
                        .ok_or_else(|| invalid("invalid furniture JSON nesting"))?
                }
                _ => {}
            }
        }
    }
    require(depth == 0 && !quoted, "incomplete furniture JSON")
}

/// A schema-specific JSON reader: no arbitrary tree or unbounded collection.
struct Json<'a>(&'a [u8]);
impl Json<'_> {
    fn whitespace(&mut self) {
        while self
            .0
            .first()
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.0 = &self.0[1..];
        }
    }
    fn consume(&mut self, byte: u8) -> bool {
        self.whitespace();
        if self.0.first() == Some(&byte) {
            self.0 = &self.0[1..];
            true
        } else {
            false
        }
    }
    fn symbol(&mut self, byte: u8) -> Result<()> {
        require(self.consume(byte), "invalid closed furniture JSON")
    }
    fn next_byte(&mut self) -> Result<u8> {
        let (&byte, remaining) = self
            .0
            .split_first()
            .ok_or_else(|| invalid("incomplete furniture JSON"))?;
        self.0 = remaining;
        Ok(byte)
    }
    fn string(&mut self, maximum: usize) -> Result<String> {
        self.symbol(b'"')?;
        let mut out = String::new();
        loop {
            let mut byte = self.next_byte()?;
            if byte == b'"' {
                return Ok(out);
            }
            require(
                (0x20..0x80).contains(&byte),
                "furniture JSON strings must decode to their ASCII catalog",
            )?;
            if byte == b'\\' {
                byte = match self.next_byte()? {
                    b'"' => b'"',
                    b'\\' => b'\\',
                    b'/' => b'/',
                    b'b' => 8,
                    b'f' => 12,
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'u' => {
                        let mut code = 0u16;
                        for _ in 0..4 {
                            let digit = char::from(self.next_byte()?)
                                .to_digit(16)
                                .ok_or_else(|| invalid("invalid furniture JSON Unicode escape"))?;
                            code = code * 16 + digit as u16;
                        }
                        require(code < 0x80, "furniture strings require ASCII values")?;
                        code as u8
                    }
                    _ => return Err(invalid("invalid furniture JSON escape")),
                };
            }
            require(out.len() < maximum, "furniture JSON string exceeds bound")?;
            out.push(char::from(byte));
        }
    }
    fn number(&mut self) -> Result<u32> {
        self.whitespace();
        let negative = self.consume(b'-');
        let first = self.next_byte()?;
        require(first.is_ascii_digit(), "furniture values require integers")?;
        let mut value = u32::from(first - b'0');
        let mut count = 1;
        while self.0.first().is_some_and(u8::is_ascii_digit) {
            require(
                first != b'0',
                "noncanonical leading zero in furniture integer",
            )?;
            let digit = self.next_byte()? - b'0';
            value = value
                .checked_mul(10)
                .and_then(|n| n.checked_add(u32::from(digit)))
                .ok_or_else(|| invalid("furniture integer exceeds bound"))?;
            count += 1;
        }
        require(
            !negative || (count == 1 && value == 0),
            "negative furniture integer",
        )?;
        require(
            !self
                .0
                .first()
                .is_some_and(|b| matches!(b, b'.' | b'e' | b'E')),
            "floating-point furniture values are forbidden",
        )?;
        Ok(value)
    }
    fn steps(&mut self) -> Result<Vec<FurnitureStep>> {
        self.symbol(b'[')?;
        let mut out = Vec::new();
        if self.consume(b']') {
            return Ok(out);
        }
        loop {
            require(out.len() < MAX_STEPS, "furniture plan exceeds 32 steps")?;
            out.push(self.step()?);
            if self.consume(b']') {
                return Ok(out);
            }
            self.symbol(b',')?;
        }
    }
    fn dependencies(&mut self) -> Result<Vec<String>> {
        self.symbol(b'[')?;
        let mut out = Vec::new();
        if self.consume(b']') {
            return Ok(out);
        }
        loop {
            require(
                out.len() < MAX_STEPS - 1,
                "furniture dependency bound exceeded",
            )?;
            out.push(self.string(48)?);
            if self.consume(b']') {
                return Ok(out);
            }
            self.symbol(b',')?;
        }
    }
    fn target(&mut self) -> Result<[u32; 3]> {
        self.symbol(b'[')?;
        let x = self.number()?;
        self.symbol(b',')?;
        let y = self.number()?;
        self.symbol(b',')?;
        let z = self.number()?;
        self.symbol(b']')?;
        Ok([x, y, z])
    }
    fn step(&mut self) -> Result<FurnitureStep> {
        self.symbol(b'{')?;
        let (mut name, mut kind, mut item, mut target, mut after) = (None, None, None, None, None);
        if !self.consume(b'}') {
            loop {
                let field = self.string(16)?;
                self.symbol(b':')?;
                match field.as_str() {
                    "name" if name.is_none() => name = Some(self.string(48)?),
                    "kind" if kind.is_none() => {
                        kind = Some(match self.string(8)?.as_str() {
                            "bed" => BuildKind::Bed,
                            "chair" => BuildKind::Chair,
                            "table" => BuildKind::Table,
                            _ => return Err(invalid("unsupported furniture kind")),
                        })
                    }
                    "item" if item.is_none() => item = Some(self.number()?),
                    "target" if target.is_none() => target = Some(self.target()?),
                    "after" if after.is_none() => after = Some(self.dependencies()?),
                    _ => return Err(invalid("unknown or duplicate furniture step field")),
                }
                if self.consume(b'}') {
                    break;
                }
                self.symbol(b',')?;
            }
        }
        let missing = || invalid("furniture step lacks required fields");
        Ok(FurnitureStep {
            name: name.ok_or_else(missing)?,
            selection: BuildSelection::new(
                kind.ok_or_else(missing)?,
                item.ok_or_else(missing)?,
                target.ok_or_else(missing)?,
            )?,
            after: after.unwrap_or_default(),
        })
    }
}

#[cfg(test)]
#[path = "furniture_batch_tests.rs"]
mod tests;
