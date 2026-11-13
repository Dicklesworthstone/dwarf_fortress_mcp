//! Immutable withdrawal of a physically quiet original, with retained audit roots.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableObjectiveWithdrawal {
    pub fortress_id: FortressId,
    pub plan_digest: Digest32,
    pub anchor: StateAnchor,
    pub requester_session_id: SessionId,
    pub proof_digest: Digest32,
    /// Positive dispatch custody supplied by the current semantic owner. A
    /// cancelled deferred step can be never dispatched without changing its
    /// immutable Cancelled receipt or the historical T/N record semantics.
    pub never_dispatched_steps: BTreeSet<u32>,
}

impl DurableObjectiveWithdrawal {
    /// Seal the original source, complete archived terminal census, exact
    /// withdrawal frontier and positive nondispatch custody. The semantic owner
    /// independently verifies current authority, lineage and physical quiet.
    pub fn new(
        objective: &DurableObjective,
        terminal: &DurableCommit,
        anchor: StateAnchor,
        requester_session_id: SessionId,
        never_dispatched_steps: BTreeSet<u32>,
    ) -> Result<Self> {
        let mut withdrawal = Self {
            fortress_id: objective.fortress_id,
            plan_digest: objective.plan_digest,
            anchor,
            requester_session_id,
            proof_digest: Digest32::ZERO,
            never_dispatched_steps,
        };
        withdrawal.validate_source(objective, terminal)?;
        withdrawal.proof_digest = withdrawal.computed_proof(objective, terminal);
        Ok(withdrawal)
    }

    pub fn validate_source(
        &self,
        objective: &DurableObjective,
        terminal: &DurableCommit,
    ) -> Result<()> {
        if self.fortress_id != objective.fortress_id
            || self.anchor.fortress_id != self.fortress_id
            || self.plan_digest != objective.plan_digest
            || self.requester_session_id == SessionId::NIL
            || objective.restore_abandoned_anchor.is_some()
            || terminal.fortress_id != objective.fortress_id
            || terminal.plan_digest != objective.plan_digest
            || terminal.intent_id != objective.intent_id
            || terminal.sealed_state_hash != objective.sealed_state_hash
            || terminal.source != objective.source
            || terminal.steps.is_empty()
            || terminal.steps.len() > MAX_STEPS_PER_COMMIT
            || terminal.steps.len() != terminal.step_anchors.len()
            || self.never_dispatched_steps.len() > terminal.steps.len()
        {
            return Err(corrupt(
                "withdrawal lost its exact original terminal custody",
            ));
        }
        for (step, state) in &terminal.steps {
            let anchor = terminal.step_anchors.get(step).ok_or_else(|| {
                corrupt("withdrawal requires every terminal step's atomic anchor")
            })?;
            if !matches!(
                state.as_str(),
                "verified" | "failed" | "cancelled" | "compensated" | "not_dispatched"
            ) || !anchor_precedes(*anchor, self.anchor)
                || (state == "not_dispatched" && !self.never_dispatched_steps.contains(step))
            {
                return Err(corrupt(
                    "withdrawal has unfinished or out-of-lineage action evidence",
                ));
            }
        }
        if self.never_dispatched_steps.iter().any(|step| {
            !terminal.steps.get(step).is_some_and(|state| {
                matches!(state.as_str(), "not_dispatched" | "cancelled" | "failed")
            })
        }) {
            return Err(corrupt(
                "withdrawal nondispatch contradicts its terminal receipt",
            ));
        }
        Ok(())
    }

    pub fn proof_matches(&self, objective: &DurableObjective, terminal: &DurableCommit) -> bool {
        self.proof_digest == self.computed_proof(objective, terminal)
    }

