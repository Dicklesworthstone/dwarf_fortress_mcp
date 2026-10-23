#![forbid(unsafe_code)]

//! WP-PLN-03: bounded civilian protection and observation-driven recovery.
//!
//! Every output is an action proposal. Caller-declared threat assessments and
//! source-scope identities are not native evidence or mutation authority.

use std::collections::BTreeSet;

use dfmcp_core::{
    DfmcpError, Digest32, EntityId, ErrorCode, FortressId, GameTick, Result, StateAnchor,
};

use crate::action::Action;

const MAX_ALERT_UNITS: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ThreatLevel {
    Peace,
    Cautious,
    EmergencyLockdown,
    Recovering,
}

/// A shell-supplied assessment, not something inferred from missing entities.
/// `Clear` requires independently established complete threat-domain coverage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreatAssessment {
    Present,
    Clear,
    Unknown,
}

/// Bind every assessment and the supplied civilian roster to one exact anchor.
/// The source scope must change with source generation or coverage/policy scope.
/// Its digest labels a scope; possession of it proves nothing and grants nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlertObservation {
    pub anchor: StateAnchor,
    pub source_scope: Digest32,
    pub threat: ThreatAssessment,
    pub complete_civilian_roster: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlertRecoveryPolicy {
    pub minimum_clear_ticks: u64,
    pub minimum_clear_samples: u32,
    pub maximum_sample_gap_ticks: u64,
}

impl Default for AlertRecoveryPolicy {
    fn default() -> Self {
        Self {
            minimum_clear_ticks: 100,
            minimum_clear_samples: 2,
            maximum_sample_gap_ticks: 100,
        }
    }
}

/// A decision at one anchor, not proof that any proposed action happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertDecision {
    pub anchor: StateAnchor,
    pub source_scope: Digest32,
    pub target_level: ThreatLevel,
    pub actions: Vec<Action>,
    pub clear_since: Option<GameTick>,
    pub clear_samples: u32,
    pub release_ready: bool,
}

#[derive(Clone, Debug)]
pub struct CivilianAlertFsm {
    current_level: ThreatLevel,
    safe_burrow_id: EntityId,
    military_squad_ids: Vec<EntityId>,
    recovery_policy: AlertRecoveryPolicy,
    last_observation: Option<(AlertObservation, Vec<EntityId>)>,
    clear_since: Option<GameTick>,
    clear_samples: u32,
    // Threat recognition must survive until an observed release, including when
    // the original lockdown proposal has not yet been confirmed by the shell.
    emergency_seen: bool,
}

impl CivilianAlertFsm {
    pub fn new(safe_burrow_id: EntityId, military_squad_ids: Vec<EntityId>) -> Result<Self> {
        Self::with_recovery_policy(
            safe_burrow_id,
            military_squad_ids,
            AlertRecoveryPolicy::default(),
        )
    }

    pub fn with_recovery_policy(
        safe_burrow_id: EntityId,
        military_squad_ids: Vec<EntityId>,
        recovery_policy: AlertRecoveryPolicy,
    ) -> Result<Self> {
        if safe_burrow_id == EntityId::NIL {
            return Err(invalid("alert FSM requires a nonzero safe burrow"));
        }
        let military_squad_ids = canonical_units(&military_squad_ids)?;
        if !(2..=1_024).contains(&recovery_policy.minimum_clear_samples)
            || !(1..=1_000_000).contains(&recovery_policy.minimum_clear_ticks)
            || !(1..=1_000_000).contains(&recovery_policy.maximum_sample_gap_ticks)
        {
            return Err(invalid("alert recovery policy exceeds fixed positive bounds"));
        }
        Ok(Self {
            current_level: ThreatLevel::Peace,
            safe_burrow_id,
            military_squad_ids,
            recovery_policy,
            last_observation: None,
            clear_since: None,
            clear_samples: 0,
            emergency_seen: false,
        })
    }

    #[must_use]
    pub const fn current_level(&self) -> ThreatLevel {
        self.current_level
    }

    #[must_use]
    pub const fn safe_burrow_id(&self) -> EntityId {
        self.safe_burrow_id
    }

    #[must_use]
    pub fn military_squad_ids(&self) -> &[EntityId] {
        &self.military_squad_ids
    }

