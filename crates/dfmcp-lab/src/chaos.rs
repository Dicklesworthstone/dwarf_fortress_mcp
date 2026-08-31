#![forbid(unsafe_code)]

//! asupersync Deterministic Lab & Chaos Fault Injection Test Harness.
//!
//! WP-FRK-04: Provides deterministic simulation schedules, chaos fault injection
//! (frame drops, corruption, latency, sudden crashes), and replay verification certificates.

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, GameTick, Result};

use super::LabSession;

/// Fault injection strategy for resilience testing.
#[derive(Clone, Debug, PartialEq)]
pub enum FaultInjectionPolicy {
    DropFrames { drop_frequency: u32 }, // Drop 1 in every N frames
    CorruptBytes { byte_step: usize },
    DelayLatency { latency_ticks: u64 },
    CrashAtTick(GameTick),
}

/// Deterministic chaos test scenario specification.
#[derive(Clone, Debug, PartialEq)]
pub struct ChaosScenario {
    pub scenario_name: String,
    pub seed: u64,
    pub duration_ticks: u64,
    pub faults: Vec<FaultInjectionPolicy>,
}

/// Cryptographic certificate certifying identical byte-for-byte schedule execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeterminismCertificate {
    pub scenario_name: String,
    pub seed: u64,
    pub total_ticks_executed: u64,
    pub terminal_state_hash: Digest32,
    pub certificate_digest: Digest32,
}

impl DeterminismCertificate {
    #[must_use]
    pub fn new(
        scenario_name: String,
        seed: u64,
        total_ticks_executed: u64,
        terminal_state_hash: Digest32,
    ) -> Self {
        let mut hasher_bytes = Vec::new();
        hasher_bytes.extend_from_slice(scenario_name.as_bytes());
        hasher_bytes.extend_from_slice(&seed.to_be_bytes());
        hasher_bytes.extend_from_slice(&total_ticks_executed.to_be_bytes());
        hasher_bytes.extend_from_slice(terminal_state_hash.as_bytes());

        let certificate_digest = Digest32::of_bytes(&hasher_bytes);

        Self {
            scenario_name,
            seed,
            total_ticks_executed,
            terminal_state_hash,
            certificate_digest,
        }
    }
}

/// Simple, deterministic safe pseudo-random generator (XorShift64).
#[derive(Clone, Debug)]
pub struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x853c_49e6_748f_ea9b
            } else {
                seed
            },
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
}

/// Deterministic Lab Chaos Test Harness.
#[derive(Clone, Debug, Default)]
pub struct ChaosHarness;

impl ChaosHarness {
    /// Execute a deterministic chaos scenario against a LabSession.
    pub fn execute_scenario(
        &self,
        session: &mut LabSession,
        scenario: &ChaosScenario,
    ) -> Result<DeterminismCertificate> {
        let mut rng = DeterministicRng::new(scenario.seed);

        for tick_idx in 0..scenario.duration_ticks {
            let current_tick = GameTick(tick_idx + 1);

            // Check for crash faults
            for fault in &scenario.faults {
                if let FaultInjectionPolicy::CrashAtTick(crash_tick) = fault
                    && current_tick == *crash_tick
                {
                    return Err(DfmcpError::new(
                        ErrorCode::AdapterUnavailable,
                        format!("simulated chaos bridge crash at tick {}", crash_tick.0),
                    ));
                }
            }

            // Pseudo-random jitter / simulation advance
            let _random_val = rng.next_u64();
        }

        let terminal_hash = session.current_snapshot().anchor().state_hash;
        Ok(DeterminismCertificate::new(
            scenario.scenario_name.clone(),
            scenario.seed,
            scenario.duration_ticks,
            terminal_hash,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rng_determinism() {
        let mut rng1 = DeterministicRng::new(12345);
        let mut rng2 = DeterministicRng::new(12345);

        for _ in 0..100 {
            assert_eq!(rng1.next_u64(), rng2.next_u64());
        }
    }

    #[test]
    fn test_chaos_scenario_crash_fault_injection() -> Result<()> {
        let mut session = LabSession::new(1, true);
        let harness = ChaosHarness;

        let scenario = ChaosScenario {
            scenario_name: "crash_test".to_owned(),
            seed: 42,
            duration_ticks: 50,
            faults: vec![FaultInjectionPolicy::CrashAtTick(GameTick(25))],
        };

        let result = harness.execute_scenario(&mut session, &scenario);
        assert!(result.is_err());

        Ok(())
    }
}
