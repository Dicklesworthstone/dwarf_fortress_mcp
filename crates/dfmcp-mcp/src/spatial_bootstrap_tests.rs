use super::*;
#[path = "../../dfmcp-adapter/tests/support/production_portfolio_spatial.rs"]
mod fixture;

#[derive(Clone, Default)]
struct Spy {
    connects: Arc<AtomicUsize>,
    reads: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    allowances: Arc<Mutex<Vec<Duration>>>,
}
struct Script {
    observation: Option<LiveSpatialCitizenObservation>,
    spy: Spy,
}
impl Source for Script {
    fn read(&mut self, allowance: Duration) -> Result<LiveSpatialCitizenObservation> {
        self.spy.reads.fetch_add(1, Ordering::SeqCst);
        lock(&self.spy.allowances)?.push(allowance);
        self.observation
            .take()
            .ok_or_else(|| error(ErrorCode::AdapterUnavailable, "bootstrap fixture exhausted"))
    }
    fn poisoned(&self) -> bool {
        false
    }
    fn fence(&mut self) {}
    fn pages(&self) -> u32 {
        1
    }
}
impl Drop for Script {
    fn drop(&mut self) {
        self.spy.drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn limits() -> CitizenSpatialLimits {
    CitizenSpatialLimits {
        spatial: SpatialLimits {
            operations: PagedOperationsLimits::default(),
            region: Region {
                origin: [0, 0, 5],
                size: [3, 3, 1],
            },
        },
        citizens: 4096,
    }
}
fn budget() -> WorkBudget {
    WorkBudget {
        max_entities: limits().entity_limit(),
        max_wall_millis: 60000,
        ..WorkBudget::default()
    }
}
fn run(
    spy: &Spy,
    limits: CitizenSpatialLimits,
    budget: WorkBudget,
    times: &[u64],
) -> Result<(Script, LiveSpatialCitizenState)> {
    let observation = fixture::observation(3, 2, 2, false)?;
    let mut times = times.iter().copied();
    establish(
        limits,
        budget,
        |allowance| {
            spy.connects.fetch_add(1, Ordering::SeqCst);
            lock(&spy.allowances)?.push(allowance);
            Ok(Script {
                observation: Some(observation),
                spy: spy.clone(),
            })
        },
        || Duration::from_millis(times.next().unwrap_or(60000)),
    )
}

#[test]
fn bootstrap_connect_and_first_capture_receive_a_decreasing_shared_allowance() -> Result<()> {
    let spy = Spy::default();
    let (source, state) = run(&spy, limits(), budget(), &[1000, 6000, 7000, 8000])?;
    assert_eq!(
        lock(&spy.allowances)?.as_slice(),
        &[Duration::from_secs(59), Duration::from_secs(54)]
    );
    assert!(state.snapshot().is_some());
    assert_eq!(spy.reads.load(Ordering::SeqCst), 1);
    assert_eq!(spy.drops.load(Ordering::SeqCst), 0);
    drop(source);
    assert_eq!(spy.drops.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn exhausted_bootstrap_stages_do_not_return_a_source_or_world() -> Result<()> {
    for (times, connects, reads) in [
        (vec![10], 0, 0),
        (vec![0, 10], 1, 0),
        (vec![0, 1, 10], 1, 1),
        (vec![0, 1, 2, 10], 1, 1),
    ] {
        let spy = Spy::default();
        let mut budget = budget();
        budget.max_wall_millis = 10;
        assert!(
            matches!(run(&spy,limits(),budget,&times),Err(e)if e.code==ErrorCode::BudgetExceeded)
        );
        assert_eq!(spy.connects.load(Ordering::SeqCst), connects);
        assert_eq!(spy.reads.load(Ordering::SeqCst), reads);
        assert_eq!(spy.drops.load(Ordering::SeqCst), connects);
    }
    Ok(())
}

#[test]
fn invalid_configuration_refuses_before_connecting() -> Result<()> {
    for invalid_budget in [false, true] {
        let spy = Spy::default();
        let mut limits = limits();
        let mut budget = budget();
        if invalid_budget {
            budget.max_bytes = 0;
        } else {
            limits.citizens = 0;
        }
        assert!(run(&spy, limits, budget, &[0, 0, 0, 0]).is_err());
        assert_eq!(spy.connects.load(Ordering::SeqCst), 0);
        assert_eq!(spy.reads.load(Ordering::SeqCst), 0);
    }
    Ok(())
}

#[test]
fn first_capture_must_fit_roster_region_and_projected_entity_limits() -> Result<()> {
    for case in 0..3 {
        let spy = Spy::default();
        let mut limits = limits();
        let mut budget = budget();
        match case {
            0 => limits.citizens = 1,
            1 => limits.spatial.region.origin[0] = 1,
            _ => budget.max_entities = 1,
        }
        assert!(
            matches!(run(&spy,limits,budget,&[0,0,0,0]),Err(e)if e.code==ErrorCode::BudgetExceeded)
        );
        assert_eq!(spy.reads.load(Ordering::SeqCst), 1);
        assert_eq!(spy.drops.load(Ordering::SeqCst), 1);
    }
    Ok(())
}
