//! Optional, explicit watch proposals over the exact blueprint mask. Producing
//! this request neither registers a watch nor executes or proves excavation.
use super::{BlueprintLayout, DigMode, OperationContext, Result, area_json, invalid};
use dfmcp_core::{Capability, Digest32, RiskTier};
use serde::Deserialize;
use serde_json::{Value, json};

const POLICY: &str = "dfmcp.blueprint-shape-monitor/1";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Options {
    key: String,
    deadline_tick: u64,
    poll_interval_ticks: Option<u64>,
    stable_observations: Option<u32>,
}

pub(super) fn proposal(
    layout: &BlueprintLayout,
    dimensions: [u32; 3],
    context: &OperationContext,
    options: Options,
) -> Result<Value> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let cadence = options.poll_interval_ticks.unwrap_or(1);
    let stability = options.stable_observations.unwrap_or(2);
    if options.key.is_empty() || options.key.len() > 64 || options.key.contains('\0') {
        return Err(invalid(
            "blueprint monitor key must contain 1..64 UTF-8 bytes without NUL",
        ));
    }
    if !(1..=1_000_000).contains(&cadence) || !(1..=64).contains(&stability) {
        return Err(invalid(
            "blueprint monitor cadence must be 1..1000000 and stability 1..64",
        ));
    }
    if options.deadline_tick <= context.anchor.tick.0
        || options
            .deadline_tick
            .checked_sub(context.anchor.tick.0)
            .is_none_or(|ticks| ticks > context.budget.max_game_ticks)
    {
        return Err(invalid(
            "blueprint monitor deadline must be future and within the negotiated game-tick horizon",
        ));
    }
    if dimensions.iter().any(|size| *size == 0 || *size > 32_768) {
        return Err(invalid(
            "blueprint monitor requires established map dimensions",
        ));
    }
    let first = layout
        .excavations()
        .first()
        .ok_or_else(|| invalid("blueprint monitor mask is empty"))?;
    let mut areas = Vec::with_capacity(layout.excavations().len());
    for part in layout.excavations() {
        if part.mode != first.mode {
            return Err(invalid(
                "mixed excavation modes need separate explicit shape watches",
            ));
        }
        for point in [part.area.min, part.area.max] {
            for (coordinate, size) in [point.x, point.y, point.z].into_iter().zip(dimensions) {
                if coordinate < 0 || coordinate as u32 >= size {
                    return Err(invalid(
                        "blueprint monitor footprint extends outside the map; no coordinates were dropped or clamped",
                    ));
                }
            }
        }
        areas.push(area_json(part.area));
    }
    // These are declared shape goals, not a native dig-mode completion registry.
    let targets: &[&str] = match first.mode {
        DigMode::Mine => &["floor"],
        DigMode::Channel => &["empty", "ramp_top"],
        _ => {
            return Err(invalid(
                "blueprint excavation mode has no declared shape-monitor policy",
            ));
        }
    };
    let fields: Vec<Value> = targets
        .iter()
        .map(|shape| {
            json!({"op":"field",
        "field":"shape","comparison":"eq","value":{"type":"text","value":shape}})
        })
        .collect();
    let predicate = if fields.len() == 1 {
        fields[0].clone()
    } else {
        json!({"op":"any","args":fields})
    };
    let condition = json!({"op":"terrain_count","areas":areas,"predicate":predicate,
        "comparison":"eq","value":layout.tile_count()});
    let request = json!({"schema":"dfmcp.query/1","expected_anchor":super::super::anchor_json(context.anchor),
        "query":{"kind":"watch","key":options.key,"label":layout.summary(),"condition":condition,
            "deadline_tick":options.deadline_tick,"poll_interval_ticks":cadence,"stable_observations":stability}});
    let identity =
        json!({"policy":POLICY,"session_id":context.session_id.to_string(),"request":request});
    let encoded = serde_json::to_vec(&identity)
        .map_err(|_| invalid("blueprint watch proposal cannot be encoded"))?;
    Ok(
        json!({"policy":POLICY,"proposal_digest":Digest32::of_bytes(&encoded).to_string(),
        "tool":"fortress.query","session_id":context.session_id.to_string(),"watch_request":request,
        "target_shapes":targets,"requested_tiles":layout.tile_count(),"watch_registered":false,
        "native_job_completion_proven":false,"mutation_cause_proven":false,
        "interpretation":"Submit watch_request explicitly with this session_id. It monitors the declared shapes over every excavation coordinate, including corridors but excluding reserved crossings. Existing shapes can satisfy it; no mining causation, job completion, access or safety is proved."}),
    )
}

#[cfg(test)]
#[path = "spatial_blueprint_monitor_tests.rs"]
mod tests;
