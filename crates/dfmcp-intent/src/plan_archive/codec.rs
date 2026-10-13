//! Closed lossless schema. Tags are archive-specific, not native enum ordinals.
use super::*;

impl<F: FnMut() -> Result<()>> Writer<'_, F> {
    fn coord(&mut self, value: MapCoord) -> Result<()> {
        self.i32(value.x)?; self.i32(value.y)?; self.i32(value.z)
    }
    fn area(&mut self, value: MapCuboid) -> Result<()> {
        self.coord(value.min)?; self.coord(value.max)
    }
    fn anchor(&mut self, value: StateAnchor) -> Result<()> {
        self.u64(value.fortress_id.get())?;
        self.u64(value.cursor.epoch)?; self.u64(value.cursor.sequence)?;
        self.u64(value.tick.0)?; self.raw(value.state_hash.as_bytes())
    }
    fn ids(&mut self, ids: &[EntityId]) -> Result<()> {
        self.count(ids.len(), MAX_COLLECTION)?;
        for id in ids { self.u64(id.get())?; }
        Ok(())
    }
    fn strings(&mut self, values: &BTreeSet<String>) -> Result<()> {
        self.count(values.len(), MAX_COLLECTION)?;
        for value in values { self.text(value, MAX_TEXT)?; }
        Ok(())
    }
    fn optional_u32(&mut self, value: Option<u32>) -> Result<()> {
        self.boolean(value.is_some())?;
        if let Some(value) = value { self.u32(value)?; }
        Ok(())
    }
    fn action(&mut self, action: &Action) -> Result<()> {
        self.node(0)?;
        match action {
            Action::Pause { paused } => { self.byte(0)?; self.boolean(*paused)?; }
            Action::DesignateDig { area, mode } => {
                self.byte(1)?; self.area(*area)?; self.dig_mode(*mode)?;
            }
            Action::Build { kind, location, footprint, material } => {
                self.byte(2)?; self.building_kind(kind)?; self.coord(*location)?;
                self.area(*footprint)?; self.strings(&material.required_tokens)?;
                self.strings(&material.forbidden_tokens)?;
                self.boolean(material.prefer_nearest)?; self.u32(material.reserve_count)?;
            }
            Action::SetLabor { units, labor, enabled } => {
                self.byte(3)?; self.ids(units)?; self.text(labor, MAX_TEXT)?; self.boolean(*enabled)?;
            }
            Action::CreateWorkOrder { name, job_token, amount, conditions } => {
                self.byte(4)?; self.text(name, MAX_TEXT)?; self.text(job_token, MAX_TEXT)?;
                self.u32(*amount)?; self.count(conditions.len(), MAX_COLLECTION)?;
                for condition in conditions {
                    match condition {
                        WorkOrderCondition::ItemCountBelow { item_token, threshold } => {
                            self.byte(0)?; self.text(item_token, MAX_TEXT)?; self.u32(*threshold)?;
                        }
                        WorkOrderCondition::MaterialAvailable { material_token, minimum } => {
                            self.byte(1)?; self.text(material_token, MAX_TEXT)?; self.u32(*minimum)?;
                        }
                        WorkOrderCondition::CompletedOrder { order_name } => {
                            self.byte(2)?; self.text(order_name, MAX_TEXT)?;
                        }
                    }
                }
            }
            Action::ConfigureStockpile { stockpile, accepts, max_bins, max_barrels, max_wheelbarrows } => {
                self.byte(5)?; self.u64(stockpile.get())?; self.strings(accepts)?;
                self.optional_u32(*max_bins)?; self.optional_u32(*max_barrels)?;
                self.optional_u32(*max_wheelbarrows)?;
            }
            Action::AssignSquad { units, squad } => {
                self.byte(6)?; self.ids(units)?; self.u64(squad.get())?;
            }
            Action::SetBurrowMembership { units, burrow, assigned } => {
                self.byte(7)?; self.ids(units)?; self.u64(burrow.get())?; self.boolean(*assigned)?;
            }
            Action::SetStandingOrder { key, value } => {
                self.byte(8)?; self.text(key, MAX_TEXT)?; self.text(value, MAX_TEXT)?;
            }
            Action::Extension { namespace, name, parameters } => {
                self.byte(9)?; self.text(namespace, MAX_TEXT)?; self.text(name, MAX_TEXT)?;
                self.count(parameters.len(), MAX_COLLECTION)?;
                for (key, value) in parameters { self.text(key, MAX_TEXT)?; self.text(value, MAX_TEXT)?; }
            }
        }
        Ok(())
    }
    fn value(&mut self, value: &Value, depth: usize) -> Result<()> {
        self.node(depth)?;
        match value {
            Value::Null => self.byte(0)?,
            Value::Bool(value) => { self.byte(1)?; self.boolean(*value)?; }
            Value::I64(value) => { self.byte(2)?; self.i64(*value)?; }
            Value::U64(value) => { self.byte(3)?; self.u64(*value)?; }
            Value::Fixed { units, scale } => { self.byte(4)?; self.i64(*units)?; self.u32(*scale)?; }
            Value::Text(value) => { self.byte(5)?; self.text(value, MAX_TEXT)?; }
            Value::Entity(value) => { self.byte(6)?; self.u64(value.get())?; }
            Value::Coord(value) => { self.byte(7)?; self.coord(*value)?; }
            Value::Bytes(value) => { self.byte(8)?; self.blob(value)?; }
            Value::List(values) => {
                self.byte(9)?; self.count(values.len(), MAX_COLLECTION)?;
                for value in values { self.value(value, depth + 1)?; }
            }
            Value::Object(values) => {
                self.byte(10)?; self.count(values.len(), MAX_COLLECTION)?;
                for (key, value) in values { self.text(key, MAX_TEXT)?; self.value(value, depth + 1)?; }
            }
        }
        Ok(())
    }
    fn predicate(&mut self, predicate: &Predicate, depth: usize) -> Result<()> {
        self.node(depth)?;
        match predicate {
            Predicate::True => self.byte(0)?,
            Predicate::False => self.byte(1)?,
            Predicate::EntityExists(id) => { self.byte(2)?; self.u64(id.get())?; }
            Predicate::EntityKind { entity_id, kind } => {
                self.byte(3)?; self.u64(entity_id.get())?; self.entity_kind(kind)?;
            }
            Predicate::FieldCompare { entity_id, field, op, value } => {
                self.byte(4)?; self.u64(entity_id.get())?; self.text(field, 256)?;
                self.compare_op(*op)?; self.value(value, depth + 1)?;
            }
            Predicate::EdgeExists { edge_id, kind } => {
                self.byte(5)?; self.u128(edge_id.get())?; self.boolean(kind.is_some())?;
                if let Some(kind) = kind { self.edge_kind(kind)?; }
            }
            Predicate::Paused(value) => { self.byte(6)?; self.boolean(*value)?; }
            Predicate::All(values) | Predicate::Any(values) => {
                self.byte(if matches!(predicate, Predicate::All(_)) { 7 } else { 8 })?;
                self.count(values.len(), MAX_COLLECTION)?;
                for value in values { self.predicate(value, depth + 1)?; }
            }
            Predicate::Not(value) => { self.byte(9)?; self.predicate(value, depth + 1)?; }
            Predicate::RegionTerrain { area, tile_code } => {
                self.byte(10)?; self.area(*area)?; self.u32(*tile_code)?;
            }
        }
        Ok(())
    }
    fn predicates(&mut self, values: &[Predicate]) -> Result<()> {
        self.count(values.len(), 64)?;
        for value in values { self.predicate(value, 1)?; }
        Ok(())
    }
    fn obligation(&mut self, value: &ObligationSpec) -> Result<()> {
        self.node(0)?; self.predicate(&value.terminal, 1)?;
        self.boolean(value.failure.is_some())?;
        if let Some(failure) = &value.failure { self.predicate(failure, 1)?; }
        self.u64(value.deadline_tick.0)?; self.u64(value.poll_interval_ticks)?;
        self.u32(value.stable_for_observations)
    }
    pub(super) fn plan(&mut self, value: &PreparedPlan) -> Result<()> {
        self.node(0)?; self.u128(value.id.get())?; self.u128(value.intent_id.get())?;
        self.anchor(value.anchor)?; self.text(&value.summary, 4096)?;
        self.predicate(&value.terminal_condition, 1)?;
        self.count(value.steps.len(), self.limits.max_steps)?;
        for step in &value.steps {
            self.u32(step.id.get())?; self.action(&step.action)?;
            self.predicates(&step.preconditions)?; self.predicates(&step.postconditions)?;
            self.boolean(step.compensation.is_some())?;
            if let Some(compensation) = &step.compensation { self.action(compensation)?; }
            self.boolean(step.obligation.is_some())?;
            if let Some(obligation) = &step.obligation { self.obligation(obligation)?; }
            self.count(step.depends_on.len(), self.limits.max_steps)?;
            for id in &step.depends_on { self.u32(id.get())?; }
            self.risk(step.risk)?; self.capability(step.required_capability)?;
            self.text(&step.idempotency_key, 64)?;
        }
        self.risk(value.max_risk)?;
        self.count(value.required_capabilities.len(), 18)?;
        for capability in &value.required_capabilities { self.capability(*capability)?; }
        self.boolean(value.requires_checkpoint)?; self.u64(value.expires_at_tick.0)?;
        self.raw(value.digest.as_bytes())
    }
}

