//! Connection negotiation and the first complete capture consume one allowance.
//! This is the unadmitted runtime's source bootstrap, not production admission.
use super::*;
use std::net::SocketAddr;

pub(in super::super) fn connect_and_capture(endpoint: SocketAddr, token: Vec<u8>, nonce: Vec<u8>,
    limits: CitizenSpatialLimits, budget: WorkBudget)
    -> Result<(CitizenSpatialRpcClient<DeadlineStream>, LiveSpatialCitizenState)> {
    let started = Instant::now();
    establish(limits, budget,
        |remaining| CitizenSpatialRpcClient::connect(endpoint, token, nonce, remaining, limits),
        || started.elapsed())
}

pub(super) fn allowance(budget: WorkBudget, elapsed: Duration) -> Result<Duration> {
    budget.validate()?;
    Duration::from_millis(budget.max_wall_millis)
        .checked_sub(elapsed)
        .filter(|remaining| *remaining >= Duration::from_millis(1))
        .ok_or_else(|| error(ErrorCode::BudgetExceeded,
            "spatial source operation exhausted its shared connection/capture/validation deadline"))
}

pub(super) fn establish<S: Source>(limits: CitizenSpatialLimits, budget: WorkBudget,
    connect: impl FnOnce(Duration) -> Result<S>, mut elapsed: impl FnMut() -> Duration)
    -> Result<(S, LiveSpatialCitizenState)> {
    budget.validate()?;
    limits.validate()?;
    let mut source = connect(allowance(budget, elapsed())?)?;
    let observation = source.read(allowance(budget, elapsed())?)?;
    allowance(budget, elapsed())?;
    check_bounds(&observation, limits)?;
    let mut state = LiveSpatialCitizenState::default();
    state.publish(observation)?;
    let snapshot = state.snapshot().ok_or_else(|| error(ErrorCode::InternalInvariantViolation,
        "spatial bootstrap did not produce a complete canonical snapshot"))?;
    if snapshot.graph.entities.len() > budget.max_entities as usize {
        return Err(error(ErrorCode::BudgetExceeded,
            "spatial bootstrap projection exceeds the negotiated entity allowance"));
    }
    allowance(budget, elapsed())?;
    // Every failure above drops the unpublished source. Neither a session entry
    // nor an observation/watch journal has been created by this helper.
    Ok((source, state))
}
