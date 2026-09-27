//! Exact Python-compatible DFMCPG01 and DFMCPS01 evidence codecs.

use super::*;
use crate::build_placement::{BuildPhase, MAX_NATIVE_TICK, MAX_RECORD_BYTES};
use crate::live_operations::OperationsProfile;

impl Timing {
    fn validate(self, before_tick: u64) -> Result<()> {
        require(
            self.deadline > before_tick
                && self.deadline <= MAX_NATIVE_TICK
                && (1..=403_200).contains(&self.interval)
                && (2..=64).contains(&self.stable_samples)
                && (1..=4_032_000).contains(&self.stable_span)
                && (self.interval..=4_032_000).contains(&self.max_gap)
                && (self.stable_samples..=MAX_OBSERVATIONS).contains(&self.max_observations),
            "invalid fixed construction timing",
        )?;
        let needed = self
            .stable_span
            .max(u64::from(self.stable_samples - 1) * u64::from(self.interval));
        require(
            needed < self.deadline - before_tick,
            "construction condition cannot fit before fixed deadline",
        )
    }
    fn encode(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.deadline.to_be_bytes());
        out.extend_from_slice(&self.interval.to_be_bytes());
        out.extend_from_slice(&self.stable_samples.to_be_bytes());
        out.extend_from_slice(&self.stable_span.to_be_bytes());
        out.extend_from_slice(&self.max_gap.to_be_bytes());
        out.extend_from_slice(&self.max_observations.to_be_bytes());
    }
}

impl Goal {
    pub fn new(mut records: Vec<BuildRecord>, timing: Timing) -> Result<Self> {
        require(
            (1..=MAX_TARGETS).contains(&records.len()),
            "construction plan requires 1..32 original placed receipts",
        )?;
        let first = records
            .first()
            .ok_or_else(|| invalid("empty construction goal"))?;
        let source = first.plan().before();
        let mut keys = BTreeSet::new();
        let mut buildings = BTreeSet::new();
        let mut items = BTreeSet::new();
        let mut jobs = BTreeSet::new();
        let mut positions = BTreeSet::new();
        for record in &records {
            require(
                record.phase() == BuildPhase::Placed,
                "construction requires original Placed receipts",
            )?;
            let before = record.plan().before();
            require(
                before.same_source(source),
                "construction receipts belong to different fortress incarnations",
            )?;
            timing.validate(before.tick())?;
            let p = record
                .insertion()
                .ok_or_else(|| invalid("placed receipt lacks insertion"))?;
            require(
                keys.insert(record.plan().key())
                    && buildings.insert(p.building_id())
                    && items.insert(p.item_id())
                    && jobs.insert(p.job_id())
                    && positions.insert(p.position()),
                "duplicate construction key, building, item, job or target",
            )?;
        }
        records.sort_by_key(|record| record.insertion().map(|p| p.building_id()));
        let mut canonical = b"DFMCPG01".to_vec();
        write_records(
            &mut canonical,
            records.iter().map(|record| record.canonical_bytes()),
        )?;
        timing.encode(&mut canonical);
        require(canonical.len() <= MAX_GOAL, "oversized construction goal")?;
        let mut hashed = b"dfmcp.construction-plan-goal/1\0".to_vec();
        hashed.extend_from_slice(&canonical);
        Ok(Self {
            records,
            timing,
            digest: Digest32::of_bytes(&hashed),
            canonical,
        })
    }
    pub fn decode(raw: &[u8]) -> Result<Self> {
        let mut r = Cursor::new(raw, MAX_GOAL)?;
        require(
            r.take(8)? == b"DFMCPG01",
            "wrong construction goal generation",
        )?;
        let records = r
            .records()?
            .iter()
            .map(|raw| BuildRecord::decode(raw))
            .collect::<Result<Vec<_>>>()?;
        let timing = Timing {
            deadline: r.u64()?,
            interval: r.u32()?,
            stable_samples: r.u32()?,
            stable_span: r.u64()?,
            max_gap: r.u32()?,
            max_observations: r.u32()?,
        };
        r.finish()?;
        let goal = Self::new(records, timing)?;
        require(
            goal.canonical_bytes() == raw,
            "noncanonical construction goal",
        )?;
        Ok(goal)
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
    pub fn digest(&self) -> Digest32 {
        self.digest
    }
    pub fn records(&self) -> &[BuildRecord] {
        &self.records
    }
    pub fn timing(&self) -> Timing {
        self.timing
    }
}

impl Manifest {
    pub fn validate(&self) -> Result<()> {
        require(
            self.generation > 0 && self.generation < u64::MAX,
            "invalid construction source generation",
        )?;
        for value in [&self.df_version, &self.dfhack_version] {
            require(
                !value.is_empty() && value.len() <= 128 && !value.contains('\0'),
                "invalid construction source software text",
            )?;
        }
        Ok(())
    }
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut out = self.generation.to_be_bytes().to_vec();
        field(&mut out, self.df_version.as_bytes());
        field(&mut out, self.dfhack_version.as_bytes());
        Ok(out)
    }
}

