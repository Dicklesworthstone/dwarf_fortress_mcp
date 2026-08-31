#![forbid(unsafe_code)]

use dfmcp_core::{
    Capability, Digest32, FortressId, GameTick, IntentId, ObservationCursor, RiskTier, StateAnchor,
    StepId,
};
use dfmcp_intent::{Action, PlanStep, PreparedPlan};
use dfmcp_world::Predicate;
use std::collections::BTreeSet;

fn sample_plan() -> Result<PreparedPlan, Box<dyn std::error::Error>> {
    let anchor = StateAnchor {
        fortress_id: FortressId::new(42),
        cursor: ObservationCursor {
            epoch: 1,
            sequence: 10,
        },
        tick: GameTick(100),
        state_hash: Digest32::of_bytes(b"anchor_state"),
    };

    let mut caps = BTreeSet::new();
    caps.insert(Capability::ControlClock);

    let step = PlanStep {
        id: StepId::new(0),
        action: Action::Pause { paused: true },
        preconditions: vec![Predicate::Paused(false)],
        postconditions: vec![Predicate::Paused(true)],
        compensation: Some(Action::Pause { paused: false }),
        obligation: None,
        depends_on: Vec::new(),
        risk: RiskTier::Reversible,
        required_capability: Capability::ControlClock,
        idempotency_key: Digest32::of_bytes(b"step_0").to_hex(),
    };

    Ok(PreparedPlan::builder(
        IntentId::new(100),
        anchor,
        "Pause simulation for inspection",
        Predicate::Paused(true),
    )
    .steps(vec![step])
    .max_risk(RiskTier::Reversible)
    .required_capabilities(caps)
    .requires_checkpoint(false)
    .expires_at_tick(GameTick(500))
    .build())
}

#[test]
fn test_plan_digest_and_id_validity() -> Result<(), Box<dyn std::error::Error>> {
    let plan = sample_plan()?;
    assert!(plan.digest_is_valid());
    assert!(plan.id_is_valid());
    assert!(plan.validate_structure().is_ok());
    Ok(())
}

#[test]
fn test_covered_field_mutation_invalidates_plan_digest() -> Result<(), Box<dyn std::error::Error>> {
    let original = sample_plan()?;

    // 1. Mutate intent_id
    let mut mutated = original.clone();
    mutated.intent_id = IntentId::new(101);
    assert!(!mutated.digest_is_valid());
    assert!(mutated.validate_structure().is_err());

    // 2. Mutate anchor fortress_id
    let mut mutated = original.clone();
    mutated.anchor.fortress_id = FortressId::new(999);
    assert!(!mutated.digest_is_valid());
    assert!(mutated.validate_structure().is_err());

    // 3. Mutate anchor cursor
    let mut mutated = original.clone();
    mutated.anchor.cursor.sequence += 1;
    assert!(!mutated.digest_is_valid());
    assert!(mutated.validate_structure().is_err());

    // 4. Mutate anchor tick
    let mut mutated = original.clone();
    mutated.anchor.tick = GameTick(101);
    assert!(!mutated.digest_is_valid());
    assert!(mutated.validate_structure().is_err());

    // 5. Mutate anchor state_hash
    let mut mutated = original.clone();
    mutated.anchor.state_hash = Digest32::ZERO;
    assert!(!mutated.digest_is_valid());
    assert!(mutated.validate_structure().is_err());

    // 6. Mutate summary
    let mut mutated = original.clone();
    mutated.summary = "Tampered summary".to_string();
    assert!(!mutated.digest_is_valid());
    assert!(mutated.validate_structure().is_err());

    // 7. Mutate terminal condition
    let mut mutated = original.clone();
    mutated.terminal_condition = Predicate::Paused(false);
    assert!(!mutated.digest_is_valid());
    assert!(mutated.validate_structure().is_err());

    // 8. Mutate max_risk
    let mut mutated = original.clone();
    mutated.max_risk = RiskTier::Irreversible;
    assert!(!mutated.digest_is_valid());
    assert!(mutated.validate_structure().is_err());

    // 9. Mutate required_capabilities
    let mut mutated = original.clone();
    mutated.required_capabilities.insert(Capability::Checkpoint);
    assert!(!mutated.digest_is_valid());
    assert!(mutated.validate_structure().is_err());

    // 10. Mutate requires_checkpoint
    let mut mutated = original.clone();
    mutated.requires_checkpoint = true;
    assert!(!mutated.digest_is_valid());
    assert!(mutated.validate_structure().is_err());

    // 11. Mutate expires_at_tick
    let mut mutated = original.clone();
    mutated.expires_at_tick = GameTick(999);
    assert!(!mutated.digest_is_valid());
    assert!(mutated.validate_structure().is_err());

    // 12. Mutate step action
    let mut mutated = original.clone();
    mutated.steps[0].action = Action::Pause { paused: false };
    assert!(!mutated.digest_is_valid());
    assert!(mutated.validate_structure().is_err());

    // 13. Mutate step idempotency_key
    let mut mutated = original.clone();
    mutated.steps[0].idempotency_key = Digest32::of_bytes(b"tampered_idempotency").to_hex();
    assert!(!mutated.digest_is_valid());
    assert!(mutated.validate_structure().is_err());
    Ok(())
}