fn insert<K: Ord, V>(map: &mut BTreeMap<K, V>, key: K, value: V) -> Result<()> {
    if map.last_key_value().is_some_and(|(last, _)| last >= &key) {
        return Err(invalid("archive map keys are not strictly ordered"));
    }
    map.insert(key, value);
    Ok(())
}
fn insert_set<T: Ord>(set: &mut BTreeSet<T>, value: T) -> Result<()> {
    if set.last().is_some_and(|last| last >= &value) {
        return Err(invalid("archive set members are not strictly ordered"));
    }
    set.insert(value);
    Ok(())
}

impl<F: FnMut() -> Result<()>> Reader<'_, '_, F> {
    fn coord(&mut self) -> Result<MapCoord> { Ok(MapCoord::new(self.i32()?, self.i32()?, self.i32()?)) }
    fn area(&mut self) -> Result<MapCuboid> { Ok(MapCuboid { min: self.coord()?, max: self.coord()? }) }
    fn anchor(&mut self) -> Result<StateAnchor> {
        Ok(StateAnchor { fortress_id: FortressId::new(self.u64()?),
            cursor: ObservationCursor { epoch: self.u64()?, sequence: self.u64()? },
            tick: GameTick(self.u64()?), state_hash: Digest32::from_bytes(self.array()?) })
    }
    fn ids(&mut self) -> Result<Vec<EntityId>> {
        let count = self.count(MAX_COLLECTION, 8)?;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count { values.push(EntityId::new(self.u64()?)); }
        Ok(values)
    }
    fn strings(&mut self) -> Result<BTreeSet<String>> {
        let count = self.count(MAX_COLLECTION, 4)?;
        let mut values = BTreeSet::new();
        for _ in 0..count { insert_set(&mut values, self.text(MAX_TEXT)?)?; }
        Ok(values)
    }
    fn optional_u32(&mut self) -> Result<Option<u32>> {
        if self.boolean()? { Ok(Some(self.u32()?)) } else { Ok(None) }
    }
    fn action(&mut self) -> Result<Action> {
        self.node(0)?;
        Ok(match self.byte()? {
            0 => Action::Pause { paused: self.boolean()? },
            1 => Action::DesignateDig { area: self.area()?, mode: self.dig_mode()? },
            2 => Action::Build { kind: self.building_kind()?, location: self.coord()?, footprint: self.area()?,
                material: MaterialSelector { required_tokens: self.strings()?, forbidden_tokens: self.strings()?,
                    prefer_nearest: self.boolean()?, reserve_count: self.u32()? } },
            3 => Action::SetLabor { units: self.ids()?, labor: self.text(MAX_TEXT)?, enabled: self.boolean()? },
            4 => {
                let name = self.text(MAX_TEXT)?; let job_token = self.text(MAX_TEXT)?;
                let amount = self.u32()?; let count = self.count(MAX_COLLECTION, 5)?;
                let mut conditions = Vec::with_capacity(count);
                for _ in 0..count {
                    conditions.push(match self.byte()? {
                        0 => WorkOrderCondition::ItemCountBelow { item_token: self.text(MAX_TEXT)?, threshold: self.u32()? },
                        1 => WorkOrderCondition::MaterialAvailable { material_token: self.text(MAX_TEXT)?, minimum: self.u32()? },
                        2 => WorkOrderCondition::CompletedOrder { order_name: self.text(MAX_TEXT)? },
                        _ => return Err(invalid("unknown archived work-order condition")),
                    });
                }
                Action::CreateWorkOrder { name, job_token, amount, conditions }
            }
            5 => Action::ConfigureStockpile { stockpile: EntityId::new(self.u64()?), accepts: self.strings()?,
                max_bins: self.optional_u32()?, max_barrels: self.optional_u32()?, max_wheelbarrows: self.optional_u32()? },
            6 => Action::AssignSquad { units: self.ids()?, squad: EntityId::new(self.u64()?) },
            7 => Action::SetBurrowMembership { units: self.ids()?, burrow: EntityId::new(self.u64()?), assigned: self.boolean()? },
            8 => Action::SetStandingOrder { key: self.text(MAX_TEXT)?, value: self.text(MAX_TEXT)? },
            9 => {
                let namespace = self.text(MAX_TEXT)?; let name = self.text(MAX_TEXT)?;
                let count = self.count(MAX_COLLECTION, 8)?;
                let mut parameters = BTreeMap::new();
                for _ in 0..count { let key = self.text(MAX_TEXT)?; let value = self.text(MAX_TEXT)?; insert(&mut parameters, key, value)?; }
                Action::Extension { namespace, name, parameters }
            }
            _ => return Err(invalid("unknown archived action")),
        })
    }
    fn value(&mut self, depth: usize) -> Result<Value> {
        self.node(depth)?;
        Ok(match self.byte()? {
            0 => Value::Null,
            1 => Value::Bool(self.boolean()?),
            2 => Value::I64(self.i64()?),
            3 => Value::U64(self.u64()?),
            4 => Value::Fixed { units: self.i64()?, scale: self.u32()? },
            5 => Value::Text(self.text(MAX_TEXT)?),
            6 => Value::Entity(EntityId::new(self.u64()?)),
            7 => Value::Coord(self.coord()?),
            8 => Value::Bytes(self.blob()?),
            9 => {
                let count = self.count(MAX_COLLECTION, 1)?;
                let mut values = Vec::with_capacity(count);
                for _ in 0..count { values.push(self.value(depth + 1)?); }
                Value::List(values)
            }
            10 => {
                let count = self.count(MAX_COLLECTION, 5)?;
                let mut values = BTreeMap::new();
                for _ in 0..count { let key = self.text(MAX_TEXT)?; let value = self.value(depth + 1)?; insert(&mut values, key, value)?; }
                Value::Object(values)
            }
            _ => return Err(invalid("unknown archived value")),
        })
    }
    fn predicate(&mut self, depth: usize) -> Result<Predicate> {
        self.node(depth)?;
        Ok(match self.byte()? {
            0 => Predicate::True,
            1 => Predicate::False,
            2 => Predicate::EntityExists(EntityId::new(self.u64()?)),
            3 => Predicate::EntityKind { entity_id: EntityId::new(self.u64()?), kind: self.entity_kind()? },
            4 => Predicate::FieldCompare { entity_id: EntityId::new(self.u64()?), field: self.text(256)?,
                op: self.compare_op()?, value: self.value(depth + 1)? },
            5 => {
                let edge_id = EdgeId::new(self.u128()?);
                let kind = if self.boolean()? { Some(self.edge_kind()?) } else { None };
                Predicate::EdgeExists { edge_id, kind }
            }
            6 => Predicate::Paused(self.boolean()?),
            tag @ (7 | 8) => {
                let count = self.count(MAX_COLLECTION, 1)?;
                let mut values = Vec::with_capacity(count);
                for _ in 0..count { values.push(self.predicate(depth + 1)?); }
                if tag == 7 { Predicate::All(values) } else { Predicate::Any(values) }
            }
            9 => Predicate::Not(Box::new(self.predicate(depth + 1)?)),
            10 => Predicate::RegionTerrain { area: self.area()?, tile_code: self.u32()? },
            _ => return Err(invalid("unknown archived predicate")),
        })
    }
    fn predicates(&mut self) -> Result<Vec<Predicate>> {
        let count = self.count(64, 1)?;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count { values.push(self.predicate(1)?); }
        Ok(values)
    }
    fn obligation(&mut self) -> Result<ObligationSpec> {
        self.node(0)?;
        Ok(ObligationSpec { terminal: self.predicate(1)?,
            failure: if self.boolean()? { Some(self.predicate(1)?) } else { None },
            deadline_tick: GameTick(self.u64()?), poll_interval_ticks: self.u64()?, stable_for_observations: self.u32()? })
    }
    pub(super) fn plan(&mut self) -> Result<PreparedPlan> {
        self.node(0)?;
        let id = PlanId::new(self.u128()?); let intent_id = IntentId::new(self.u128()?);
        let anchor = self.anchor()?; let summary = self.text(4096)?;
        let terminal_condition = self.predicate(1)?;
        let count = self.count(self.limits.max_steps, 1)?;
        let mut steps = Vec::with_capacity(count);
        for _ in 0..count {
            let id = StepId::new(self.u32()?); let action = self.action()?;
            let preconditions = self.predicates()?; let postconditions = self.predicates()?;
            let compensation = if self.boolean()? { Some(self.action()?) } else { None };
            let obligation = if self.boolean()? { Some(self.obligation()?) } else { None };
            let count = self.count(self.limits.max_steps, 4)?;
            let mut depends_on = Vec::with_capacity(count);
            for _ in 0..count { depends_on.push(StepId::new(self.u32()?)); }
            steps.push(PlanStep { id, action, preconditions, postconditions, compensation, obligation, depends_on,
                risk: self.risk()?, required_capability: self.capability()?, idempotency_key: self.text(64)? });
        }
        let max_risk = self.risk()?; let count = self.count(18, 1)?;
        let mut required_capabilities = BTreeSet::new();
        for _ in 0..count { insert_set(&mut required_capabilities, self.capability()?)?; }
        Ok(PreparedPlan { id, intent_id, anchor, summary, terminal_condition, steps, max_risk, required_capabilities,
            requires_checkpoint: self.boolean()?, expires_at_tick: GameTick(self.u64()?),
            digest: Digest32::from_bytes(self.array()?) })
    }
}