    /// Explicit caller-requested transition, not an automatic threat decision.
    /// Direct Peace remains an explicit override proposal; autonomous callers
    /// should use `plan_observation`, which guards release with recovery history.
    /// Recovering keeps protection active instead of prematurely releasing it.
    pub fn transition_to(
        &self,
        new_level: ThreatLevel,
        civilian_ids: &[EntityId],
    ) -> Result<Vec<Action>> {
        let civilians = canonical_units(civilian_ids)?;
        if self.current_level == new_level {
            return Ok(Vec::new());
        }
        if civilians.is_empty() {
            return Err(invalid("explicit alert transitions require civilians"));
        }
        match (self.current_level, new_level) {
            (_, ThreatLevel::EmergencyLockdown)
            | (ThreatLevel::EmergencyLockdown, ThreatLevel::Recovering) => {
                Ok(self.protection_actions(civilians, true))
            }
            (ThreatLevel::EmergencyLockdown | ThreatLevel::Recovering, ThreatLevel::Peace) => {
                Ok(self.protection_actions(civilians, false))
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Consume one caller-qualified observation and propose a bounded response.
    /// This updates monitoring history, never the observed defensive level.
    ///
    /// Only consecutive, strictly advancing ticks/cursors in the same source,
    /// epoch and complete civilian roster contribute to clearance. Replays do
    /// not count. Unknown/partial observations, source changes and gaps reset
    /// stability. Invalid, stale or conflicting input also clears stability but
    /// retains the last accepted source floor. No I/O, time advance or dispatch.
    pub fn plan_observation(
        &mut self,
        observation: AlertObservation,
        civilian_ids: &[EntityId],
    ) -> Result<AlertDecision> {
        let result = self.accept_observation(observation, civilian_ids);
        if result.is_err() {
            self.reset_clearance();
        }
        result
    }

    fn accept_observation(
        &mut self,
        observation: AlertObservation,
        civilian_ids: &[EntityId],
    ) -> Result<AlertDecision> {
        let civilians = canonical_units(civilian_ids)?;
        if observation.anchor.fortress_id == FortressId::NIL
            || observation.anchor.state_hash == Digest32::ZERO
            || observation.source_scope == Digest32::ZERO
        {
            return Err(invalid("alert observation requires nonzero source and anchor identity"));
        }
        let mut continuous = false;
        if let Some((previous, previous_civilians)) = &self.last_observation {
            if observation.anchor.fortress_id != previous.anchor.fortress_id {
                return Err(invalid("alert monitor cannot switch fortress identity"));
            }
            if observation == *previous && civilians == *previous_civilians {
                if observation.threat == ThreatAssessment::Present {
                    self.emergency_seen = true;
                }
                return Ok(self.decision(observation, civilians));
            }
            let anchor = observation.anchor;
            let prior = previous.anchor;
            if anchor.cursor <= prior.cursor
                || (anchor.cursor.epoch == prior.cursor.epoch && anchor.tick < prior.tick)
            {
                return Err(invalid("stale or conflicting alert observation"));
            }
            continuous = prior.cursor.checked_next() == Some(anchor.cursor)
                && anchor.tick > prior.tick
                && anchor.tick.0 - prior.tick.0 <= self.recovery_policy.maximum_sample_gap_ticks
                && observation.source_scope == previous.source_scope
                && civilians == *previous_civilians
                && previous.complete_civilian_roster;
        }
        if !continuous {
            self.reset_clearance();
        }
        match observation.threat {
            ThreatAssessment::Present => {
                self.emergency_seen = true;
                self.reset_clearance();
            }
            ThreatAssessment::Clear if observation.complete_civilian_roster => {
                if self.clear_since.is_none() {
                    self.clear_since = Some(observation.anchor.tick);
                }
                self.clear_samples = self.clear_samples.saturating_add(1);
            }
            ThreatAssessment::Clear | ThreatAssessment::Unknown => self.reset_clearance(),
        }
        let decision = self.decision(observation, civilians.clone());
        self.last_observation = Some((observation, civilians));
        Ok(decision)
    }

    fn decision(&self, observation: AlertObservation, civilians: Vec<EntityId>) -> AlertDecision {
        let protected = self.emergency_seen
            || matches!(
                self.current_level,
                ThreatLevel::EmergencyLockdown | ThreatLevel::Recovering
            );
        let release_ready = observation.threat == ThreatAssessment::Clear
            && observation.complete_civilian_roster
            && self.clear_samples >= self.recovery_policy.minimum_clear_samples
            && self.clear_since.is_some_and(|since| {
                observation.anchor.tick.0.saturating_sub(since.0)
                    >= self.recovery_policy.minimum_clear_ticks
            });
        let target_level = match observation.threat {
            ThreatAssessment::Present => ThreatLevel::EmergencyLockdown,
            ThreatAssessment::Unknown if protected => ThreatLevel::EmergencyLockdown,
            ThreatAssessment::Unknown => ThreatLevel::Cautious,
            ThreatAssessment::Clear if protected && !release_ready => ThreatLevel::Recovering,
            ThreatAssessment::Clear => ThreatLevel::Peace,
        };
        // Reassert protection for newly observed civilians even if the level is
        // unchanged. These are idempotent semantic proposals, not retry permits.
        let actions = match target_level {
            ThreatLevel::EmergencyLockdown | ThreatLevel::Recovering => {
                self.protection_actions(civilians, true)
            }
            ThreatLevel::Peace if protected && release_ready => {
                self.protection_actions(civilians, false)
            }
            ThreatLevel::Peace | ThreatLevel::Cautious => Vec::new(),
        };
        AlertDecision {
            anchor: observation.anchor,
            source_scope: observation.source_scope,
            target_level,
            actions,
            clear_since: self.clear_since,
            clear_samples: self.clear_samples,
            release_ready,
        }
    }

    fn protection_actions(&self, civilians: Vec<EntityId>, assigned: bool) -> Vec<Action> {
        let mut actions = Vec::with_capacity(2);
        if !civilians.is_empty() {
            actions.push(Action::SetBurrowMembership {
                units: civilians,
                burrow: self.safe_burrow_id,
                assigned,
            });
        }
        actions.push(Action::SetStandingOrder {
            key: "CIVILIAN_BURROW_RESTRICTION".to_owned(),
            value: if assigned { "ACTIVE" } else { "INACTIVE" }.to_owned(),
        });
        actions
    }

    fn reset_clearance(&mut self) {
        self.clear_since = None;
        self.clear_samples = 0;
    }

    /// The shell must independently establish this observed defensive level.
    /// A plan or transport acknowledgment is not an observation. Confirming an
    /// observed Peace retires the emergency latch, not the accepted source floor.
    pub fn confirm_observed_level(&mut self, observed_level: ThreatLevel) {
        let was_protected = matches!(
            self.current_level,
            ThreatLevel::EmergencyLockdown | ThreatLevel::Recovering
        );
        if matches!(observed_level, ThreatLevel::EmergencyLockdown | ThreatLevel::Recovering) {
            self.emergency_seen = true;
            if !was_protected {
                self.reset_clearance();
            }
        }
        self.current_level = observed_level;
        if observed_level == ThreatLevel::Peace {
            self.emergency_seen = false;
            self.reset_clearance();
        }
    }
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn canonical_units(units: &[EntityId]) -> Result<Vec<EntityId>> {
    if units.len() > MAX_ALERT_UNITS {
        return Err(DfmcpError::new(ErrorCode::BudgetExceeded, "alert roster exceeds 512 units"));
    }
    let unique: BTreeSet<_> = units.iter().copied().collect();
    if unique.len() != units.len() || unique.contains(&EntityId::NIL) {
        return Err(invalid("alert roster requires unique nonzero unit identities"));
    }
    Ok(unique.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::ObservationCursor;

    fn fsm() -> Result<CivilianAlertFsm> {
        CivilianAlertFsm::with_recovery_policy(
            EntityId::new(10),
            vec![EntityId::new(20)],
            AlertRecoveryPolicy {
                minimum_clear_ticks: 10,
                minimum_clear_samples: 2,
                maximum_sample_gap_ticks: 10,
            },
        )
    }

    fn observation(sequence: u64, tick: u64, threat: ThreatAssessment) -> AlertObservation {
        AlertObservation {
            anchor: StateAnchor {
                fortress_id: FortressId::new(1),
                cursor: ObservationCursor { epoch: 1, sequence },
                tick: GameTick(tick),
                state_hash: Digest32::of_bytes(&sequence.to_le_bytes()),
            },
            source_scope: Digest32::of_bytes(b"test threat scope"),
            threat,
            complete_civilian_roster: true,
        }
    }

    fn releases(decision: &AlertDecision) -> bool {
        decision.actions.iter().any(|action| matches!(action,
            Action::SetBurrowMembership { assigned: false, .. }
        ))
    }

    #[test]
    fn test_alert_lockdown_and_recovery_cycle() -> Result<()> {
        let mut fsm = fsm()?;
        let civilians = [EntityId::new(1), EntityId::new(2)];
        assert_eq!(fsm.transition_to(ThreatLevel::EmergencyLockdown, &civilians)?.len(), 2);
        assert_eq!(fsm.current_level(), ThreatLevel::Peace);
        fsm.confirm_observed_level(ThreatLevel::EmergencyLockdown);
        let recovery = fsm.transition_to(ThreatLevel::Recovering, &civilians)?;
        assert!(matches!(&recovery[0], Action::SetBurrowMembership { assigned: true, .. }));
        let peace = fsm.transition_to(ThreatLevel::Peace, &civilians)?;
        assert!(matches!(&peace[0], Action::SetBurrowMembership { assigned: false, .. }));
        Ok(())
    }

    #[test]
    fn only_advancing_complete_clear_window_can_release() -> Result<()> {
        let mut fsm = fsm()?;
        let civilians = [EntityId::new(1)];
        let danger = fsm.plan_observation(observation(1, 0, ThreatAssessment::Present), &civilians)?;
        assert_eq!(danger.target_level, ThreatLevel::EmergencyLockdown);
        assert_eq!(fsm.current_level(), ThreatLevel::Peace);
        let first = observation(2, 1, ThreatAssessment::Clear);
        let recovery = fsm.plan_observation(first, &civilians)?;
        assert_eq!(recovery.target_level, ThreatLevel::Recovering);
        assert!(!releases(&recovery));
        assert_eq!(fsm.plan_observation(first, &civilians)?, recovery);
        let ready = fsm.plan_observation(observation(3, 11, ThreatAssessment::Clear), &civilians)?;
        assert!(ready.release_ready && releases(&ready));
        assert_eq!(ready.clear_samples, 2);
        assert_eq!(fsm.current_level(), ThreatLevel::Peace);
        fsm.confirm_observed_level(ThreatLevel::Peace);
        let peaceful = fsm.plan_observation(
            observation(4, 12, ThreatAssessment::Clear), &civilians,
        )?;
        assert!(peaceful.actions.is_empty());
        Ok(())
    }

    #[test]
    fn unknown_and_partial_coverage_cannot_end_lockdown() -> Result<()> {
        for partial in [false, true] {
            let mut fsm = fsm()?;
            fsm.confirm_observed_level(ThreatLevel::EmergencyLockdown);
            let civilians = [EntityId::new(1)];
            fsm.plan_observation(observation(1, 0, ThreatAssessment::Clear), &civilians)?;
            let mut broken = observation(2, 10, ThreatAssessment::Unknown);
            if partial {
                broken.threat = ThreatAssessment::Clear;
                broken.complete_civilian_roster = false;
            }
            let decision = fsm.plan_observation(broken, &civilians)?;
            assert!(!decision.release_ready && !releases(&decision));
            assert_eq!(decision.clear_samples, 0);
            let fresh = fsm.plan_observation(observation(3, 20, ThreatAssessment::Clear), &civilians)?;
            assert_eq!(fresh.clear_samples, 1);
            assert!(!releases(&fresh));
        }
        Ok(())
    }

    #[test]
    fn gaps_epoch_source_and_roster_changes_restart_clearance() -> Result<()> {
        for mutation in 0..5 {
            let mut fsm = fsm()?;
            fsm.confirm_observed_level(ThreatLevel::EmergencyLockdown);
            let civilians = [EntityId::new(1)];
            fsm.plan_observation(observation(1, 0, ThreatAssessment::Clear), &civilians)?;
            let mut next = observation(2, 10, ThreatAssessment::Clear);
            let mut changed_civilians = civilians.to_vec();
            match mutation {
                0 => next.anchor.cursor.sequence = 3,
                1 => next.anchor.cursor.epoch = 2,
                2 => next.source_scope = Digest32::of_bytes(b"new source scope"),
                3 => changed_civilians.push(EntityId::new(2)),
                _ => next.anchor.tick = GameTick(11),
            }
            let decision = fsm.plan_observation(next, &changed_civilians)?;
            assert_eq!(decision.clear_samples, 1);
            assert!(!releases(&decision));
        }
        Ok(())
    }

    #[test]
    fn paused_ticks_do_not_build_clearance() -> Result<()> {
        let mut fsm = fsm()?;
        fsm.confirm_observed_level(ThreatLevel::EmergencyLockdown);
        for sequence in 1..10 {
            let result = fsm.plan_observation(observation(sequence, 0, ThreatAssessment::Clear), &[])?;
            assert_eq!(result.clear_samples, 1);
            assert!(!result.release_ready);
        }
        Ok(())
    }

    #[test]
    fn stale_conflicting_and_invalid_observations_clear_proof_not_floor() -> Result<()> {
        for mutation in 0..5 {
            let mut fsm = fsm()?;
            fsm.confirm_observed_level(ThreatLevel::EmergencyLockdown);
            let good = observation(2, 10, ThreatAssessment::Clear);
            fsm.plan_observation(good, &[])?;
            let mut bad = good;
            match mutation {
                0 => bad.anchor.cursor.sequence = 1,
                1 => bad.anchor.state_hash = Digest32::of_bytes(b"fork"),
                2 => bad.threat = ThreatAssessment::Present,
                3 => bad.anchor.fortress_id = FortressId::new(2),
                _ => bad.source_scope = Digest32::ZERO,
            }
            assert!(fsm.plan_observation(bad, &[]).is_err());
            let replay = fsm.plan_observation(good, &[])?;
            assert_eq!(replay.clear_samples, 0);
            let next = fsm.plan_observation(observation(3, 20, ThreatAssessment::Clear), &[])?;
            assert_eq!(next.clear_samples, 1);
            assert!(!next.release_ready);
        }
        Ok(())
    }

    #[test]
    fn recurrent_danger_reasserts_protection_without_confirmation() -> Result<()> {
        let mut fsm = fsm()?;
        let first = [EntityId::new(3), EntityId::new(1)];
        fsm.plan_observation(observation(1, 0, ThreatAssessment::Present), &first)?;
        let mut all = first.to_vec();
        all.push(EntityId::new(2));
        let decision = fsm.plan_observation(observation(2, 1, ThreatAssessment::Unknown), &all)?;
        assert_eq!(decision.target_level, ThreatLevel::EmergencyLockdown);
        assert!(matches!(&decision.actions[0], Action::SetBurrowMembership { units, assigned: true, .. }
            if units == &vec![EntityId::new(1), EntityId::new(2), EntityId::new(3)]));
        Ok(())
    }

    #[test]
    fn entering_observed_lockdown_cannot_reuse_peacetime_clearance() -> Result<()> {
        let mut fsm = fsm()?;
        let first = observation(1, 0, ThreatAssessment::Clear);
        let second = observation(2, 10, ThreatAssessment::Clear);
        fsm.plan_observation(first, &[])?;
        assert!(fsm.plan_observation(second, &[])?.release_ready);
        fsm.confirm_observed_level(ThreatLevel::EmergencyLockdown);
        let replay = fsm.plan_observation(second, &[])?;
        assert_eq!(replay.clear_samples, 0);
        assert!(!replay.release_ready);
        let next = fsm.plan_observation(observation(3, 20, ThreatAssessment::Clear), &[])?;
        assert_eq!(next.clear_samples, 1);
        assert!(!next.release_ready);
        Ok(())
    }

    #[test]
    fn policy_and_roster_bounds_are_enforced() -> Result<()> {
        assert!(CivilianAlertFsm::with_recovery_policy(EntityId::new(1), vec![], AlertRecoveryPolicy {
            minimum_clear_samples: 1, ..AlertRecoveryPolicy::default()
        }).is_err());
        let mut fsm = fsm()?;
        let observation = observation(1, 0, ThreatAssessment::Present);
        assert!(fsm.plan_observation(observation, &[EntityId::NIL]).is_err());
        assert!(fsm.plan_observation(observation, &[EntityId::new(1); 2]).is_err());
        let too_many: Vec<_> = (1..=513).map(EntityId::new).collect();
        assert!(fsm.plan_observation(observation, &too_many).is_err());
        Ok(())
    }
}