    fn computed_proof(&self, objective: &DurableObjective, terminal: &DurableCommit) -> Digest32 {
        let mut material = b"dfmcp-goal-withdrawal/1\0".to_vec();
        material.extend_from_slice(self.plan_digest.as_bytes());
        material.extend_from_slice(objective.sealed_state_hash.as_bytes());
        material.extend_from_slice(&objective.intent_id.to_be_bytes());
        material.extend_from_slice(&objective.owner_session_id.get().to_be_bytes());
        material.extend_from_slice(&self.requester_session_id.get().to_be_bytes());
        append_anchor(&mut material, self.anchor);
        for value in [
            objective.source.fields().0,
            objective.source.fields().1,
            objective.source.fields().2,
        ] {
            material.extend_from_slice(&(value.len() as u64).to_be_bytes());
            material.extend_from_slice(value.as_bytes());
        }
        material.extend_from_slice(&(terminal.steps.len() as u32).to_be_bytes());
        for (step, state) in &terminal.steps {
            material.extend_from_slice(&step.to_be_bytes());
            material.extend_from_slice(&(state.len() as u32).to_be_bytes());
            material.extend_from_slice(state.as_bytes());
            if let Some(anchor) = terminal.step_anchors.get(step) {
                append_anchor(&mut material, *anchor);
            }
            material.push(u8::from(self.never_dispatched_steps.contains(step)));
        }
        Digest32::of_bytes(&material)
    }

    pub(super) fn payload(&self) -> String {
        let mut payload = format!(
            "U {} {} {} {} {} {} {:032x} {} {}",
            self.fortress_id.get(),
            self.plan_digest.to_hex(),
            self.anchor.state_hash.to_hex(),
            self.anchor.tick.0,
            self.anchor.cursor.epoch,
            self.anchor.cursor.sequence,
            self.requester_session_id.get(),
            self.proof_digest.to_hex(),
            self.never_dispatched_steps.len()
        );
        for step in &self.never_dispatched_steps {
            payload.push_str(&format!(" {step}"));
        }
        payload
    }

    pub(super) fn parse(payload: &str) -> Result<Self> {
        let fields: Vec<_> = payload.split(' ').collect();
        if fields.len() < 10 || fields[0] != "U" {
            return Err(corrupt("invalid original withdrawal record"));
        }
        let count = usize::try_from(parse_u64(fields[9])?)
            .map_err(|_| corrupt("withdrawal step count exceeds its bound"))?;
        if count > MAX_STEPS_PER_COMMIT || fields.len() != count + 10 {
            return Err(corrupt("withdrawal step count or arity is invalid"));
        }
        let mut never_dispatched_steps = BTreeSet::new();
        let mut previous = None;
        for field in &fields[10..] {
            let step = u32::try_from(parse_u64(field)?)
                .map_err(|_| corrupt("withdrawal step identity is out of range"))?;
            if previous.is_some_and(|previous| previous >= step) {
                return Err(corrupt(
                    "withdrawal nondispatch steps are not strictly ordered",
                ));
            }
            previous = Some(step);
            never_dispatched_steps.insert(step);
        }
        let fortress_id = FortressId::new(parse_u64(fields[1])?);
        Ok(Self {
            fortress_id,
            plan_digest: parse_digest(fields[2])?,
            anchor: StateAnchor {
                fortress_id,
                state_hash: parse_digest(fields[3])?,
                tick: GameTick(parse_u64(fields[4])?),
                cursor: ObservationCursor {
                    epoch: parse_u64(fields[5])?,
                    sequence: parse_u64(fields[6])?,
                },
            },
            requester_session_id: SessionId::new(parse_identifier(fields[7])?),
            proof_digest: parse_digest(fields[8])?,
            never_dispatched_steps,
        })
    }
}

fn append_anchor(material: &mut Vec<u8>, anchor: StateAnchor) {
    material.extend_from_slice(&anchor.fortress_id.get().to_be_bytes());
    material.extend_from_slice(&anchor.tick.0.to_be_bytes());
    material.extend_from_slice(&anchor.cursor.epoch.to_be_bytes());
    material.extend_from_slice(&anchor.cursor.sequence.to_be_bytes());
    material.extend_from_slice(anchor.state_hash.as_bytes());
}