impl<F: FnMut() -> Result<()>> Writer<'_, F> {
    fn risk(&mut self, value: RiskTier) -> Result<()> {
        self.byte(match value {
            RiskTier::ReadOnly => 0,
            RiskTier::Reversible => 1,
            RiskTier::Guarded => 2,
            RiskTier::Irreversible => 3,
        })
    }
}
impl<F: FnMut() -> Result<()>> Reader<'_, '_, F> {
    fn risk(&mut self) -> Result<RiskTier> {
        match self.byte()? {
            0 => Ok(RiskTier::ReadOnly),
            1 => Ok(RiskTier::Reversible),
            2 => Ok(RiskTier::Guarded),
            3 => Ok(RiskTier::Irreversible),
            _ => Err(invalid("unknown archived risk")),
        }
    }
}
impl<F: FnMut() -> Result<()>> Writer<'_, F> {
    fn capability(&mut self, value: Capability) -> Result<()> {
        self.byte(match value {
            Capability::Observe => 0,
            Capability::Query => 1,
            Capability::Plan => 2,
            Capability::Designate => 3,
            Capability::Construct => 4,
            Capability::ConfigureLabor => 5,
            Capability::ConfigureProduction => 6,
            Capability::ConfigureLogistics => 7,
            Capability::ConfigureMilitary => 8,
            Capability::ControlClock => 9,
            Capability::Checkpoint => 10,
            Capability::Restore => 11,
            Capability::Extension => 12,
            Capability::DiagnosticRaw => 13,
            Capability::Doctor => 14,
            Capability::RepairPlan => 15,
            Capability::RepairApply => 16,
            Capability::Admin => 17,
        })
    }
}
impl<F: FnMut() -> Result<()>> Reader<'_, '_, F> {
    fn capability(&mut self) -> Result<Capability> {
        match self.byte()? {
            0 => Ok(Capability::Observe),
            1 => Ok(Capability::Query),
            2 => Ok(Capability::Plan),
            3 => Ok(Capability::Designate),
            4 => Ok(Capability::Construct),
            5 => Ok(Capability::ConfigureLabor),
            6 => Ok(Capability::ConfigureProduction),
            7 => Ok(Capability::ConfigureLogistics),
            8 => Ok(Capability::ConfigureMilitary),
            9 => Ok(Capability::ControlClock),
            10 => Ok(Capability::Checkpoint),
            11 => Ok(Capability::Restore),
            12 => Ok(Capability::Extension),
            13 => Ok(Capability::DiagnosticRaw),
            14 => Ok(Capability::Doctor),
            15 => Ok(Capability::RepairPlan),
            16 => Ok(Capability::RepairApply),
            17 => Ok(Capability::Admin),
            _ => Err(invalid("unknown archived capability")),
        }
    }
}
impl<F: FnMut() -> Result<()>> Writer<'_, F> {
    fn compare_op(&mut self, value: CompareOp) -> Result<()> {
        self.byte(match value {
            CompareOp::Eq => 0,
            CompareOp::Ne => 1,
            CompareOp::Lt => 2,
            CompareOp::Le => 3,
            CompareOp::Gt => 4,
            CompareOp::Ge => 5,
        })
    }
}
impl<F: FnMut() -> Result<()>> Reader<'_, '_, F> {
    fn compare_op(&mut self) -> Result<CompareOp> {
        match self.byte()? {
            0 => Ok(CompareOp::Eq),
            1 => Ok(CompareOp::Ne),
            2 => Ok(CompareOp::Lt),
            3 => Ok(CompareOp::Le),
            4 => Ok(CompareOp::Gt),
            5 => Ok(CompareOp::Ge),
            _ => Err(invalid("unknown archived compare_op")),
        }
    }
}
impl<F: FnMut() -> Result<()>> Writer<'_, F> {
    fn dig_mode(&mut self, value: DigMode) -> Result<()> {
        self.byte(match value {
            DigMode::Mine => 0,
            DigMode::Channel => 1,
            DigMode::UpStair => 2,
            DigMode::DownStair => 3,
            DigMode::UpDownStair => 4,
            DigMode::Ramp => 5,
            DigMode::RemoveConstruction => 6,
        })
    }
}
impl<F: FnMut() -> Result<()>> Reader<'_, '_, F> {
    fn dig_mode(&mut self) -> Result<DigMode> {
        match self.byte()? {
            0 => Ok(DigMode::Mine),
            1 => Ok(DigMode::Channel),
            2 => Ok(DigMode::UpStair),
            3 => Ok(DigMode::DownStair),
            4 => Ok(DigMode::UpDownStair),
            5 => Ok(DigMode::Ramp),
            6 => Ok(DigMode::RemoveConstruction),
            _ => Err(invalid("unknown archived dig_mode")),
        }
    }
}
impl<F: FnMut() -> Result<()>> Writer<'_, F> {
    fn entity_kind(&mut self, value: &EntityKind) -> Result<()> {
        match value {
            EntityKind::Fortress => self.byte(0),
            EntityKind::Unit => self.byte(1),
            EntityKind::Item => self.byte(2),
            EntityKind::Building => self.byte(3),
            EntityKind::Job => self.byte(4),
            EntityKind::WorkOrder => self.byte(5),
            EntityKind::Stockpile => self.byte(6),
            EntityKind::Zone => self.byte(7),
            EntityKind::Burrow => self.byte(8),
            EntityKind::Squad => self.byte(9),
            EntityKind::MilitaryOrder => self.byte(10),
            EntityKind::TileFeature => self.byte(11),
            EntityKind::Plant => self.byte(12),
            EntityKind::Creature => self.byte(13),
            EntityKind::HistoricalFigure => self.byte(14),
            EntityKind::Civilization => self.byte(15),
            EntityKind::Announcement => self.byte(16),
            EntityKind::Syndrome => self.byte(17),
            EntityKind::Other(text) => { self.byte(18)?; self.text(text, 128) } ,
        }
    }
}
impl<F: FnMut() -> Result<()>> Reader<'_, '_, F> {
    fn entity_kind(&mut self) -> Result<EntityKind> {
        match self.byte()? {
            0 => Ok(EntityKind::Fortress),
            1 => Ok(EntityKind::Unit),
            2 => Ok(EntityKind::Item),
            3 => Ok(EntityKind::Building),
            4 => Ok(EntityKind::Job),
            5 => Ok(EntityKind::WorkOrder),
            6 => Ok(EntityKind::Stockpile),
            7 => Ok(EntityKind::Zone),
            8 => Ok(EntityKind::Burrow),
            9 => Ok(EntityKind::Squad),
            10 => Ok(EntityKind::MilitaryOrder),
            11 => Ok(EntityKind::TileFeature),
            12 => Ok(EntityKind::Plant),
            13 => Ok(EntityKind::Creature),
            14 => Ok(EntityKind::HistoricalFigure),
            15 => Ok(EntityKind::Civilization),
            16 => Ok(EntityKind::Announcement),
            17 => Ok(EntityKind::Syndrome),
            18 => Ok(EntityKind::Other(self.text(128)?)),
            _ => Err(invalid("unknown archived entity_kind")),
        }
    }
}
impl<F: FnMut() -> Result<()>> Writer<'_, F> {
    fn edge_kind(&mut self, value: &EdgeKind) -> Result<()> {
        match value {
            EdgeKind::LocatedAt => self.byte(0),
            EdgeKind::ContainedIn => self.byte(1),
            EdgeKind::AssignedTo => self.byte(2),
            EdgeKind::MemberOf => self.byte(3),
            EdgeKind::Performs => self.byte(4),
            EdgeKind::Requires => self.byte(5),
            EdgeKind::Produces => self.byte(6),
            EdgeKind::Uses => self.byte(7),
            EdgeKind::Supports => self.byte(8),
            EdgeKind::Threatens => self.byte(9),
            EdgeKind::OrderedBy => self.byte(10),
            EdgeKind::ParentOf => self.byte(11),
            EdgeKind::Custom(text) => { self.byte(12)?; self.text(text, 128) } ,
        }
    }
}
impl<F: FnMut() -> Result<()>> Reader<'_, '_, F> {
    fn edge_kind(&mut self) -> Result<EdgeKind> {
        match self.byte()? {
            0 => Ok(EdgeKind::LocatedAt),
            1 => Ok(EdgeKind::ContainedIn),
            2 => Ok(EdgeKind::AssignedTo),
            3 => Ok(EdgeKind::MemberOf),
            4 => Ok(EdgeKind::Performs),
            5 => Ok(EdgeKind::Requires),
            6 => Ok(EdgeKind::Produces),
            7 => Ok(EdgeKind::Uses),
            8 => Ok(EdgeKind::Supports),
            9 => Ok(EdgeKind::Threatens),
            10 => Ok(EdgeKind::OrderedBy),
            11 => Ok(EdgeKind::ParentOf),
            12 => Ok(EdgeKind::Custom(self.text(128)?)),
            _ => Err(invalid("unknown archived edge_kind")),
        }
    }
}
impl<F: FnMut() -> Result<()>> Writer<'_, F> {
    fn building_kind(&mut self, value: &BuildingKind) -> Result<()> {
        match value {
            BuildingKind::Workshop(text) => { self.byte(0)?; self.text(text, 65536) } ,
            BuildingKind::Furnace(text) => { self.byte(1)?; self.text(text, 65536) } ,
            BuildingKind::Furniture(text) => { self.byte(2)?; self.text(text, 65536) } ,
            BuildingKind::Construction(text) => { self.byte(3)?; self.text(text, 65536) } ,
            BuildingKind::Trap(text) => { self.byte(4)?; self.text(text, 65536) } ,
            BuildingKind::FarmPlot => self.byte(5),
            BuildingKind::Bridge => self.byte(6),
            BuildingKind::Well => self.byte(7),
            BuildingKind::Custom(text) => { self.byte(8)?; self.text(text, 65536) } ,
        }
    }
}
impl<F: FnMut() -> Result<()>> Reader<'_, '_, F> {
    fn building_kind(&mut self) -> Result<BuildingKind> {
        match self.byte()? {
            0 => Ok(BuildingKind::Workshop(self.text(65536)?)),
            1 => Ok(BuildingKind::Furnace(self.text(65536)?)),
            2 => Ok(BuildingKind::Furniture(self.text(65536)?)),
            3 => Ok(BuildingKind::Construction(self.text(65536)?)),
            4 => Ok(BuildingKind::Trap(self.text(65536)?)),
            5 => Ok(BuildingKind::FarmPlot),
            6 => Ok(BuildingKind::Bridge),
            7 => Ok(BuildingKind::Well),
            8 => Ok(BuildingKind::Custom(self.text(65536)?)),
            _ => Err(invalid("unknown archived building_kind")),
        }
    }
}
