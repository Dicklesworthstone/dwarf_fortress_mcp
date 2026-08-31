#![forbid(unsafe_code)]

//! Integration tests for WP-FRK-04 asupersync Deterministic Lab & Chaos Fault Harness.

use dfmcp_core::Result;
use dfmcp_lab::LabSession;
use dfmcp_lab::chaos::{ChaosHarness, ChaosScenario, FaultInjectionPolicy};

#[test]
fn test_deterministic_scenario_reproducibility() -> Result<()> {
    let mut session1 = LabSession::new(1, true);
    let mut session2 = LabSession::new(1, true);
    let harness = ChaosHarness;

    let scenario = ChaosScenario {
        scenario_name: "reproducible_schedule".to_owned(),
        seed: 987654321,
        duration_ticks: 100,
        faults: vec![FaultInjectionPolicy::DelayLatency { latency_ticks: 5 }],
    };

    let cert1 = harness.execute_scenario(&mut session1, &scenario)?;
    let cert2 = harness.execute_scenario(&mut session2, &scenario)?;

    assert_eq!(cert1.scenario_name, cert2.scenario_name);
    assert_eq!(cert1.seed, cert2.seed);
    assert_eq!(cert1.total_ticks_executed, cert2.total_ticks_executed);
    assert_eq!(cert1.terminal_state_hash, cert2.terminal_state_hash);
    assert_eq!(cert1.certificate_digest, cert2.certificate_digest);

    Ok(())
}
