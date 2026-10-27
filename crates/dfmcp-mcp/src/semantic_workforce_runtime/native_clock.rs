//! Optional run/1.13 source authority is issued independently of labor authority.
//! This profile does not encode a fortress lineage. Never reinterpret a named
//! fortress grant as native clock authority or use a clock receipt as goal proof.
use super::{
    Arc, CLOCK_TOKEN, Config, Deserialize, MIB, Mode, Work, denied, digest, invalid, nonce, token,
};
use dfmcp_adapter::bounded_run::journal::{RunBinding, RunJournal, RunMode};
use dfmcp_adapter::bounded_run::private_file::{PrivateRunFile, open_private_run_journal_exact};
use dfmcp_adapter::bounded_run::rpc::{RunManifest, RunRpcClient, RunSource, RunTcpStream};
use dfmcp_adapter::bounded_run::{RunObservation, RunPlan, RunRecord};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, FortressId, ObservationCursor,
    OperationContext, Result, RiskTier, StateAnchor,
};
use std::net::SocketAddr;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClockInput {
    generation: u64,
    #[serde(default, deserialize_with = "journal_identity")]
    journal_id: Option<String>,
}
fn journal_identity<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}
#[derive(Clone)]
pub(super) struct ClockConfig {
    generation: u64,
    journal_id: Option<Digest32>,
}
impl ClockConfig {
    pub(super) fn journal_id(&self) -> Option<Digest32> {
        self.journal_id
    }
}
impl ClockInput {
    pub(super) fn validate(self, initialize: bool) -> Result<ClockConfig> {
        if self.generation == 0
            || self.generation == u64::MAX
            || initialize == self.journal_id.is_some()
        {
            return Err(invalid(
                "clock requires an exact source incarnation and an external journal identity only on reopening",
            ));
        }
        Ok(ClockConfig {
            generation: self.generation,
            journal_id: self.journal_id.as_deref().map(digest).transpose()?,
        })
    }
}

