//! Parent-plan custody at each child evidence boundary.
use crate::semantic_workforce_runtime::{denied, exhausted, identity::CapsuleStore};
use dfmcp_adapter::build_placement::journal::private_file::PrivateBuildFile;
use dfmcp_adapter::live_projection::project_live_capsule;
use dfmcp_adapter::live_routing::LiveRoutingEvidence;
use dfmcp_adapter::plan_archive::PlanArchiveStore;
use dfmcp_adapter::semantic_work_orders::WorkOrderEvidenceOwner;
use dfmcp_adapter::semantic_workforce::WorkforceEvidenceOwner;
use dfmcp_adapter::work_orders::WorkOrderObservation;
use dfmcp_adapter::workforce_control::WorkforceCapture;
use dfmcp_core::{OperationContext, Result};
use dfmcp_intent::PreparedPlan;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ARCHIVE_BYTES: u64 = 4 * 1024 * 1024;
const IDENTITY_BYTES: u64 = 32 * 1024 * 1024;

struct Armed {
    context: OperationContext,
    until: Instant,
    bytes: u64,
}
struct Custody {
    archive: PlanArchiveStore<PrivateBuildFile>,
    identity: CapsuleStore<PrivateBuildFile>,
    original: PreparedPlan,
    armed: Option<Armed>,
    generation: u64,
}

#[derive(Clone)]
pub(super) struct SharedCustody(Arc<Mutex<Custody>>);
impl SharedCustody {
    pub(super) fn new(
        archive: PlanArchiveStore<PrivateBuildFile>,
        identity: CapsuleStore<PrivateBuildFile>,
        original: PreparedPlan,
    ) -> Self {
        Self(Arc::new(Mutex::new(Custody {
            archive,
            identity,
            original,
            armed: None,
            generation: 0,
        })))
    }
    /// The caller has already reserved this allowance from the public operation.
    pub(super) fn arm(&self, c: &OperationContext) -> Result<()> {
        c.budget.validate()?;
        if c.cancellation_requested {
            return Err(denied("custody operation cancelled"));
        }
        let until = Instant::now()
            .checked_add(Duration::from_millis(c.budget.max_wall_millis))
            .ok_or_else(|| exhausted("custody deadline overflow"))?;
        let mut state = self
            .0
            .try_lock()
            .map_err(|_| denied("custody is unavailable"))?;
        if state.armed.is_some() {
            return Err(denied("custody operation already armed"));
        }
        state.generation = state
            .generation
            .checked_add(1)
            .ok_or_else(|| exhausted("custody generation exhausted"))?;
        state.armed = Some(Armed {
            context: c.clone(),
            until,
            bytes: c.budget.max_bytes,
        });
        Ok(())
    }
    pub(super) fn disarm(&self) -> Result<()> {
        let mut state = self
            .0
            .try_lock()
            .map_err(|_| denied("custody is unavailable"))?;
        state.armed = None;
        Ok(())
    }
    pub(super) fn verify(&self, c: &OperationContext) -> Result<()> {
        self.verify_generation(c).map(|_| ())
    }
    fn verify_generation(&self, c: &OperationContext) -> Result<u64> {
        let mut state = self
            .0
            .try_lock()
            .map_err(|_| denied("custody is unavailable"))?;
        // Any new check withdraws previously published routing, including when
        // this verification fails before reaching either private file.
        state.generation = state
            .generation
            .checked_add(1)
            .ok_or_else(|| exhausted("custody generation exhausted"))?;
        let armed = state
            .armed
            .as_mut()
            .ok_or_else(|| denied("custody operation is not armed"))?;
        if c.session_id != armed.context.session_id
            || c.request_id != armed.context.request_id
            || c.cancellation_requested
        {
            return Err(denied("custody operation context changed or was cancelled"));
        }
        c.budget.validate()?;
        let needed = ARCHIVE_BYTES + IDENTITY_BYTES;
        armed.bytes = armed
            .bytes
            .checked_sub(needed)
            .ok_or_else(|| exhausted("custody byte allowance exhausted"))?;
        let incoming_until = Instant::now()
            .checked_add(Duration::from_millis(c.budget.max_wall_millis))
            .ok_or_else(|| exhausted("custody deadline overflow"))?;
        let until = armed.until.min(incoming_until);
        let context = |bytes| -> Result<OperationContext> {
            let remaining = until
                .checked_duration_since(Instant::now())
                .ok_or_else(|| exhausted("custody deadline expired"))?
                .as_millis();
            if remaining == 0 {
                return Err(exhausted("custody deadline expired"));
            }
            let mut current = c.clone();
            current.budget.max_wall_millis =
                u64::try_from(remaining).map_err(|_| exhausted("custody deadline overflow"))?;
            current.budget.max_bytes = bytes;
            Ok(current)
        };
        if state.archive.recover(&context(ARCHIVE_BYTES)?)? != state.original {
            return Err(denied(
                "original plan archive changed at the effect boundary",
            ));
        }
        let capsule = state.identity.recover(&context(IDENTITY_BYTES)?)?;
        let projection = project_live_capsule(
            &capsule,
            state.original.anchor.fortress_id,
            state.original.anchor.cursor,
        )?;
        if projection.snapshot.anchor() != state.original.anchor {
            return Err(denied(
                "original identity archive changed at the effect boundary",
            ));
        }
        context(1)?;
        Ok(state.generation)
    }
    fn narrow(&self, c: &OperationContext) -> Result<OperationContext> {
        let state = self
            .0
            .try_lock()
            .map_err(|_| denied("custody is unavailable"))?;
        let armed = state
            .armed
            .as_ref()
            .ok_or_else(|| denied("custody operation is not armed"))?;
        if c.session_id != armed.context.session_id
            || c.request_id != armed.context.request_id
            || c.cancellation_requested
        {
            return Err(denied("custody operation context changed or was cancelled"));
        }
        let millis = armed
            .until
            .checked_duration_since(Instant::now())
            .ok_or_else(|| exhausted("custody deadline expired"))?
            .as_millis();
        let millis = u64::try_from(millis).map_err(|_| exhausted("custody deadline overflow"))?;
        if millis == 0 {
            return Err(exhausted("custody deadline expired"));
        }
        let mut current = c.clone();
        current.budget.max_wall_millis = current.budget.max_wall_millis.min(millis);
        Ok(current)
    }
    fn route_current(&self, generation: u64) -> Result<()> {
        let state = self
            .0
            .try_lock()
            .map_err(|_| denied("custody is unavailable"))?;
        let armed = state
            .armed
            .as_ref()
            .ok_or_else(|| denied("custody operation is not armed"))?;
        if state.generation != generation || Instant::now() >= armed.until {
            return Err(denied("custody routing evidence is stale"));
        }
        Ok(())
    }
}

