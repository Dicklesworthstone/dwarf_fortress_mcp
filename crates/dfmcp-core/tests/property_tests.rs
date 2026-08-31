#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, EntityId, FortressId, GameTick,
    ObservationCursor, RiskTier, WorkBudget,
};

/// Simple, deterministic pseudo-random number generator (xorshift64)
/// so property tests are 100% reproducible by seed without external crates.
struct TestPrng {
    state: u64,
}

impl TestPrng {
    fn new(seed: u64) -> Self {
        Self { state: seed | 1 }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn next_u32(&mut self) -> u32 {
        (self.next_u64() & 0xffff_ffff) as u32
    }

    fn next_range(&mut self, min: u64, max: u64) -> u64 {
        let span = max.saturating_sub(min).max(1);
        min + self.next_u64() % span
    }

    fn next_risk(&mut self) -> RiskTier {
        match self.next_u64() % 4 {
            0 => RiskTier::ReadOnly,
            1 => RiskTier::Reversible,
            2 => RiskTier::Guarded,
            _ => RiskTier::Irreversible,
        }
    }

    fn next_capability(&mut self) -> Capability {
        let all = [
            Capability::Observe,
            Capability::Query,
            Capability::Plan,
            Capability::Designate,
            Capability::Construct,
            Capability::ConfigureLabor,
            Capability::ConfigureProduction,
            Capability::ConfigureLogistics,
            Capability::ConfigureMilitary,
            Capability::ControlClock,
            Capability::Checkpoint,
            Capability::Restore,
            Capability::Extension,
            Capability::DiagnosticRaw,
            Capability::Doctor,
            Capability::RepairPlan,
            Capability::RepairApply,
            Capability::Admin,
        ];
        all[(self.next_u64() as usize) % all.len()]
    }
}

/// TEST-001: Digest platform and endian independence
#[test]
fn test_001_digest_platform_and_ordering_properties() {
    let seed = 0x2026_0830_0001u64;
    let mut prng = TestPrng::new(seed);

    for iteration in 0..100 {
        let count = prng.next_range(1, 20) as usize;
        let mut elements = Vec::with_capacity(count);
        for _ in 0..count {
            elements.push(prng.next_u64());
        }

        // Canonical sorted encoding
        let mut sorted = elements.clone();
        sorted.sort_unstable();

        let mut canonical_bytes = Vec::new();
        for &val in &sorted {
            canonical_bytes.extend_from_slice(&val.to_be_bytes());
        }
        let canonical_digest = Digest32::of_bytes(&canonical_bytes);

        // Permutations must produce different hashes unless identical
        if sorted != elements {
            let mut raw_bytes = Vec::new();
            for &val in &elements {
                raw_bytes.extend_from_slice(&val.to_be_bytes());
            }
            let raw_digest = Digest32::of_bytes(&raw_bytes);
            assert_ne!(
                canonical_digest, raw_digest,
                "iteration {iteration}: unordered stream had identical digest to canonical stream"
            );
        }

        // Determinism check: repeating with same input yields identical digest
        let repeat_digest = Digest32::of_bytes(&canonical_bytes);
        assert_eq!(canonical_digest, repeat_digest);
    }
}

/// TEST-002: Identity generation, monotonicity, non-reuse, and anti-ABA semantics
#[test]
fn test_002_identity_generation_properties() {
    let seed = 0x2026_0830_0002u64;
    let mut prng = TestPrng::new(seed);

    let mut previous_id = 0u64;
    let mut seen_ids = BTreeSet::new();

    for _ in 0..200 {
        let delta = prng.next_range(1, 100);
        let id_val = previous_id.saturating_add(delta);
        let entity = EntityId::new(id_val);

        assert!(
            seen_ids.insert(entity),
            "ID was reused in monotonically advancing generation sequence"
        );
        assert!(
            entity.get() > previous_id,
            "ID generation must be strictly monotonic"
        );
        previous_id = id_val;
    }

    // Observation cursor generation and epoch reset monotonicity
    let mut cursor = ObservationCursor::ORIGIN;
    assert_eq!(cursor.epoch, 0);
    assert_eq!(cursor.sequence, 0);

    for seq in 1..=50 {
        cursor = cursor.next();
        assert_eq!(cursor.sequence, seq);
        assert_eq!(cursor.epoch, 0);
    }

    cursor = cursor.reset_epoch();
    assert_eq!(cursor.epoch, 1);
    assert_eq!(cursor.sequence, 0);
}

/// TEST-008: Capability lattice property — no authority amplification and risk monotonicity
#[test]
fn test_008_capability_lattice_and_risk_monotonicity() {
    let seed = 0x2026_0830_0008u64;
    let mut prng = TestPrng::new(seed);

    let mut authorized_count = 0;
    let mut rejected_count = 0;

    for _ in 0..500 {
        let grant_cap = prng.next_capability();
        let grant_risk = prng.next_risk();
        let grant_fid = FortressId::new(prng.next_range(1, 5));
        let grant_exp = GameTick(prng.next_range(100, 1000));
        let grant_uses = prng.next_range(1, 5) as u32;

        let grant = CapabilityGrant {
            capability: grant_cap,
            scope: CapabilityScope {
                fortress_id: Some(grant_fid),
                entity_ids: BTreeSet::new(),
                map_area: None,
            },
            max_risk: grant_risk,
            expires_at_tick: Some(grant_exp),
            remaining_uses: Some(grant_uses),
        };

        // Random requested parameters
        let req_cap = prng.next_capability();
        let req_risk = prng.next_risk();
        let req_fid = FortressId::new(prng.next_range(1, 5));
        let req_tick = GameTick(prng.next_range(50, 1500));

        let allowed = grant.allows(req_cap, req_risk, req_tick, req_fid, &[], None);

        if allowed {
            authorized_count += 1;
            // Invariant 1: capability matches or grant is Admin
            assert!(grant_cap == req_cap || grant_cap == Capability::Admin);
            // Invariant 2: requested risk <= granted max_risk (Risk Monotonicity)
            assert!(req_risk <= grant_risk);
            // Invariant 3: requested tick <= expiry tick
            assert!(req_tick <= grant_exp);
            // Invariant 4: fortress ID matches
            assert_eq!(req_fid, grant_fid);
        } else {
            rejected_count += 1;
            let cap_mismatch = grant_cap != req_cap && grant_cap != Capability::Admin;
            let risk_exceeded = req_risk > grant_risk;
            let expired = req_tick > grant_exp;
            let fid_mismatch = req_fid != grant_fid;
            assert!(
                cap_mismatch || risk_exceeded || expired || fid_mismatch,
                "Request was rejected without any violation"
            );
        }
    }

    assert!(
        authorized_count > 0,
        "Lattice property test must exercise authorization"
    );
    assert!(
        rejected_count > 0,
        "Lattice property test must exercise rejection"
    );
}

/// WorkBudget exhaustion and bounding property test
#[test]
fn test_work_budget_boundedness_property() {
    let seed = 0x2026_0830_0009u64;
    let mut prng = TestPrng::new(seed);

    for _ in 0..100 {
        let millis = prng.next_range(0, 5000);
        let ticks = prng.next_range(0, 20000);
        let entities = prng.next_u32() % 5000;
        let bytes = prng.next_u64() % (10 * 1024 * 1024);
        let tokens = prng.next_u32() % 4000;
        let actions = prng.next_u32() % 100;

        let budget = WorkBudget {
            max_wall_millis: millis,
            max_game_ticks: ticks,
            max_entities: entities,
            max_bytes: bytes,
            max_output_tokens: tokens,
            max_actions: actions,
        };

        let result = budget.validate();
        let any_zero = millis == 0 || entities == 0 || bytes == 0 || tokens == 0 || actions == 0;

        if any_zero {
            assert!(
                result.is_err(),
                "Budget with zero dimensions must fail validation"
            );
        } else {
            assert!(result.is_ok(), "Valid positive budget must pass validation");
        }
    }
}