fn field(out: &mut Vec<u8>, raw: &[u8]) {
    out.extend_from_slice(&(raw.len() as u16).to_be_bytes());
    out.extend_from_slice(raw);
}
fn write_records<'a>(
    out: &mut Vec<u8>,
    values: impl ExactSizeIterator<Item = &'a [u8]>,
) -> Result<()> {
    require(
        (1..=MAX_TARGETS).contains(&values.len()),
        "construction sample requires 1..32 receipt bytes",
    )?;
    out.push(values.len() as u8);
    for raw in values {
        require(
            !raw.is_empty() && raw.len() <= MAX_RECORD_BYTES,
            "invalid construction sample receipt size",
        )?;
        field(out, raw);
    }
    Ok(())
}

impl LinkedSample {
    fn check_shape(&self) -> Result<usize> {
        self.before.validate()?;
        self.operations.validate()?;
        self.after.validate()?;
        require(
            !self.capture.is_empty() && self.capture.len() <= MAX_CAPTURE,
            "invalid complete construction capture size",
        )?;
        let mut size = 8 + 3 * 12 + 4 + self.capture.len();
        for manifest in [&self.before, &self.operations, &self.after] {
            size += manifest.df_version.len() + manifest.dfhack_version.len();
        }
        for records in [&self.before_records, &self.after_records] {
            require(
                (1..=MAX_TARGETS).contains(&records.len()),
                "invalid construction receipt count",
            )?;
            size += 1;
            for raw in records {
                require(
                    !raw.is_empty() && raw.len() <= MAX_RECORD_BYTES,
                    "invalid construction receipt size",
                )?;
                size += raw.len() + 2;
            }
        }
        require(size <= MAX_SAMPLE, "oversized construction sample")?;
        Ok(size)
    }
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        let size = self.check_shape()?;
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(b"DFMCPS01");
        out.extend_from_slice(&self.before.canonical_bytes()?);
        write_records(&mut out, self.before_records.iter().map(Vec::as_slice))?;
        out.extend_from_slice(&self.operations.canonical_bytes()?);
        out.extend_from_slice(&(self.capture.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.capture);
        out.extend_from_slice(&self.after.canonical_bytes()?);
        write_records(&mut out, self.after_records.iter().map(Vec::as_slice))?;
        Ok(out)
    }
    pub fn decode(raw: &[u8]) -> Result<Self> {
        let mut r = Cursor::new(raw, MAX_SAMPLE)?;
        require(
            r.take(8)? == b"DFMCPS01",
            "wrong construction sample generation",
        )?;
        let before = r.manifest()?;
        let before_records = r.records()?;
        let operations = r.manifest()?;
        let size = r.u32()? as usize;
        require(
            (1..=MAX_CAPTURE).contains(&size),
            "invalid complete construction capture size",
        )?;
        let capture = r.take(size)?.to_vec();
        let after = r.manifest()?;
        let after_records = r.records()?;
        r.finish()?;
        let sample = Self {
            before,
            before_records,
            operations,
            capture,
            after,
            after_records,
        };
        require(
            sample.check_shape()? == raw.len(),
            "noncanonical construction sample",
        )?;
        Ok(sample)
    }
    pub fn validate(
        &self,
        goal: &Goal,
        context: &OperationContext,
    ) -> Result<LiveOperationsObservation> {
        let mut work = Work::new(goal, context, 0)?;
        self.validate_work(goal, &mut work)
    }
    pub(super) fn validate_work(
        &self,
        goal: &Goal,
        work: &mut Work,
    ) -> Result<LiveOperationsObservation> {
        work.charge(1)?;
        let size = self.check_shape()?;
        if size as u64 > work.context.budget.max_bytes {
            return Err(bounded("construction sample exceeds byte allowance"));
        }
        require(
            self.before_records == self.after_records
                && self.before_records.len() == goal.records.len()
                && self
                    .before_records
                    .iter()
                    .zip(&goal.records)
                    .all(|(raw, record)| raw == record.canonical_bytes()),
            "every original construction receipt must be retained in canonical order",
        )?;
        require(
            self.before == self.after
                && goal.records.first().is_some_and(|record| {
                    self.before.generation == record.plan().before().generation()
                }),
            "construction placement source changed across observation",
        )?;
        require(
            self.before.df_version == self.operations.df_version
                && self.before.dfhack_version == self.operations.dfhack_version,
            "construction native software families disagree",
        )?;
        // Fixed byte/count bounds in the existing codec precede allocations.
        // Bytes are bounded separately above (and reserved by the journal).
        // Charge bounded capture chunks, then repeat authority/deadline checks
        // and charge all roster validation work after the one semantic decode.
        work.charge(self.capture.len().div_ceil(64 * 1024) as u64)?;
        let observed = LiveOperationsObservation::decode_profile(
            &self.capture,
            self.operations.generation,
            self.operations.df_version.clone(),
            self.operations.dfhack_version.clone(),
            OperationsProfile::PagedV1_4,
        )?;
        work.observe_tick(observed.jobs.tick().get())?;
        let entities =
            1 + observed.jobs.jobs.len() + observed.buildings.len() + observed.items.len();
        if entities > work.context.budget.max_entities as usize {
            return Err(bounded("construction capture exceeds entity allowance"));
        }
        work.charge(6 * (entities + observed.attachments.len()) as u64)?;
        // The generic operations decoder preserves enum keys as data. This
        // receipt profile additionally requires a bijection within each roster,
        // matching the independent Python completion decoder.
        enum_identity(
            observed
                .jobs
                .jobs
                .iter()
                .map(|v| (v.job_type, v.type_key.as_str())),
            work,
        )?;
        enum_identity(
            observed
                .buildings
                .iter()
                .map(|v| (v.building_type, v.type_key.as_str())),
            work,
        )?;
        enum_identity(
            observed
                .items
                .iter()
                .map(|v| (v.item_type, v.type_key.as_str())),
            work,
        )?;
        work.charge(1)?;
        Ok(observed)
    }
}

