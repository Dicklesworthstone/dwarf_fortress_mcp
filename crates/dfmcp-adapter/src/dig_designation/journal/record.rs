//! Closed canonical history for the dig coordinator. No retained authority.
use std::net::SocketAddr;

use dfmcp_core::{
    Capability, Digest32, ErrorCode, FortressId, GameTick, MapCoord, MapCuboid, OperationContext,
    Result, RiskTier,
};

use super::super::{
    DigEffect, DigObservation, DigPhase, DigPlan, MAX_EFFECT_BYTES, MAX_PLAN_BYTES, Reader,
    append_text, error, require,
    rpc::{DigManifest, DigSource},
    text,
};

pub const MAX_BINDING_BYTES: usize = 940;
pub const MAX_BODY_BYTES: usize = MAX_PLAN_BYTES + MAX_EFFECT_BYTES + 9;

pub(super) fn check(ok: bool) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(error(
            ErrorCode::CorruptLedger,
            "invalid dig coordinator history",
        ))
    }
}

/// One named fortress, existing native incarnation/software and operator-selected scope.
/// The containing runtime must prevent bypass by choosing a different journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigBinding {
    endpoint: SocketAddr,
    manifest: DigManifest,
    folder: String,
    site: u32,
    scope: MapCuboid,
}
impl DigBinding {
    pub fn new(
        endpoint: SocketAddr,
        manifest: DigManifest,
        before: &DigObservation,
        scope: MapCuboid,
    ) -> Result<Self> {
        let binding = Self {
            endpoint,
            manifest,
            folder: before.folder().to_owned(),
            site: before.site(),
            scope,
        };
        binding.validate()?;
        binding.capture(before)?;
        Ok(binding)
    }
    fn validate(&self) -> Result<()> {
        require(
            self.endpoint.ip().is_loopback()
                && self.endpoint.port() != 0
                && self.endpoint.to_string().len() <= 128,
            "dig journal requires numeric loopback",
        )?;
        require(
            self.manifest.generation > 0
                && self.manifest.generation < u64::MAX
                && self.site <= i32::MAX as u32,
            "invalid dig journal source identity",
        )?;
        for (value, limit) in [
            (&self.folder, 512),
            (&self.manifest.df_version, 128),
            (&self.manifest.dfhack_version, 128),
        ] {
            require(
                !value.is_empty() && value.len() <= limit && !value.contains('\0'),
                "invalid dig journal source text",
            )?;
        }
        check(
            self.scope.min.x >= 0
                && self.scope.min.y >= 0
                && self.scope.min.z >= 0
                && self.scope.max.x <= 32767
                && self.scope.max.y <= 32767
                && self.scope.max.z <= 32767,
        )?;
        MapCuboid::new(self.scope.min, self.scope.max)?;
        Ok(())
    }
    pub(super) fn authorize(&self, context: &OperationContext, tick: u64) -> Result<()> {
        if context.anchor.fortress_id != self.fortress_id() {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "dig journal belongs to another fortress",
            ));
        }
        let mut current = context.clone();
        current.anchor.tick = GameTick(tick.max(context.anchor.tick.get()));
        current.authorize(Capability::Query, RiskTier::ReadOnly, &[], Some(self.scope))
    }
    pub fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }
    pub fn manifest(&self) -> &DigManifest {
        &self.manifest
    }
    pub fn folder(&self) -> &str {
        &self.folder
    }
    pub fn site(&self) -> u32 {
        self.site
    }
    pub fn scope(&self) -> MapCuboid {
        self.scope
    }
    pub fn fortress_id(&self) -> FortressId {
        crate::workforce_control::fortress_id(&self.folder, self.site)
    }
    pub(super) fn capture(&self, value: &DigObservation) -> Result<()> {
        require(
            value.generation() == self.manifest.generation
                && value.folder() == self.folder
                && value.site() == self.site
                && self.scope.contains_cuboid(value.region().halo())
                && self.scope.contains_cuboid(value.region().write_area()),
            "dig observation differs from the retained source and region",
        )
    }
    pub(super) fn source<N: DigSource>(&self, source: &N) -> Result<()> {
        require(
            source.endpoint() == Some(self.endpoint) && source.manifest() == &self.manifest,
            "dig source endpoint, software or incarnation changed",
        )
    }
    pub(super) fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        append_text(&mut out, &self.endpoint.to_string());
        out.extend_from_slice(&self.manifest.generation.to_be_bytes());
        append_text(&mut out, &self.manifest.df_version);
        append_text(&mut out, &self.manifest.dfhack_version);
        append_text(&mut out, &self.folder);
        out.extend_from_slice(&self.site.to_be_bytes());
        for point in [self.scope.min, self.scope.max] {
            for value in [point.x, point.y, point.z] {
                out.extend_from_slice(&value.to_be_bytes());
            }
        }
        out
    }
    pub(super) fn decode(raw: &[u8]) -> Result<Self> {
        check(raw.len() <= MAX_BINDING_BYTES)?;
        let mut r = Reader(raw);
        let address = text(&mut r, 128)?;
        let endpoint: SocketAddr = address
            .parse()
            .map_err(|_| error(ErrorCode::CorruptLedger, "invalid dig journal endpoint"))?;
        check(endpoint.to_string() == address)?;
        let generation = r.u64()?;
        let df_version = text(&mut r, 128)?;
        let dfhack_version = text(&mut r, 128)?;
        let folder = text(&mut r, 512)?;
        let site = r.u32()?;
        let min = MapCoord::new(r.u32()? as i32, r.u32()? as i32, r.u32()? as i32);
        let max = MapCoord::new(r.u32()? as i32, r.u32()? as i32, r.u32()? as i32);
        let scope = MapCuboid::new(min, max)?;
        r.finish()?;
        let value = Self {
            endpoint,
            manifest: DigManifest {
                generation,
                df_version,
                dfhack_version,
            },
            folder,
            site,
            scope,
        };
        value.validate()?;
        Ok(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DigState {
    Intent = 0,
    Prepared = 1,
    DispatchStarted = 2,
    Tracking = 3,
    CancelRequested = 4,
    Terminal = 5,
}
impl DigState {
    pub fn terminal(self) -> bool {
        self == Self::Terminal
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Intent => "intent",
            Self::Prepared => "prepared",
            Self::DispatchStarted => "dispatch_started",
            Self::Tracking => "tracking",
            Self::CancelRequested => "cancel_requested",
            Self::Terminal => "terminal",
        }
    }
    fn decode(tag: u8) -> Result<Self> {
        match tag {
            0 => Ok(Self::Intent),
            1 => Ok(Self::Prepared),
            2 => Ok(Self::DispatchStarted),
            3 => Ok(Self::Tracking),
            4 => Ok(Self::CancelRequested),
            5 => Ok(Self::Terminal),
            _ => Err(error(
                ErrorCode::CorruptLedger,
                "unknown dig coordinator state",
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigRecord {
    pub(super) plan: DigPlan,
    pub(super) state: DigState,
    pub(super) effect: Option<DigEffect>,
}
impl DigRecord {
    pub fn plan(&self) -> &DigPlan {
        &self.plan
    }
    pub fn state(&self) -> DigState {
        self.state
    }
    pub fn effect(&self) -> Option<&DigEffect> {
        self.effect.as_ref()
    }
    pub fn permanent_unknown(&self) -> bool {
        self.effect
            .as_ref()
            .is_some_and(|e| e.phase() == DigPhase::Unknown)
    }
    pub fn needs_reconciliation(&self) -> bool {
        !self.state.terminal() && !self.permanent_unknown()
    }
    pub(super) fn encode(&self) -> Vec<u8> {
        let plan = self.plan.canonical_bytes();
        let effect = self
            .effect
            .as_ref()
            .map_or(&[][..], DigEffect::canonical_bytes);
        let mut out = vec![self.state as u8];
        out.extend_from_slice(&(plan.len() as u32).to_be_bytes());
        out.extend_from_slice(&plan);
        out.extend_from_slice(&(effect.len() as u32).to_be_bytes());
        out.extend_from_slice(effect);
        out
    }
    pub(super) fn decode(raw: &[u8], binding: &DigBinding) -> Result<Self> {
        check(raw.len() <= MAX_BODY_BYTES)?;
        let mut r = Reader(raw);
        let state = DigState::decode(r.byte()?)?;
        let n = r.u32()? as usize;
        check(n <= MAX_PLAN_BYTES)?;
        let plan = DigPlan::decode(r.take(n)?)?;
        binding.capture(plan.before())?;
        let n = r.u32()? as usize;
        check(n <= MAX_EFFECT_BYTES)?;
        let effect = if n == 0 {
            None
        } else {
            Some(DigEffect::decode(r.take(n)?, &plan)?)
        };
        r.finish()?;
        let phase = effect.as_ref().map(DigEffect::phase);
        check(match state {
            DigState::Intent => phase.is_none(),
            DigState::Prepared | DigState::DispatchStarted => phase == Some(DigPhase::Prepared),
            DigState::Tracking => matches!(phase, Some(DigPhase::Prepared | DigPhase::Unknown)),
            DigState::CancelRequested => {
                phase.is_none() || matches!(phase, Some(DigPhase::Prepared | DigPhase::Unknown))
            }
            DigState::Terminal => phase.is_some_and(DigPhase::terminal),
        })?;
        Ok(Self {
            plan,
            state,
            effect,
        })
    }
}

pub(super) fn transition(old: Option<&DigRecord>, next: &DigRecord) -> Result<()> {
    use DigState::*;
    let Some(old) = old else {
        return check(next.state == Intent && next.effect.is_none());
    };
    check(
        next.plan == old.plan && next != old && !old.state.terminal() && !old.permanent_unknown(),
    )?;
    if old.effect.is_some() {
        check(next.effect.is_some())?;
    }
    check(match (old.state, next.state) {
        (Intent, Prepared | Tracking | Terminal) => true,
        (Intent | Prepared | DispatchStarted | Tracking, CancelRequested) => {
            next.effect == old.effect
        }
        (Prepared, DispatchStarted) => next.effect == old.effect,
        (Prepared | DispatchStarted | Tracking, Tracking | Terminal) => true,
        (CancelRequested, CancelRequested | Terminal) => true,
        _ => false,
    })
}

/// Remaining worst-case transitions, including cancellation/reconciliation.
pub(super) fn reserve(record: &DigRecord) -> usize {
    if record.permanent_unknown() {
        return 0;
    }
    match record.state {
        DigState::Intent => 6,
        DigState::Prepared => 5,
        DigState::DispatchStarted => 4,
        DigState::Tracking => 3,
        DigState::CancelRequested => 2,
        DigState::Terminal => 0,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigSummary {
    pub key: String,
    pub plan_digest: Digest32,
    pub state: DigState,
    pub native_phase: Option<DigPhase>,
    pub receipt: Option<Digest32>,
    /// Process-local eligibility only. Never proof that native work will succeed.
    pub dispatchable: bool,
}
