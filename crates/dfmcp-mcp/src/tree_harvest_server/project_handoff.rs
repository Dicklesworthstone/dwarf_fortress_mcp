//! Compact historical transfer record. Deserialization never imports custody or
//! proves current supply: the Workshop owner must reopen the exact Tree journal
//! and independent acknowledgement in Offline mode and verify this descriptor.
use super::*;
use dfmcp_adapter::tree_harvest::journal::{CLOCK_ORIGIN_BYTES, TreeMaterialOrigin};

const SCHEMA: &str = "dfmcp.tree-project-handoff/1";
const MAX_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct TreeProjectHandoff {
    directory: PathBuf,
    binding: Binding,
    root: [u32; 3],
    acknowledged: WorkshopPin,
    original_key: String,
    original_plan_digest: Digest32,
    deadline_tick: u64,
    origin_witness: Digest32,
    achieved_tick: u64,
    progress: Vec<u8>,
    high_tick: u64,
    high_generation: u64,
    high_sequence: u64,
    high_item: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceInput {
    endpoint: String,
    world_folder: String,
    site_id: u32,
    df_version: String,
    dfhack_version: String,
    generation: u64,
    map_dimensions: [u32; 3],
    scope: [i32; 6],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: String,
    directory: PathBuf,
    binding: SourceInput,
    root: [u32; 3],
    acknowledged: PinInput,
    original_key: String,
    original_plan_digest: String,
    deadline_tick: u64,
    origin_witness: String,
    historical_supply_observed_tick: u64,
    progress_hex: String,
    high_tick: u64,
    high_generation: u64,
    high_sequence: u64,
    high_next_item_id: u32,
}

impl TreeProjectHandoff {
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }
    pub(crate) fn binding(&self) -> &Binding {
        &self.binding
    }
    pub(crate) fn root(&self) -> [u32; 3] {
        self.root
    }
    pub(crate) fn acknowledged(&self) -> WorkshopPin {
        self.acknowledged
    }
    pub(crate) fn original_key(&self) -> &str {
        &self.original_key
    }
    pub(crate) fn original_plan_digest(&self) -> Digest32 {
        self.original_plan_digest
    }
    pub(crate) fn project_deadline_tick(&self) -> u64 {
        self.deadline_tick
    }
    pub(crate) fn origin_witness(&self) -> Digest32 {
        self.origin_witness
    }
    pub(crate) fn high_tick(&self) -> u64 {
        self.high_tick
    }
    pub(crate) fn high_generation(&self) -> u64 {
        self.high_generation
    }
    pub(crate) fn high_sequence(&self) -> u64 {
        self.high_sequence
    }
    pub(crate) fn high_next_item_id(&self) -> u32 {
        self.high_item
    }

    pub(crate) fn to_json(&self) -> Value {
        let b = &self.binding;
        json!({"schema":SCHEMA,"directory":self.directory,
            "binding":{"endpoint":b.endpoint.to_string(),"world_folder":b.source.folder(),
                "site_id":b.source.site(),"df_version":b.df_version,"dfhack_version":b.dfhack_version,
                "generation":b.generation,"map_dimensions":b.dimensions,
                "scope":[b.scope.min.x,b.scope.min.y,b.scope.min.z,b.scope.max.x,b.scope.max.y,b.scope.max.z]},
            "root":self.root,"acknowledged":pin_json(self.acknowledged),
            "original_key":self.original_key,"original_plan_digest":self.original_plan_digest.to_hex(),
            "deadline_tick":self.deadline_tick,"origin_witness":self.origin_witness.to_hex(),
            "historical_supply_observed_tick":self.achieved_tick,"progress_hex":hex(&self.progress),
            "high_tick":self.high_tick,"high_generation":self.high_generation,
            "high_sequence":self.high_sequence,"high_next_item_id":self.high_item})
    }

    /// Parse a bounded historical reference only. `verify_origin` remains
    /// mandatory under the importer's exclusive Offline custody before I/O.
    pub(crate) fn from_json(value: &Value) -> Result<Self> {
        if value.to_string().len() > MAX_BYTES {
            return Err(invalid("Tree handoff exceeds its bound"));
        }
        let input: Input = serde_json::from_value(value.clone())
            .map_err(|_| invalid("invalid closed Tree project handoff"))?;
        let path = input
            .directory
            .to_str()
            .ok_or_else(|| invalid("Tree handoff path must be UTF-8"))?;
        if input.schema != SCHEMA
            || !input.directory.is_absolute()
            || !(2..=3800).contains(&path.len())
            || path[1..]
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == ".." || s.chars().any(char::is_control))
            || input.progress_hex.is_empty()
            || input.progress_hex.len() > 8192
            || input.progress_hex.len() % 2 != 0
            || !input
                .progress_hex
                .bytes()
                .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
            || input.original_key.is_empty()
            || input.original_key.len() > 160
            || input.original_key.chars().any(char::is_control)
            || input.deadline_tick > MAX_NATIVE_TICK
            || input.high_tick >= input.deadline_tick
            || input.historical_supply_observed_tick > input.high_tick
            || input.high_generation != input.binding.generation
            || input.high_sequence == u64::MAX
            || input.high_next_item_id > i32::MAX as u32
        {
            return Err(invalid("invalid Tree handoff historical bounds"));
        }
        let endpoint = dfmcp_adapter::parse_loopback_endpoint(&input.binding.endpoint)?;
        if endpoint.to_string() != input.binding.endpoint {
            return Err(invalid("noncanonical Tree handoff endpoint"));
        }
        let a = input.binding.scope;
        let binding = Binding {
            endpoint,
            source: FortressIdentity::new(&input.binding.world_folder, input.binding.site_id)?,
            generation: input.binding.generation,
            dimensions: input.binding.map_dimensions,
            scope: MapCuboid::new(
                MapCoord::new(a[0], a[1], a[2]),
                MapCoord::new(a[3], a[4], a[5]),
            )?,
            df_version: input.binding.df_version,
            dfhack_version: input.binding.dfhack_version,
        };
        binding.validate()?;
        if (0..3).any(|i| input.root[i] >= binding.dimensions[i]) {
            return Err(invalid("Tree handoff root outside source"));
        }
        let acknowledged = WorkshopPin {
            journal_id: digest(&input.acknowledged.journal_id)?,
            head: digest(&input.acknowledged.head)?,
            frames: input.acknowledged.frames,
        };
        let original_plan_digest = digest(&input.original_plan_digest)?;
        let origin_witness = digest(&input.origin_witness)?;
        if acknowledged.journal_id == Digest32::ZERO
            || acknowledged.head == Digest32::ZERO
            || !(1..=40).contains(&acknowledged.frames)
            || original_plan_digest == Digest32::ZERO
            || origin_witness == Digest32::ZERO
        {
            return Err(invalid("Tree handoff lost exact original custody identity"));
        }
        let progress = input
            .progress_hex
            .as_bytes()
            .chunks_exact(2)
            .map(|v| {
                let n = |v: u8| {
                    if v.is_ascii_digit() {
                        v - b'0'
                    } else {
                        v - b'a' + 10
                    }
                };
                n(v[0]) * 16 + n(v[1])
            })
            .collect();
        Ok(Self {
            directory: input.directory,
            binding,
            root: input.root,
            acknowledged,
            original_key: input.original_key,
            original_plan_digest,
            deadline_tick: input.deadline_tick,
            origin_witness,
            achieved_tick: input.historical_supply_observed_tick,
            progress,
            high_tick: input.high_tick,
            high_generation: input.high_generation,
            high_sequence: input.high_sequence,
            high_item: input.high_next_item_id,
        })
    }

    pub(crate) fn verify_origin(&self, origin: &TreeMaterialOrigin) -> Result<()> {
        let original = origin.original();
        if origin.binding() != &self.binding
            || origin.pin() != self.acknowledged
            || original.plan.before.root != self.root
            || original.plan.key() != self.original_key
            || original.plan.digest != self.original_plan_digest
            || original.deadline_tick != self.deadline_tick
            || origin.witness() != self.origin_witness
            || origin.achieved_tick() != Some(self.achieved_tick)
        {
            return Err(denied(
                "Tree handoff differs from independently verified original custody",
            ));
        }
        let progress = Progress::decode(&self.progress, &original.plan)?;
        if !progress.supply_observed()
            || progress.generation != self.binding.generation
            || progress.tick != self.high_tick
            || progress.sequence != self.high_sequence
            || progress.next_item != self.high_item
            || self.high_generation != self.binding.generation
            || self.high_tick < origin.high_tick()
            || self.high_sequence < origin.high_sequence()
            || self.high_item < origin.high_item()
            || self.high_tick >= self.deadline_tick
            || self.achieved_tick > self.deadline_tick
        {
            return Err(denied(
                "Tree handoff lost continuous timely original supply evidence",
            ));
        }
        Ok(())
    }
}

