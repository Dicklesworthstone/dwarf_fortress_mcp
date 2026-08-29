use std::collections::BTreeSet;

use crate::{
    DfmcpError, Digest32, EntityId, ErrorCode, EvidenceId, FortressId, RequestId, Result, SessionId,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct GameTick(pub u64);

impl GameTick {
    #[must_use]
    pub const fn saturating_add(self, amount: u64) -> Self {
        Self(self.0.saturating_add(amount))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ObservationCursor {
    pub epoch: u64,
    pub sequence: u64,
}

impl ObservationCursor {
    pub const ORIGIN: Self = Self {
        epoch: 0,
        sequence: 0,
    };

    #[must_use]
    pub const fn next(self) -> Self {
        Self {
            epoch: self.epoch,
            sequence: self.sequence.saturating_add(1),
        }
    }

    #[must_use]
    pub const fn reset_epoch(self) -> Self {
        Self {
            epoch: self.epoch.saturating_add(1),
            sequence: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateAnchor {
    pub fortress_id: FortressId,
    pub cursor: ObservationCursor,
    pub tick: GameTick,
    pub state_hash: Digest32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct MapCoord {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MapCuboid {
    pub min: MapCoord,
    pub max: MapCoord,
}

impl MapCuboid {
    pub fn new(min: MapCoord, max: MapCoord) -> Result<Self> {
        if min.x > max.x || min.y > max.y || min.z > max.z {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "map cuboid minimum must not exceed maximum",
            ));
        }
        Ok(Self { min, max })
    }

    #[must_use]
    pub const fn contains(self, coord: MapCoord) -> bool {
        coord.x >= self.min.x
            && coord.x <= self.max.x
            && coord.y >= self.min.y
            && coord.y <= self.max.y
            && coord.z >= self.min.z
            && coord.z <= self.max.z
    }

    #[must_use]
    pub const fn contains_cuboid(self, other: Self) -> bool {
        self.contains(other.min) && self.contains(other.max)
    }

    #[must_use]
    pub fn tile_count(self) -> Option<u64> {
        let width = i64::from(self.max.x) - i64::from(self.min.x) + 1;
        let height = i64::from(self.max.y) - i64::from(self.min.y) + 1;
        let depth = i64::from(self.max.z) - i64::from(self.min.z) + 1;
        let width = u64::try_from(width).ok()?;
        let height = u64::try_from(height).ok()?;
        let depth = u64::try_from(depth).ok()?;
        width.checked_mul(height)?.checked_mul(depth)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RiskTier {
    ReadOnly,
    Reversible,
    Guarded,
    Irreversible,
}

impl RiskTier {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Reversible => "reversible",
            Self::Guarded => "guarded",
            Self::Irreversible => "irreversible",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    Observe,
    Query,
    Plan,
    Designate,
    Construct,
    ConfigureLabor,
    ConfigureProduction,
    ConfigureLogistics,
    ConfigureMilitary,
    ControlClock,
    Checkpoint,
    Restore,
    Extension,
    DiagnosticRaw,
    Doctor,
    RepairPlan,
    RepairApply,
    Admin,
}

impl Capability {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Query => "query",
            Self::Plan => "plan",
            Self::Designate => "designate",
            Self::Construct => "construct",
            Self::ConfigureLabor => "configure_labor",
            Self::ConfigureProduction => "configure_production",
            Self::ConfigureLogistics => "configure_logistics",
            Self::ConfigureMilitary => "configure_military",
            Self::ControlClock => "control_clock",
            Self::Checkpoint => "checkpoint",
            Self::Restore => "restore",
            Self::Extension => "extension",
            Self::DiagnosticRaw => "diagnostic_raw",
            Self::Doctor => "doctor",
            Self::RepairPlan => "repair_plan",
            Self::RepairApply => "repair_apply",
            Self::Admin => "admin",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CapabilityScope {
    pub fortress_id: Option<FortressId>,
    pub entity_ids: BTreeSet<EntityId>,
    pub map_area: Option<MapCuboid>,
}

impl CapabilityScope {
    #[must_use]
    pub fn permits(
        &self,
        fortress_id: FortressId,
        entity_ids: &[EntityId],
        map_area: Option<MapCuboid>,
    ) -> bool {
        if self
            .fortress_id
            .is_some_and(|allowed| allowed != fortress_id)
        {
            return false;
        }
        if !self.entity_ids.is_empty()
            && (entity_ids.is_empty()
                || entity_ids
                    .iter()
                    .any(|entity_id| !self.entity_ids.contains(entity_id)))
        {
            return false;
        }
        match (self.map_area, map_area) {
            (Some(allowed), Some(requested)) => allowed.contains_cuboid(requested),
            (Some(_), None) => false,
            (None, _) => true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityGrant {
    pub capability: Capability,
    pub scope: CapabilityScope,
    pub max_risk: RiskTier,
    pub expires_at_tick: Option<GameTick>,
    pub remaining_uses: Option<u32>,
}

impl CapabilityGrant {
    #[must_use]
    pub fn allows(
        &self,
        capability: Capability,
        risk: RiskTier,
        tick: GameTick,
        fortress_id: FortressId,
        entity_ids: &[EntityId],
        map_area: Option<MapCuboid>,
    ) -> bool {
        let kind_matches = self.capability == capability || self.capability == Capability::Admin;
        let not_expired = match self.expires_at_tick {
            Some(expires_at) => tick <= expires_at,
            None => true,
        };
        let uses_available = match self.remaining_uses {
            Some(remaining) => remaining > 0,
            None => true,
        };
        kind_matches
            && risk <= self.max_risk
            && not_expired
            && uses_available
            && self.scope.permits(fortress_id, entity_ids, map_area)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkBudget {
    pub max_wall_millis: u64,
    pub max_game_ticks: u64,
    pub max_entities: u32,
    pub max_bytes: u64,
    pub max_output_tokens: u32,
    pub max_actions: u32,
}

impl WorkBudget {
    pub const CONSERVATIVE_DEFAULT: Self = Self {
        max_wall_millis: 2_000,
        max_game_ticks: 10_000,
        max_entities: 2_000,
        max_bytes: 4 * 1024 * 1024,
        max_output_tokens: 1_500,
        max_actions: 64,
    };

    pub fn validate(self) -> Result<()> {
        if self.max_wall_millis == 0
            || self.max_entities == 0
            || self.max_bytes == 0
            || self.max_output_tokens == 0
            || self.max_actions == 0
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "all non-time work-budget dimensions must be nonzero",
            ));
        }
        Ok(())
    }
}

impl Default for WorkBudget {
    fn default() -> Self {
        Self::CONSERVATIVE_DEFAULT
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationContext {
    pub session_id: SessionId,
    pub request_id: RequestId,
    pub anchor: StateAnchor,
    pub budget: WorkBudget,
    pub grants: Vec<CapabilityGrant>,
    pub cancellation_requested: bool,
}

impl OperationContext {
    pub fn authorize(
        &self,
        capability: Capability,
        risk: RiskTier,
        entity_ids: &[EntityId],
        map_area: Option<MapCuboid>,
    ) -> Result<()> {
        if self.cancellation_requested {
            return Err(
                DfmcpError::new(ErrorCode::CancellationRequested, "operation is cancelled")
                    .retryable(false),
            );
        }
        self.budget.validate()?;
        if self.grants.iter().any(|grant| {
            grant.allows(
                capability,
                risk,
                self.anchor.tick,
                self.anchor.fortress_id,
                entity_ids,
                map_area,
            )
        }) {
            return Ok(());
        }
        Err(DfmcpError::new(
            ErrorCode::CapabilityDenied,
            format!(
                "capability {} is not granted for risk tier {} and requested scope",
                capability.as_str(),
                risk.as_str()
            ),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EvidenceKind {
    Observation,
    AdapterReceipt,
    Postcondition,
    Checkpoint,
    Replay,
    Diagnostic,
    HumanConfirmation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evidence {
    pub id: EvidenceId,
    pub kind: EvidenceKind,
    pub subject: Option<EntityId>,
    pub anchor: StateAnchor,
    pub digest: Digest32,
    pub summary: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CommitState {
    Prepared,
    Committing,
    AppliedAwaitingVerification,
    Verified,
    CompensationPending,
    Compensated,
    CancelRequested,
    Cancelled,
    Failed,
    Indeterminate,
}

impl CommitState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Verified | Self::Compensated | Self::Cancelled | Self::Failed
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationOutcome<T> {
    Succeeded(T),
    Failed(DfmcpError),
    Cancelled {
        final_anchor: Option<StateAnchor>,
        reason: String,
    },
    Indeterminate {
        last_anchor: Option<StateAnchor>,
        reason: String,
        reconciliation_required: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::{
        Capability, CapabilityGrant, CapabilityScope, GameTick, MapCoord, MapCuboid, RiskTier,
    };
    use crate::{EntityId, FortressId};

    #[test]
    fn cuboid_scope_is_inclusive() -> crate::Result<()> {
        let outer = MapCuboid::new(
            MapCoord { x: 0, y: 0, z: 0 },
            MapCoord { x: 9, y: 9, z: 2 },
        )?;
        let inner = MapCuboid::new(
            MapCoord { x: 1, y: 1, z: 0 },
            MapCoord { x: 8, y: 8, z: 1 },
        )?;
        assert!(outer.contains_cuboid(inner));
        assert_eq!(outer.tile_count(), Some(300));
        Ok(())
    }


    #[test]
    fn restricted_scope_rejects_an_unscoped_request() -> crate::Result<()> {
        let area = MapCuboid::new(
            MapCoord { x: 0, y: 0, z: 0 },
            MapCoord { x: 9, y: 9, z: 0 },
        )?;
        let mut scope = CapabilityScope {
            fortress_id: Some(FortressId::new(7)),
            map_area: Some(area),
            ..CapabilityScope::default()
        };
        scope.entity_ids.insert(EntityId::new(11));
        assert!(!scope.permits(FortressId::new(7), &[], None));
        assert!(!scope.permits(FortressId::new(7), &[EntityId::new(11)], None));
        assert!(scope.permits(
            FortressId::new(7),
            &[EntityId::new(11)],
            Some(area),
        ));
        Ok(())
    }

    #[test]
    fn grant_enforces_scope_risk_and_expiry() {
        let mut scope = CapabilityScope {
            fortress_id: Some(FortressId::new(7)),
            ..CapabilityScope::default()
        };
        scope.entity_ids.insert(EntityId::new(11));
        let grant = CapabilityGrant {
            capability: Capability::ConfigureLabor,
            scope,
            max_risk: RiskTier::Reversible,
            expires_at_tick: Some(GameTick(100)),
            remaining_uses: Some(1),
        };
        assert!(grant.allows(
            Capability::ConfigureLabor,
            RiskTier::Reversible,
            GameTick(99),
            FortressId::new(7),
            &[EntityId::new(11)],
            None,
        ));
        assert!(!grant.allows(
            Capability::ConfigureLabor,
            RiskTier::Guarded,
            GameTick(99),
            FortressId::new(7),
            &[EntityId::new(11)],
            None,
        ));
    }
}
