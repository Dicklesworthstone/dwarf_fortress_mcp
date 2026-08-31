#![forbid(unsafe_code)]

//! Multi-Agent Clock & Game-Speed Governance Protocol.
//!
//! WP-LEA-02: Coordinates simulation pause/unpause and tick rate advancement across
//! multiple concurrent autonomous agent sessions. Enforces emergency pause overrides
//! (any agent can instantly halt simulation) and unpause quorum consensus.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::{DfmcpError, ErrorCode, Result};
use crate::ids::SessionId;
use crate::model::GameTick;

/// Governance policy for simulation clock control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClockPolicy {
    /// Any single agent can pause, all registered agents must agree to unpause.
    UnanimousUnpause,
    /// Majority of active agents must agree to unpause.
    MajorityUnpause,
}

/// Multi-Agent Clock Speed and Simulation Advancement Governor.
#[derive(Clone, Debug)]
pub struct ClockGovernor {
    policy: ClockPolicy,
    registered_sessions: BTreeSet<SessionId>,
    unpause_votes: BTreeSet<SessionId>,
    emergency_pauses: BTreeSet<SessionId>,
    session_tick_budgets: BTreeMap<SessionId, u64>,
    current_tick: GameTick,
}

impl Default for ClockGovernor {
    fn default() -> Self {
        Self::new(ClockPolicy::UnanimousUnpause)
    }
}

impl ClockGovernor {
    #[must_use]
    pub fn new(policy: ClockPolicy) -> Self {
        Self {
            policy,
            registered_sessions: BTreeSet::new(),
            unpause_votes: BTreeSet::new(),
            emergency_pauses: BTreeSet::new(),
            session_tick_budgets: BTreeMap::new(),
            current_tick: GameTick(0),
        }
    }

    /// Register a participating agent session.
    pub fn register_session(&mut self, session_id: SessionId, initial_budget: u64) {
        self.registered_sessions.insert(session_id);
        self.session_tick_budgets.insert(session_id, initial_budget);
    }

    /// Unregister a session on disconnect.
    pub fn unregister_session(&mut self, session_id: SessionId) {
        self.registered_sessions.remove(&session_id);
        self.unpause_votes.remove(&session_id);
        self.emergency_pauses.remove(&session_id);
        self.session_tick_budgets.remove(&session_id);
    }

    /// Emergency pause override: any single agent can immediately halt simulation.
    pub fn request_emergency_pause(&mut self, session_id: SessionId) {
        self.emergency_pauses.insert(session_id);
    }

    /// Release an emergency pause previously requested by this session.
    pub fn release_emergency_pause(&mut self, session_id: SessionId) {
        self.emergency_pauses.remove(&session_id);
    }

    /// Vote to unpause the simulation.
    pub fn vote_unpause(&mut self, session_id: SessionId) {
        if self.registered_sessions.contains(&session_id) {
            self.unpause_votes.insert(session_id);
        }
    }

    /// Vote to pause the simulation.
    pub fn vote_pause(&mut self, session_id: SessionId) {
        self.unpause_votes.remove(&session_id);
    }

    /// Determine if the simulation is currently allowed to advance (unpaused).
    #[must_use]
    pub fn is_unpaused(&self) -> bool {
        // 1. If any emergency pause is active, clock is halted
        if !self.emergency_pauses.is_empty() {
            return false;
        }

        if self.registered_sessions.is_empty() {
            return false;
        }

        // 2. Check unpause policy consensus
        match self.policy {
            ClockPolicy::UnanimousUnpause => {
                self.unpause_votes.len() >= self.registered_sessions.len()
            }
            ClockPolicy::MajorityUnpause => {
                self.unpause_votes.len() > self.registered_sessions.len() / 2
            }
        }
    }

    /// Consume tick budget for a session executing simulation steps.
    pub fn consume_tick_budget(&mut self, session_id: SessionId, ticks: u64) -> Result<()> {
        let budget = self
            .session_tick_budgets
            .get_mut(&session_id)
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::SessionNotFound,
                    "session not registered with governor",
                )
            })?;

        if *budget < ticks {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                format!(
                    "session {} exceeded tick budget (available: {}, requested: {})",
                    session_id.get(),
                    *budget,
                    ticks
                ),
            ));
        }

        *budget = budget.saturating_sub(ticks);
        Ok(())
    }

    /// Advance governor clock by `ticks` if unpaused.
    pub fn advance_ticks(&mut self, ticks: u64) -> Result<GameTick> {
        if !self.is_unpaused() {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "cannot advance clock while simulation is paused by governor policy",
            ));
        }

        self.current_tick = GameTick(self.current_tick.0.saturating_add(ticks));
        Ok(self.current_tick)
    }

    /// Current game tick.
    #[must_use]
    pub const fn current_tick(&self) -> GameTick {
        self.current_tick
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unanimous_unpause_governance() -> Result<()> {
        let mut governor = ClockGovernor::new(ClockPolicy::UnanimousUnpause);
        let s1 = SessionId::new(1);
        let s2 = SessionId::new(2);

        governor.register_session(s1, 100);
        governor.register_session(s2, 100);

        // Initially paused
        assert!(!governor.is_unpaused());

        // s1 votes unpause -> still paused (requires unanimous)
        governor.vote_unpause(s1);
        assert!(!governor.is_unpaused());

        // s2 votes unpause -> now unpaused!
        governor.vote_unpause(s2);
        assert!(governor.is_unpaused());

        // Advancing ticks works
        let new_tick = governor.advance_ticks(10)?;
        assert_eq!(new_tick, GameTick(10));

        // Emergency pause by s1 halts clock immediately
        governor.request_emergency_pause(s1);
        assert!(!governor.is_unpaused());

        Ok(())
    }
}
