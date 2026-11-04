//! Observed workforce attachments and advisory choices; never mutation authority.
use super::*;
use dfmcp_adapter::live_routing::{LiveRoutingEvidence, NativeUnitIdentity};
use dfmcp_adapter::workforce_control::{MAX_UNITS, WorkforceCapture};
use dfmcp_world::EntityKind;

pub(super) const MAX_CHOICES: usize = 8;

pub(super) fn select(
    capsule: &LiveObservationCapsule,
    projection: &LiveWorldProjection,
    scope: &workforce::policy::SourceAcceptance,
    c: &OperationContext,
) -> Result<(Vec<NativeUnitIdentity>, usize)> {
    let policy = scope.citizens(capsule, projection, c)?;
    let evidence = PredicateEvidence::scoped(&projection.snapshot, policy)?;
    let routing = LiveRoutingEvidence::citizens_v1(evidence, projection, capsule)?;
    let ids = projection
        .snapshot
        .graph
        .entities
        .iter()
        .filter(|(_, entity)| entity.kind == EntityKind::Unit)
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Ok((Vec::new(), 0));
    }
    Ok((
        routing.resolve_units(&ids[..ids.len().min(MAX_UNITS)])?,
        ids.len(),
    ))
}

pub(super) fn empty(anchor: StateAnchor, seal: Digest32) -> Value {
    json!({"schema":"dfmcp.combined-workforce-choices/1","canonical_anchor":anchor_json(anchor),
        "canonical_observation_seal":seal.to_hex(),"epistemic_state":"observed",
        "selected_citizens":[],"options":[],"native_observed":false,
        "coverage":{"canonical_citizens":0,"selected_citizens":0,"omitted_citizens":0,
            "work_detail_domain":"unknown","omitted_options":0},
        "revalidation_required":true,"mutation_authority":false,"mutation_precondition_evidence":false})
}

pub(super) fn project(
    capsule: &LiveObservationCapsule,
    projection: &LiveWorldProjection,
    scope: &workforce::policy::SourceAcceptance,
    capture: &WorkforceCapture,
    c: &OperationContext,
) -> Result<Value> {
    let (units, total) = select(capsule, projection, scope, c)?;
    if capture.generation() != scope.workforce_generation
        || capture.folder() != scope.world_folder
        || capture.site() != scope.site_id
        || capture.tick() != projection.snapshot.tick.get()
        || !capture.paused()
        || capture.ids() != units.iter().map(|unit| unit.native_id).collect::<Vec<_>>()
    {
        return Err(denied(
            "workforce choices differ from the exact paused canonical selection",
        ));
    }
    if capture.entity_cost() > c.budget.max_entities as usize {
        return Err(exhausted(
            "workforce choice projection exceeds entity allowance",
        ));
    }
    let selected = units
        .iter()
        .zip(capture.citizens())
        .map(|(identity, native)| {
            json!({"entity_id":identity.canonical_id.get().to_string(),
            "generation":identity.canonical_generation,"native_id":native.id(),
            "historical_id":native.historical_id(),"eligible":native.eligible()})
        })
        .collect::<Vec<_>>();
    let mut options = Vec::new();
    for (column, key) in capture.labor_keys().iter().enumerate() {
        if !workforce::request::valid_labor_key(key) {
            continue;
        }
        let matching = capture
            .details()
            .iter()
            .enumerate()
            .filter(|(_, detail)| {
                detail.selected_only()
                    && detail
                        .labors()
                        .iter()
                        .enumerate()
                        .all(|(index, bit)| *bit == u8::from(index == column))
            })
            .collect::<Vec<_>>();
        let [(detail_index, detail)] = matching.as_slice() else {
            continue;
        };
        let mut member_ids = Vec::new();
        let mut enabled_ids = Vec::new();
        let mut enable = Vec::new();
        let mut disable = Vec::new();
        for (identity, citizen) in units.iter().zip(capture.citizens()) {
            let id = identity.canonical_id.get().to_string();
            let member = detail.members().binary_search(&citizen.id()).is_ok();
            let enabled = citizen.labors()[column] != 0;
            if member {
                member_ids.push(id.clone());
            }
            if enabled {
                enabled_ids.push(id.clone());
            }
            if !capture.automatic() || !citizen.eligible() {
                continue;
            }
            if !member && !enabled {
                enable.push(id.clone());
            }
            let other_grant = capture.details().iter().enumerate().any(|(index, other)| {
                index != *detail_index
                    && other.labors()[column] != 0
                    && (!other.selected_only()
                        || other.members().binary_search(&citizen.id()).is_ok())
            });
            if member && enabled && !other_grant {
                disable.push(id);
            }
        }
        if enable.is_empty() && disable.is_empty() {
            continue;
        }
        options.push(json!({"labor":key,"detail_name":detail.name(),"selected_only":true,
            "member_ids":member_ids,"labor_enabled_ids":enabled_ids,
            "enable_candidates":enable,"disable_candidates":disable,
            "candidates_epistemic_state":"certified_derived","other_recomputed_labor_effects":"unknown"}));
    }
    options.sort_by(|left, right| left["labor"].as_str().cmp(&right["labor"].as_str()));
    let omitted_options = options.len().saturating_sub(MAX_CHOICES);
    options.truncate(MAX_CHOICES);
    let seal = observation_seal(capsule, projection);
    let mut bytes = b"dfmcp-combined-workforce-choices/1\0".to_vec();
    bytes.extend_from_slice(&(MAX_UNITS as u32).to_be_bytes());
    bytes.extend_from_slice(&(MAX_CHOICES as u32).to_be_bytes());
    bytes.extend_from_slice(b"canonical_id_ascending\0labor_key_ascending\0");
    bytes.extend_from_slice(seal.as_bytes());
    bytes.extend_from_slice(capture.witness().as_bytes());
    Ok(json!({"schema":"dfmcp.combined-workforce-choices/1",
        "canonical_anchor":anchor_json(projection.snapshot.anchor()),
        "canonical_observation_seal":seal.to_hex(),"epistemic_state":"observed",
        "witness":Digest32::of_bytes(&bytes).to_hex(),"native_observed":true,
        "source":{"protocol":"workforce/1.17","generation":capture.generation(),
            "sequence":capture.sequence(),"capture_witness":capture.witness().to_hex(),
            "df_version":scope.df_version,"dfhack_version":scope.dfhack_version,
            "tick":capture.tick(),"paused":capture.paused(),"automatic":capture.automatic()},
        "labor_keys":capture.labor_keys(),"selected_citizens":selected,"options":options,
        "coverage":{"canonical_citizens":total,"selected_citizens":units.len(),
            "omitted_citizens":total.saturating_sub(units.len()),"selection":"canonical_id_ascending_first_32",
            "work_detail_domain":"complete_capture_partial_projection",
            "native_work_details":capture.details().len(),"all_work_details_returned":false,
            "projection":"actionable_single_labor_options_for_selected_citizens",
            "unrepresented_work_details":capture.details().len().saturating_sub(options.len()),
            "option_limit":MAX_CHOICES,
            "omitted_options":omitted_options,"option_order":"labor_key_ascending"},
        "revalidation_required":true,"mutation_authority":false,"mutation_precondition_evidence":false,
        "current_state_proven":false,"productivity_proven":false}))
}

#[cfg(test)]
#[path = "choices_tests.rs"]
mod tests;
