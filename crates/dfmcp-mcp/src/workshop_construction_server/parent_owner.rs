//! Parent ownership helpers. These retain authority and recovery without I/O.
use super::*;

impl Runtime {
    pub(crate) fn parent_authority_tick_floor(&self) -> u64 {
        self.authority_tick
            .max(self.journal.high_tick())
            .max(
                self.tree_material
                    .as_ref()
                    .map_or(0, tree_material::Owner::high_tick),
            )
            .max(self.clock.as_ref().map_or(0, clock::Owner::high_tick))
    }

    pub(crate) fn retain_parent_authority_floor(&mut self, tick: u64) {
        self.authority_tick = self.parent_authority_tick_floor().max(tick);
    }

    pub(crate) fn parent_effect_uncertain(&self) -> bool {
        self.journal
            .unverified_original_reference()
            .is_some_and(|entry| {
                entry.unresolved() || (entry.dispatch_started() && entry.needs_reconciliation())
            })
            || self.clock.as_ref().is_some_and(|clock| {
                let reference = clock.reference();
                reference["unresolved"].as_bool() == Some(true)
                    || reference["pending"].as_bool() == Some(true)
            })
    }

    /// Cached references are not a new authentication of the files. In
    /// particular an unacknowledged journal head is never substituted for ACK.
    pub(crate) fn parent_recovery_reference(&self) -> Value {
        let acknowledged = self
            .guard
            .acks
            .try_lock()
            .ok()
            .and_then(|acknowledgements| acknowledgements.latest());
        json!({
            "acknowledged":acknowledged.map(pin_json),
            "journal_pin_unverified":pin_json(self.journal.pin()),
            "authority_tick_floor":self.parent_authority_tick_floor(),
            "effect_uncertain":self.parent_effect_uncertain(),
            "original":self.journal.unverified_original_reference().map(|entry|json!({
                "key":entry.plan().key(),"plan_digest":entry.plan().digest().to_hex(),
                "state":entry.state().as_str(),"dispatch_started":entry.dispatch_started(),
                "unresolved":entry.unresolved(),
            })),
            "clock":self.clock.as_ref().map(clock::Owner::reference),
            "custody_currently_verified":false,
            "current_goal_available":false,"automatic_redispatch":false,
        })
    }
}
