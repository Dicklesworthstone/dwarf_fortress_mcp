//! Runtime-owned shell for the independently configured joint room clock.
use super::{error, exhausted, room_clock, room_monitor, runtime};
use dfmcp_adapter::bounded_run::journal::RunMode;
use dfmcp_adapter::bounded_run::private_file::{PrivateRunFile, open_private_run_journal_exact};
use dfmcp_adapter::build_placement::journal::BuildMode;
use dfmcp_adapter::room_condition::clock::{Clock, Recovery};
use dfmcp_adapter::room_condition::{Definition, Sample, SourceIdentity};
use dfmcp_core::{Capability, ErrorCode, OperationContext, Result, RiskTier};
use std::time::{Duration, Instant};
pub(super) struct Io<'a> {
    pub control: &'a runtime::RequestControl,
    pub config: &'a runtime::Config,
    pub opening_recovery: Option<Recovery>,
    pub observed_floor: u64,
}
#[allow(clippy::too_many_arguments)]
pub(super) fn open(
    control: &runtime::RequestControl,
    config: &runtime::Config,
    selected: &room_clock::Config,
    definition: &Definition,
    source: &SourceIdentity,
    c: &OperationContext,
    n: &OperationContext,
    recovery: &mut Option<Recovery>,
) -> Result<Clock<PrivateRunFile>> {
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(
            c.budget.max_wall_millis.min(n.budget.max_wall_millis),
        ))
        .ok_or_else(exhausted)?;
    if c.budget.max_bytes.min(n.budget.max_bytes) < room_clock::OPEN_BYTES {
        return Err(exhausted());
    }
    let remaining = || {
        deadline
            .checked_duration_since(Instant::now())
            .and_then(|left| u64::try_from(left.as_millis()).ok())
            .filter(|left| *left > 0)
            .ok_or_else(exhausted)
    };
    if config.room_clock.as_ref() != Some(selected) || config.clock_only || config.completion_only {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "original room clock configuration differs",
        ));
    }
    runtime::selected_clock_boundary(control, config, selected.initialize, true)?;
    selected.matches(definition.original())?;
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let mode = match config.mode {
        BuildMode::Offline => RunMode::Offline,
        BuildMode::Control if runtime::room_clock_enabled()? => RunMode::Control,
        _ => RunMode::Recover,
    };
    if selected.initialize && mode != RunMode::Control {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "room clock creation requires explicit retained clock control",
        ));
    }
    *recovery = if selected.initialize {
        let id = selected
            .binding
            .initial_journal_id(n.session_id, n.request_id);
        Some(Recovery {
            journal_id: id,
            head: id,
        })
    } else {
        selected.recovery
    };
    let mut opening = n.clone();
    opening.budget.max_bytes = room_clock::VIEW_BYTES;
    opening.budget.max_wall_millis = remaining()?;
    let journal = open_private_run_journal_exact(
        &selected.path,
        &opening,
        mode,
        Some(selected.binding.clone()),
        selected.initialize,
    )?;
    runtime::selected_clock_boundary(control, config, selected.initialize, true)?;
    let mut canonical = c.clone();
    let mut native = n.clone();
    let wall = remaining()?;
    canonical.budget.max_bytes = 2 * room_clock::VIEW_BYTES;
    native.budget.max_bytes = canonical.budget.max_bytes;
    canonical.budget.max_wall_millis = wall;
    native.budget.max_wall_millis = wall;
    let owner = Clock::new(
        journal,
        definition.clone(),
        source.clone(),
        selected.recovery,
        &canonical,
        &native,
    )?;
    *recovery = Some(owner.recovery());
    Ok(owner)
}
impl<'a> room_clock::Io for Io<'a> {
    type Source = runtime::ClockSource<'a>;
    fn opening_recovery(&self) -> Option<Recovery> {
        self.opening_recovery
    }
    fn observed_tick_floor(&self) -> u64 {
        self.observed_floor
    }
    fn open(
        &mut self,
        selected: &room_clock::Config,
        definition: &Definition,
        source: &SourceIdentity,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<Clock<PrivateRunFile>> {
        open(
            self.control,
            self.config,
            selected,
            definition,
            source,
            c,
            n,
            &mut self.opening_recovery,
        )
    }
    fn connect(
        &mut self,
        selected: &room_clock::Config,
        n: &OperationContext,
        control: bool,
    ) -> Result<Self::Source> {
        runtime::connect_clock(self.control, self.config, selected, n, control, true)
    }
    fn sample(&mut self, definition: &Definition, c: &OperationContext) -> Result<Sample> {
        let mut reader = runtime::RoomIo {
            control: self.control,
            config: self.config,
            observed_floor: self.observed_floor,
        };
        let outcome = room_monitor::Io::sample(
            &mut reader,
            definition.origin().definition().binding(),
            definition,
            c,
        );
        self.observed_floor = self.observed_floor.max(reader.observed_floor);
        outcome
    }
    fn checkpoint(&mut self, c: &OperationContext, control: bool) -> Result<()> {
        c.budget.validate()?;
        if c.cancellation_requested {
            return Err(error(
                ErrorCode::CancellationRequested,
                "room clock request cancelled",
            ));
        }
        runtime::selected_clock_boundary(self.control, self.config, control, true)
    }
}