impl Runtime {
    /// Export only after a new source-bound supply read, complete retained
    /// custody verification on both sides, and typed clock settlement. The
    /// parent must persist this descriptor before releasing this owner.
    pub(crate) fn project_handoff(&mut self, remaining: WorkBudget) -> Result<TreeProjectHandoff> {
        let start = Instant::now();
        let ceiling = self.budget;
        let mut source_entered = false;
        let result = (|| {
            self.project_current()?;
            self.budget = narrow(ceiling, remaining, start)?;
            if self.mode == Mode::Offline || self.project.fenced || self.journal.dispatchable() {
                return Err(denied(
                    "Tree project transfer requires a continuous resolved original",
                ));
            }
            let mut current = self.current()?;
            let mut work = TurnWork::new(&current, start)?;
            let clock_origin = self
                .journal
                .clock_origin(&mut self.acks, &work.reserve(&current, CLOCK_ORIGIN_BYTES)?)?;
            self.high_tick = self.high_tick.max(clock_origin.high_tick());
            self.high_sequence = self.high_sequence.max(clock_origin.high_sequence());
            self.project.high_generation = self
                .project
                .high_generation
                .max(clock_origin.high_generation());
            self.project.high_item = self.project.high_item.max(clock_origin.high_item());
            self.project.fenced |= clock_origin.continuity_lost();
            current.anchor.tick = GameTick(self.high_tick);
            if clock_origin.pending_sample()
                || clock_origin.continuity_lost()
                || clock_origin.achieved_tick().is_none()
                || Some(clock_origin.original().deadline_tick) != self.config.project_deadline
                || clock_origin.binding() != &self.config.binding
            {
                return Err(denied(
                    "Tree original has no continuous, acknowledged completion for this project",
                ));
            }
            self.config.project_effect_deadline(self.high_tick)?;
            if let Some(clock) = &mut self.clock {
                clock.project_handoff(
                    &self.config.binding,
                    &self.journal,
                    clock_origin.clone(),
                    &work.reserve(&current, 3 * dfmcp_adapter::tree_harvest::clock::VIEW_BYTES)?,
                )?;
                self.high_tick = self.high_tick.max(clock.high_tick());
                current.anchor.tick = GameTick(self.high_tick);
            }
            let origin = self.journal.material_checkpoint(
                &mut self.acks,
                &work.reserve(&current, CLOCK_ORIGIN_BYTES)?,
            )?;
            if origin.pin() != clock_origin.pin()
                || origin.original() != clock_origin.original()
                || origin.achieved_tick() != clock_origin.achieved_tick()
            {
                return Err(denied("Tree original changed during project transfer"));
            }
            self.config.project_effect_deadline(self.high_tick)?;
            source_entered = true;
            let mut source = self
                .config
                .connect(&work.reserve(&current, CONNECT_BYTES)?, Mode::Recover)?;
            self.project.high_generation = self
                .project
                .high_generation
                .max(source.binding().generation);
            self.project.fenced |= source.binding() != origin.binding();
            if self.project.fenced {
                source.fence();
                return Err(denied("Tree project source incarnation changed"));
            }
            source.require_progress_at_or_after(self.high_tick);
            let sampled = source.progress(
                &origin.original().plan,
                &work.reserve(&current, CONNECT_BYTES)?,
            );
            // The native client rejects a mismatched reply incarnation. Keep
            // every decoded floor before later refusal; an entered-source
            // error below stickily fences even an undecodable response.
            if let Some(progress) = source.consumed_progress.take() {
                self.retain_project_progress(&progress);
            }
            source.fence();
            let progress = sampled?;
            current.anchor.tick = GameTick(self.high_tick);
            let after = self.journal.material_checkpoint(
                &mut self.acks,
                &work.reserve(&current, CLOCK_ORIGIN_BYTES)?,
            )?;
            if after != origin {
                return Err(denied(
                    "Tree original custody changed after current supply read",
                ));
            }
            if let Some(clock) = &mut self.clock {
                clock.project_handoff(
                    &self.config.binding,
                    &self.journal,
                    clock_origin,
                    &work.reserve(&current, 3 * dfmcp_adapter::tree_harvest::clock::VIEW_BYTES)?,
                )?;
                self.high_tick = self.high_tick.max(clock.high_tick());
            }
            self.project_current()?;
            self.config.project_effect_deadline(self.high_tick)?;
            if self.project.fenced || progress.tick != self.high_tick || !progress.supply_observed()
            {
                return Err(denied(
                    "current Tree evidence does not support continuous project transfer",
                ));
            }
            let handoff = TreeProjectHandoff {
                directory: self.config.input.directory.clone(),
                binding: origin.binding().clone(),
                root: self.config.input.root,
                acknowledged: origin.pin(),
                original_key: origin.original().plan.key().to_owned(),
                original_plan_digest: origin.original().plan.digest,
                deadline_tick: origin.original().deadline_tick,
                origin_witness: origin.witness(),
                achieved_tick: origin
                    .achieved_tick()
                    .ok_or_else(|| denied("original supply achievement unavailable"))?,
                progress: progress.bytes().to_vec(),
                high_tick: self.high_tick,
                high_generation: self.project.high_generation,
                high_sequence: self.high_sequence,
                high_item: self.project.high_item,
            };
            handoff.verify_origin(&origin)?;
            TreeProjectHandoff::from_json(&handoff.to_json())?;
            Ok(handoff)
        })();
        self.budget = ceiling;
        if result.is_err() {
            if source_entered {
                self.project.fenced = true;
                if let Some(clock) = &mut self.clock {
                    clock.fence_advancement();
                }
            }
            self.withdraw();
        }
        result
    }
}