/// This is an independent operator-authority issuer, not a grant conversion.
/// The named-fortress request still needs its own Plan/ControlClock permission;
/// the native owner receives only source-scoped grants under this separate gate.
pub(super) fn context(
    config: &Config,
    canonical: &OperationContext,
    control: bool,
) -> Result<OperationContext> {
    config.current()?;
    if config.clock.is_none() || canonical.anchor.fortress_id != config.sources.fortress() {
        return Err(denied(
            "source clock authority requires the configured original fortress context",
        ));
    }
    canonical.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if control {
        if !Config::clock_enabled() {
            return Err(denied("independent operator clock authority is disabled"));
        }
        canonical.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
    }
    issue_context(canonical, control)
}
fn issue_context(canonical: &OperationContext, control: bool) -> Result<OperationContext> {
    canonical.budget.validate()?;
    let mut capabilities = vec![Capability::Query];
    if control {
        capabilities.extend([Capability::Plan, Capability::ControlClock]);
    }
    Ok(OperationContext {
        session_id: canonical.session_id,
        request_id: canonical.request_id,
        anchor: StateAnchor {
            fortress_id: FortressId::NIL,
            cursor: ObservationCursor::ORIGIN,
            tick: canonical.anchor.tick,
            state_hash: Digest32::ZERO,
        },
        budget: canonical.budget,
        grants: capabilities
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(FortressId::NIL),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::Guarded,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        cancellation_requested: canonical.cancellation_requested,
    })
}
fn identity(config: &Config, manifest: &RunManifest, endpoint: Option<SocketAddr>) -> Result<()> {
    let clock = config
        .clock
        .as_ref()
        .ok_or_else(|| denied("operator clock source is not configured"))?;
    if manifest.generation != clock.generation
        || manifest.df_version != config.sources.df_version
        || manifest.dfhack_version != config.sources.dfhack_version
        || endpoint != Some(config.endpoint)
    {
        return Err(denied(
            "clock source differs from its exact operator endpoint, versions or incarnation",
        ));
    }
    Ok(())
}
pub struct PinnedClock {
    inner: RunRpcClient<RunTcpStream>,
    config: Arc<Config>,
    fenced: bool,
}
impl PinnedClock {
    fn check(&mut self, c: &OperationContext, control: bool) -> Result<()> {
        if self.fenced {
            return Err(denied(
                "clock source is fenced after an earlier boundary failure",
            ));
        }
        let result = (|| {
            self.config.current()?;
            identity(&self.config, self.inner.manifest(), self.inner.endpoint())?;
            if c.anchor.fortress_id != FortressId::NIL {
                return Err(denied("native clock requires independent source authority"));
            }
            c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
            if control {
                if !Config::clock_enabled() {
                    return Err(denied("operator clock authority was revoked"));
                }
                c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
            }
            self.inner.restrict_deadline(c)
        })();
        if result.is_err() {
            self.fenced = true;
        }
        result
    }
}
impl RunSource for PinnedClock {
    fn manifest(&self) -> &RunManifest {
        self.inner.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.inner.endpoint()
    }
    fn observe(&mut self, c: &OperationContext) -> Result<RunObservation> {
        self.check(c, false)?;
        let value = self.inner.observe(c)?;
        self.check(c, false)?;
        Ok(value)
    }
    fn prepare(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.check(c, true)?;
        let value = self.inner.prepare(p, c)?;
        self.check(c, false)?;
        Ok(value)
    }
    fn commit(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.check(c, true)?;
        let value = self.inner.commit(p, c)?;
        self.check(c, false)?;
        Ok(value)
    }
    fn query(&mut self, p: &RunPlan, c: &OperationContext) -> Result<Option<RunRecord>> {
        self.check(c, false)?;
        let value = self.inner.query(p, c)?;
        self.check(c, false)?;
        Ok(value)
    }
    fn cancel(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.check(c, true)?;
        let value = self.inner.cancel(p, c)?;
        self.check(c, false)?;
        Ok(value)
    }
}
pub(super) fn connect(config: &Arc<Config>, c: &OperationContext) -> Result<PinnedClock> {
    config.current()?;
    if config.clock.is_none() {
        return Err(denied("operator clock source is not configured"));
    }
    let inner = RunRpcClient::connect(
        config.endpoint,
        token(CLOCK_TOKEN)?,
        nonce(c, b"labor-clock"),
        c,
    )?;
    let mut source = PinnedClock {
        inner,
        config: Arc::clone(config),
        fenced: false,
    };
    source.check(c, false)?;
    Ok(source)
}
pub(super) fn open(
    config: &Arc<Config>,
    mode: Mode,
    canonical: &OperationContext,
) -> Result<Option<RunJournal<PrivateRunFile>>> {
    config.current()?;
    let Some(clock) = &config.clock else {
        match std::fs::symlink_metadata(config.path("clock.journal")) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            _ => {
                return Err(denied(
                    "existing or unverifiable clock custody requires its exact configuration and recovery identity",
                ));
            }
        }
    };
    let mode = match mode {
        Mode::Offline => RunMode::Offline,
        Mode::Control if Config::clock_enabled() => RunMode::Control,
        _ => RunMode::Recover,
    };
    if config.initialize && mode != RunMode::Control {
        return Err(denied(
            "clock initialization requires independent operator clock control",
        ));
    }
    let native = context(config, canonical, mode == RunMode::Control)?;
    let mut work = Work::new(&native)?;
    let binding = RunBinding::new(
        config.endpoint,
        RunManifest {
            generation: clock.generation,
            df_version: config.sources.df_version.clone(),
            dfhack_version: config.sources.dfhack_version.clone(),
        },
    )?;
    let inspect = work.reserve(8 * MIB)?;
    let mut journal = open_private_run_journal_exact(
        &config.path("clock.journal"),
        &work.rest()?,
        mode,
        Some(binding.clone()),
        config.initialize,
    )?;
    journal.records(&work.narrow(&inspect)?)?;
    if journal.binding() != &binding || clock.journal_id.is_some_and(|id| id != journal.id()) {
        return Err(denied(
            "clock custody differs from its exact source or externally retained journal identity",
        ));
    }
    config.current()?;
    work.current()?;
    Ok(Some(journal))
}

#[cfg(test)]
#[path = "native_clock_tests.rs"]
mod tests;