/// Validity is withdrawn before every refresh and restored only after both checks.
pub(super) struct GuardedEvidence<E> {
    inner: E,
    custody: SharedCustody,
    valid: Option<u64>,
}
impl<E> GuardedEvidence<E> {
    pub(super) fn new(inner: E, custody: SharedCustody) -> Self {
        Self {
            inner,
            custody,
            valid: None,
        }
    }
    fn check_route(&self) -> Result<()> {
        self.custody.route_current(
            self.valid
                .ok_or_else(|| denied("native routing evidence is not verified"))?,
        )
    }
}
impl GuardedEvidence<crate::semantic_workforce_runtime::native::NativeEvidence> {
    /// Acquire current independently authorized citizen evidence before the
    /// workforce owner resolves semantic unit IDs. The later native-capture
    /// refreshes remain mandatory, including after DispatchStarted.
    pub(super) fn prime(&mut self, c: &OperationContext) -> Result<()> {
        self.valid = None;
        let start = Instant::now();
        self.custody.verify(c)?;
        let current = self.custody.narrow(&remaining_context(c, start)?)?;
        self.inner
            .refresh::<dfmcp_adapter::semantic_workforce::evidence_owner::CitizenRpcSource<
                std::net::TcpStream,
            >>(&current)?;
        let generation = self
            .custody
            .verify_generation(&remaining_context(c, start)?)?;
        self.valid = Some(generation);
        Ok(())
    }
}
impl<E: WorkforceEvidenceOwner> WorkforceEvidenceOwner for GuardedEvidence<E> {
    fn refresh_after_capture(
        &mut self,
        capture: &WorkforceCapture,
        c: &OperationContext,
    ) -> Result<()> {
        self.valid = None;
        let start = Instant::now();
        self.custody.verify(c)?;
        let current = self.custody.narrow(&remaining_context(c, start)?)?;
        self.inner.refresh_after_capture(capture, &current)?;
        let generation = self
            .custody
            .verify_generation(&remaining_context(c, start)?)?;
        self.valid = Some(generation);
        Ok(())
    }
    fn routing_evidence(&self) -> Result<LiveRoutingEvidence<'_>> {
        self.check_route()?;
        self.inner.routing_evidence()
    }
}
impl<E: WorkOrderEvidenceOwner> WorkOrderEvidenceOwner for GuardedEvidence<E> {
    fn refresh_after_observation(
        &mut self,
        observation: &WorkOrderObservation,
        c: &OperationContext,
    ) -> Result<()> {
        self.valid = None;
        let start = Instant::now();
        self.custody.verify(c)?;
        let current = self.custody.narrow(&remaining_context(c, start)?)?;
        self.inner
            .refresh_after_observation(observation, &current)?;
        let generation = self
            .custody
            .verify_generation(&remaining_context(c, start)?)?;
        self.valid = Some(generation);
        Ok(())
    }
    fn routing_evidence(&self) -> Result<LiveRoutingEvidence<'_>> {
        self.check_route()?;
        self.inner.routing_evidence()
    }
}
fn remaining_context(c: &OperationContext, start: Instant) -> Result<OperationContext> {
    let elapsed = u64::try_from(start.elapsed().as_millis())
        .map_err(|_| exhausted("custody deadline overflow"))?;
    let remaining = c
        .budget
        .max_wall_millis
        .checked_sub(elapsed)
        .filter(|v| *v > 0)
        .ok_or_else(|| exhausted("evidence deadline expired"))?;
    let mut current = c.clone();
    current.budget.max_wall_millis = remaining;
    Ok(current)
}

#[cfg(all(
    test,
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[path = "custody_tests.rs"]
mod tests;