impl Index {
    pub(super) fn active_objective_count(&self, fortress: FortressId) -> usize {
        self.objectives
            .get(&fortress)
            .map_or(0, BTreeMap::len)
            .saturating_sub(self.withdrawals.get(&fortress).map_or(0, BTreeMap::len))
    }

    pub(super) fn validate_withdrawal(
        &self,
        withdrawal: &DurableObjectiveWithdrawal,
    ) -> Result<()> {
        let fortress = withdrawal.fortress_id;
        let digest = withdrawal.plan_digest;
        let objective = self
            .objectives
            .get(&fortress)
            .and_then(|book| book.get(&digest))
            .ok_or_else(|| corrupt("withdrawal names an unknown original objective"))?;
        let terminal = self
            .retired_commits
            .get(&fortress)
            .and_then(|book| book.get(&digest))
            .ok_or_else(|| {
                corrupt("withdrawal requires retained complete terminal action history")
            })?;
        if self
            .commits
            .get(&fortress)
            .is_some_and(|book| book.contains_key(&digest))
        {
            return Err(corrupt(
                "withdrawal cannot discard an unfinished action commit",
            ));
        }
        if self
            .withdrawals
            .get(&fortress)
            .is_some_and(|book| book.contains_key(&digest))
        {
            return Err(corrupt("original goal withdrawal was recorded twice"));
        }
        if self.withdrawals.get(&fortress).map_or(0, BTreeMap::len)
            >= MAX_WITHDRAWN_OBJECTIVES_PER_FORTRESS
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "withdrawn original archive is full; retained evidence cannot be discarded",
            ));
        }
        withdrawal.validate_source(objective, terminal)?;
        if !withdrawal.proof_matches(objective, terminal) {
            return Err(corrupt(
                "withdrawal proof digest does not bind its original evidence",
            ));
        }
        Ok(())
    }
}

impl DurableLabStore {
    pub fn objective_withdrawal(
        &self,
        fortress: FortressId,
        digest: Digest32,
    ) -> Option<&DurableObjectiveWithdrawal> {
        self.index
            .withdrawals
            .get(&fortress)
            .and_then(|book| book.get(&digest))
    }

    pub fn persist_objective_withdrawal(
        &mut self,
        withdrawal: &DurableObjectiveWithdrawal,
        snapshot: &WorldSnapshot,
    ) -> Result<()> {
        self.ensure_writable()?;
        if let Some(previous) =
            self.objective_withdrawal(withdrawal.fortress_id, withdrawal.plan_digest)
        {
            return if previous == withdrawal {
                Ok(())
            } else {
                Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    "original withdrawal is immutable",
                ))
            };
        }
        if !snapshot.hash_is_valid()
            || snapshot.anchor() != withdrawal.anchor
            || self
                .head(withdrawal.fortress_id)
                .is_none_or(|head| head.anchor != withdrawal.anchor)
        {
            return Err(invalid(
                "withdrawal requires its exact already-published world frontier",
            ));
        }
        self.index.validate_withdrawal(withdrawal)?;
        self.append(
            Record::ObjectiveWithdrawal(withdrawal.clone()),
            Some(snapshot),
        )
    }

    pub(super) fn validate_withdrawal_history(
        &self,
        verified: &BTreeMap<Digest32, StateAnchor>,
    ) -> Result<()> {
        for fortress in self.index.objectives.keys() {
            if self.index.active_objective_count(*fortress) > MAX_OBJECTIVES_PER_FORTRESS {
                return Err(corrupt(
                    "retained originals exceed the active pursuit bound",
                ));
            }
        }
        for withdrawal in self.index.withdrawals.values().flat_map(BTreeMap::values) {
            if verified.get(&withdrawal.anchor.state_hash) != Some(&withdrawal.anchor) {
                return Err(corrupt("withdrawal lost its exact archived world frontier"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "durable_withdrawal_tests.rs"]
mod tests;
