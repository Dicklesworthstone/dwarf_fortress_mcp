//! Versioned ordinary blueprint intake and exact retained-source reconstruction.
//! Unversioned stored sources keep their original excavation-only goal and
//! deadline policy; new requests always seal the complete furnishing compiler.

use dfmcp_core::{DfmcpError, ErrorCode, IntentId, Result};
use dfmcp_intent::{BlueprintPlanner, Intent};
use dfmcp_world::WorldSnapshot;
use serde::{Deserialize, Deserializer, Serialize};

pub(super) const CURRENT_PLANNER: &str = "furnished_blueprint_v2";

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "template", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    BedroomCluster {
        origin: [i32; 3],
        rooms: u32,
        room_size: [u8; 2],
        #[serde(
            default,
            deserialize_with = "present",
            skip_serializing_if = "Option::is_none"
        )]
        planner: Option<String>,
    },
    DiningHall {
        origin: [i32; 3],
        width: u8,
        height: u8,
        #[serde(
            default,
            deserialize_with = "present",
            skip_serializing_if = "Option::is_none"
        )]
        planner: Option<String>,
    },
    WorkshopHub {
        origin: [i32; 3],
        bays: u32,
        #[serde(
            default,
            deserialize_with = "present",
            skip_serializing_if = "Option::is_none"
        )]
        planner: Option<String>,
    },
    StockpileVault {
        origin: [i32; 3],
        width: u8,
        height: u8,
        category: String,
        #[serde(
            default,
            deserialize_with = "present",
            skip_serializing_if = "Option::is_none"
        )]
        planner: Option<String>,
    },
}

fn present<'de, D>(deserializer: D) -> std::result::Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer).map(Some)
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

impl Request {
    fn planner(&self) -> &Option<String> {
        match self {
            Self::BedroomCluster { planner, .. }
            | Self::DiningHall { planner, .. }
            | Self::WorkshopHub { planner, .. }
            | Self::StockpileVault { planner, .. } => planner,
        }
    }

    fn planner_mut(&mut self) -> &mut Option<String> {
        match self {
            Self::BedroomCluster { planner, .. }
            | Self::DiningHall { planner, .. }
            | Self::WorkshopHub { planner, .. }
            | Self::StockpileVault { planner, .. } => planner,
        }
    }

    fn json(&self) -> Result<String> {
        let encoded = serde_json::to_string(self)
            .map_err(|error| invalid(format!("cannot encode blueprint source: {error}")))?;
        if encoded.len() > crate::lab_world::MAX_QUERY_JSON_BYTES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "canonical blueprint source exceeds its byte bound",
            ));
        }
        Ok(encoded)
    }

    fn geometry(&self) -> Result<(dfmcp_core::MapCoord, dfmcp_intent::BlueprintTemplate)> {
        // The original closed template parser remains the geometry authority.
        // Deserialize the caller's bytes into Request first, so this normalized
        // representation cannot hide duplicate fields or a null planner marker.
        let mut geometry = self.clone();
        *geometry.planner_mut() = None;
        crate::lab_world::parse_blueprint(&geometry.json()?)
    }
}

fn parse(raw: &str) -> Result<Request> {
    if raw.len() > crate::lab_world::MAX_QUERY_JSON_BYTES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "blueprint request exceeds its byte bound",
        ));
    }
    serde_json::from_str(raw)
        .map_err(|error| invalid(format!("invalid ordinary blueprint source: {error}")))
}

/// A caller can repeat the current marker, but cannot opt into historical
/// behavior or an unknown compiler. The marker survives normal durable custody.
pub(super) fn canonical_current(raw: &str) -> Result<String> {
    let mut request = parse(raw)?;
    if request
        .planner()
        .as_deref()
        .is_some_and(|planner| planner != CURRENT_PLANNER)
    {
        return Err(invalid(
            "ordinary blueprint requires furnished_blueprint_v2",
        ));
    }
    request.geometry()?;
    *request.planner_mut() = Some(CURRENT_PLANNER.to_owned());
    request.json()
}

/// Compile a retained source without replacing the semantics it originally
/// reviewed. Current intake must call canonical_current before retaining it.
pub(super) fn compile(id: IntentId, snapshot: &WorldSnapshot, raw: &str) -> Result<Intent> {
    let request = parse(raw)?;
    if request
        .planner()
        .as_deref()
        .is_some_and(|planner| planner != CURRENT_PLANNER)
    {
        return Err(invalid("stored blueprint names an unsupported compiler"));
    }
    let (origin, template) = request.geometry()?;
    let index = crate::lab_world::spatial_index(snapshot)?;
    if request.planner().is_none() {
        BlueprintPlanner.compile_furnished_blueprint_intent_legacy(
            id,
            snapshot.anchor(),
            origin,
            template,
            &index,
        )
    } else {
        BlueprintPlanner.compile_furnished_blueprint_intent(
            id,
            snapshot.anchor(),
            origin,
            template,
            &index,
        )
    }
}

#[cfg(test)]
#[path = "blueprint_source_tests.rs"]
mod tests;
