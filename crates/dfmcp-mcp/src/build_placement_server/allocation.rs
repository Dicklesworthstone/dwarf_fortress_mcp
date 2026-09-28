//! A fresh source-bound furnishing request becomes one immutable exact batch.
//! Detailed request and selected-item views stay separate so the complete
//! original placement and construction inventories remain visible together.
use super::{Config, error};
use dfmcp_adapter::build_placement::{BuildKind, BuildSelection};
use dfmcp_adapter::furniture_handoff::{AllocationOutcome, FurnitureRequest, Handoff, Source};
use dfmcp_core::{ErrorCode, Result};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum View {
    #[default]
    Request,
    Items,
}

pub(super) fn validate_request(request: &FurnitureRequest, config: &Config) -> Result<()> {
    if request.folder() != config.fortress.folder() || request.site() != config.fortress.site() {
        return Err(error(
            ErrorCode::StaleAnchor,
            "furniture request names another fortress",
        ));
    }
    for slot in &request.request().slots {
        let kind = match slot.kind {
            dfmcp_adapter::furniture_allocation::Kind::Bed => BuildKind::Bed,
            dfmcp_adapter::furniture_allocation::Kind::Chair => BuildKind::Chair,
            dfmcp_adapter::furniture_allocation::Kind::Table => BuildKind::Table,
        };
        // The item is not selected yet. Only validate the requested target's
        // complete native 3x3 context and the operator's protected regions.
        let halo = config.selection(BuildSelection::new(kind, 0, slot.target)?)?;
        if config
            .protected
            .iter()
            .any(|region| dfmcp_core::cuboids_intersect(region, &halo))
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "furniture target intersects protected region",
            ));
        }
    }
    Ok(())
}

pub(super) fn summary(handoff: &Handoff) -> Value {
    let source = handoff.source();
    json!({"handoff_digest":handoff.digest().to_string(),
        "request_digest":handoff.request().digest().to_string(),
        "capture_sha256":source.capture_digest.to_string(),
        "source_digest":source.source_digest.to_string(),
        "operations_generation":source.generation,"captured_tick":source.anchor.tick.get(),
        "fixed_items":handoff.items().len(),"constraints_retained":true,"items_reserved":false})
}

fn source(value: &Source) -> Value {
    json!({"protocol":"1.4","generation":value.generation,
        "endpoint":value.endpoint.to_string(),"df_version":value.df_version,"dfhack_version":value.dfhack_version,
        "capture_sha256":value.capture_digest.to_string(),"capture_bytes":value.capture_bytes,
        "source_digest":value.source_digest.to_string(),"captured_tick":value.anchor.tick.get(),
        "operations_snapshot":{"fortress_id":value.anchor.fortress_id.to_string(),
            "epoch":value.anchor.cursor.epoch,"sequence":value.anchor.cursor.sequence,
            "state_hash":value.anchor.state_hash.to_string()},
        "next_job_id":value.next_job_id,"next_building_id":value.next_building_id,"next_item_id":value.next_item_id,
        "historical_evidence_only":true,"current_availability_proven":false})
}

fn request(value: &FurnitureRequest) -> Result<Value> {
    serde_json::from_slice(value.canonical_bytes()).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "retained furniture request cannot be presented",
        )
    })
}

pub(super) fn display(handoff: &Handoff, view: View) -> Result<Value> {
    let mut out = summary(handoff);
    out["source"] = source(handoff.source());
    out["inventory_verified"] = json!(true);
    out["view"] = json!(match view {
        View::Request => "request",
        View::Items => "items",
    });
    match view {
        View::Request => out["request"] = request(handoff.request())?,
        View::Items => {
            out["items"] = json!(handoff.items().iter().map(|item| {
                let candidate = &item.candidate;
                json!({"slot":item.slot,"item_id":candidate.native_id,"kind":candidate.kind.as_str(),
                    "native_type":item.native_type,"position":candidate.position,
                    "material":[candidate.material_type,candidate.material_index],"subtype":candidate.subtype,
                    "distance":item.distance,"entity_id":item.handle.entity_id.to_string(),
                    "generation":item.handle.generation,"revision":item.handle.revision})
            }).collect::<Vec<_>>())
        }
    }
    Ok(out)
}

pub(super) fn shortage(outcome: &AllocationOutcome, requested: &FurnitureRequest) -> Result<Value> {
    let report = &outcome.report;
    let shortage = report.allocation.shortage.as_ref().ok_or_else(|| {
        error(
            ErrorCode::InternalInvariantViolation,
            "incomplete furniture assignment lacks a shortage witness",
        )
    })?;
    if outcome.handoff.is_some() || !report.allocation.assignments.is_empty() {
        return Err(error(
            ErrorCode::InternalInvariantViolation,
            "shortage unexpectedly contains an executable assignment",
        ));
    }
    Ok(json!({"request":request(requested)?,"request_digest":requested.digest().to_string(),
        "source_digest":report.source_digest.to_string(),"captured_tick":report.anchor.tick.get(),
        "observed_items":report.observed_items,"candidate_items":report.candidate_count,
        "maximum_assignable":report.allocation.maximum_assignable,
        "shortage":{"slots":shortage.slots,"candidate_items":shortage.candidate_items,"missing":shortage.missing},
        "compatible_counts":report.allocation.compatible_counts.iter().map(|(slot,count)|json!({"slot":slot,"count":count})).collect::<Vec<_>>(),
        "items":report.items.values().map(|item|json!({"item_id":item.candidate.native_id,
            "kind":item.candidate.kind.as_str(),"position":item.candidate.position,
            "material":[item.candidate.material_type,item.candidate.material_index],"subtype":item.candidate.subtype,
            "entity_id":item.handle.entity_id.to_string(),"generation":item.handle.generation,"revision":item.handle.revision})).collect::<Vec<_>>(),
        "supply_policy":dfmcp_adapter::furniture_supply::SUPPLY_POLICY,"complete_shortage_witness":true,
        "plan":null,"items_reserved":false,"game_mutation_dispatched":false,
        "interpretation":"The requested slots cannot all be assigned within this observation's conservative furniture model. No partial batch was created."}))
}