fn enum_identity<'a>(values: impl Iterator<Item = (i32, &'a str)>, work: &mut Work) -> Result<()> {
    let mut types = BTreeMap::new();
    let mut keys = BTreeMap::new();
    for (native, key) in values {
        work.charge(1)?;
        require(
            types.insert(native, key).is_none_or(|prior| prior == key)
                && keys.insert(key, native).is_none_or(|prior| prior == native),
            "inconsistent construction native enum identity",
        )?;
    }
    Ok(())
}

struct Cursor<'a> {
    raw: &'a [u8],
    offset: usize,
}
impl<'a> Cursor<'a> {
    fn new(raw: &'a [u8], maximum: usize) -> Result<Self> {
        require(
            !raw.is_empty() && raw.len() <= maximum,
            "construction evidence exceeds byte bound",
        )?;
        Ok(Self { raw, offset: 0 })
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or_else(|| invalid("construction field overflow"))?;
        let value = self
            .raw
            .get(self.offset..end)
            .ok_or_else(|| invalid("truncated construction evidence"))?;
        self.offset = end;
        Ok(value)
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| invalid("invalid construction u16"))?,
        ))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| invalid("invalid construction u32"))?,
        ))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| invalid("invalid construction u64"))?,
        ))
    }
    fn text(&mut self) -> Result<String> {
        let size = self.u16()? as usize;
        require(
            (1..=128).contains(&size),
            "invalid construction manifest text bound",
        )?;
        let value = std::str::from_utf8(self.take(size)?)
            .map_err(|_| invalid("invalid construction manifest UTF-8"))?;
        require(!value.contains('\0'), "NUL in construction manifest text")?;
        Ok(value.to_owned())
    }
    fn manifest(&mut self) -> Result<Manifest> {
        let value = Manifest {
            generation: self.u64()?,
            df_version: self.text()?,
            dfhack_version: self.text()?,
        };
        value.validate()?;
        Ok(value)
    }
    fn records(&mut self) -> Result<Vec<Vec<u8>>> {
        let count = usize::from(self.take(1)?[0]);
        require(
            (1..=MAX_TARGETS).contains(&count),
            "invalid construction receipt count",
        )?;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            let size = self.u16()? as usize;
            require(
                (1..=MAX_RECORD_BYTES).contains(&size),
                "invalid construction receipt byte bound",
            )?;
            values.push(self.take(size)?.to_vec());
        }
        Ok(values)
    }
    fn finish(self) -> Result<()> {
        require(
            self.offset == self.raw.len(),
            "trailing construction evidence bytes",
        )
    }
}
