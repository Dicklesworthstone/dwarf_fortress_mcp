//! Observed workforce attachments and advisory choices; never mutation authority.
use super::*;
use dfmcp_adapter::live_routing::{LiveRoutingEvidence, NativeUnitIdentity};
use dfmcp_adapter::workforce_control::{MAX_UNITS, WorkforceCapture};
use dfmcp_core::EntityId;
use dfmcp_world::EntityKind;
use serde::Deserialize;

/// Observe selectors change only the bounded advisory projection. They never
/// become native IDs, work-detail selectors or mutation authority.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ObserveRequest {
    units: Option<Vec<EntityId>>,
    option_offset: usize,
    existing_labor_offset: usize,
    include_production: bool,
}

// workforce/1.17 admits at most 128 labor columns. No request can ask for an
// unbounded offset, even before the complete native capture is available.
const MAX_OPTION_OFFSET: usize = 128;

impl ObserveRequest {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            units: Option<Vec<String>>,
            #[serde(default)]
            option_offset: usize,
            #[serde(default)]
            existing_labor_offset: usize,
            #[serde(default, alias = "include_operations")]
            include_production: bool,
        }
        if raw.len() > 4096 {
            return Err(exhausted("combined observe request exceeds 4096 bytes"));
        }
        let input: Input = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed combined observe request"))?;
        if input.option_offset > MAX_OPTION_OFFSET
            || input.existing_labor_offset > MAX_OPTION_OFFSET
        {
            return Err(invalid(
                "workforce option offset exceeds the native labor bound",
            ));
        }
        let units = input
            .units
            .map(|raw_units| -> Result<Vec<EntityId>> {
                if raw_units.is_empty() || raw_units.len() > MAX_UNITS {
                    return Err(invalid("select between one and 32 canonical citizens"));
                }
                let mut units = raw_units
                    .iter()
                    .map(|raw| {
                        let value = raw
                            .parse::<u64>()
                            .map_err(|_| invalid("unit identity must be canonical decimal"))?;
                        if value == 0 || raw != &value.to_string() {
                            return Err(invalid("unit identity must be nonzero canonical decimal"));
                        }
                        Ok(EntityId::new(value))
                    })
                    .collect::<Result<Vec<_>>>()?;
                units.sort();
                if units.windows(2).any(|pair| pair[0] == pair[1]) {
                    return Err(invalid(
                        "duplicate canonical citizen identities are refused",
                    ));
                }
                Ok(units)
            })
            .transpose()?;
        Ok(Self {
            units,
            option_offset: input.option_offset,
            existing_labor_offset: input.existing_labor_offset,
            include_production: input.include_production,
        })
    }

    pub(crate) fn include_production(&self) -> bool {
        self.include_production
    }

    // The independent production attachment does not change the workforce
    // projection or its existing witness bytes.
    pub(crate) fn is_default(&self) -> bool {
        self.units.is_none() && self.option_offset == 0 && self.existing_labor_offset == 0
    }

    fn selection(&self) -> &'static str {
        if self.units.is_some() {
            "canonical_id_ascending_explicit"
        } else {
            "canonical_id_ascending_first_32"
        }
    }

    pub(crate) fn selection_json(&self) -> Value {
        let mut value = json!({"option_offset":self.option_offset});
        if self.existing_labor_offset != 0 {
            value["existing_labor_offset"] = json!(self.existing_labor_offset);
        }
        if let Some(units) = &self.units {
            value["units"] = json!(
                units
                    .iter()
                    .map(|id| id.get().to_string())
                    .collect::<Vec<_>>()
            );
        }
        value
    }
}

pub(super) const MAX_CHOICES: usize = 8;

pub(super) fn select(
    capsule: &LiveObservationCapsule,
    projection: &LiveWorldProjection,
    scope: &workforce::policy::SourceAcceptance,
    request: &ObserveRequest,
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
    let selected = if let Some(units) = &request.units {
        if units.iter().any(|id| ids.binary_search(id).is_err()) {
            return Err(invalid(
                "selected citizen is absent from the exact canonical roster",
            ));
        }
        units.as_slice()
    } else {
        &ids[..ids.len().min(MAX_UNITS)]
    };
    if selected.is_empty() {
        return Ok((Vec::new(), ids.len()));
    }
    Ok((routing.resolve_units(selected)?, ids.len()))
}

pub(super) fn empty(
    anchor: StateAnchor,
    seal: Digest32,
    request: &ObserveRequest,
) -> Result<Value> {
    if !request.is_default() {
        return Err(invalid(
            "empty canonical roster has no selected citizens or later option page",
        ));
    }
    Ok(
        json!({"schema":"dfmcp.combined-workforce-choices/1","canonical_anchor":anchor_json(anchor),
        "canonical_observation_seal":seal.to_hex(),"epistemic_state":"observed",
        "selection_request":request.selection_json(),"witness_version":null,
        "next_observation_request":null,"fresh_observation_required":true,
        "selected_citizens":[],"options":[],"existing_labor_options":[],
        "next_existing_labor_observation_request":null,"native_observed":false,
        "coverage":{"canonical_citizens":0,"selected_citizens":0,"omitted_citizens":0,
            "selection":request.selection(),"work_detail_domain":"unknown","omitted_options":0,
            "paging":"independent_fresh_observations",
            "available_options":null,"option_offset":0,"returned_options":0,
            "omitted_options_before":0,"omitted_options_after":null,"next_option_offset":null,
            "existing_labor":{"available_options":null,"offset":0,"returned_options":0,
                "omitted_before":0,"omitted_after":null,"next_offset":null}},
        "revalidation_required":true,"mutation_authority":false,"mutation_precondition_evidence":false}),
    )
}

pub(super) fn project(
    capsule: &LiveObservationCapsule,
    projection: &LiveWorldProjection,
    scope: &workforce::policy::SourceAcceptance,
    capture: &WorkforceCapture,
    request: &ObserveRequest,
    c: &OperationContext,
) -> Result<Value> {
    let (units, total) = select(capsule, projection, scope, request, c)?;
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
    let mut existing_options = Vec::new();
    for (column, key) in capture.labor_keys().iter().enumerate() {
        if !workforce::request::valid_labor_key(key) {
            continue;
        }
        let enabled = units
            .iter()
            .zip(capture.citizens())
            .filter(|(_, citizen)| citizen.eligible() && citizen.labors()[column] != 0)
            .map(|(identity, _)| identity.canonical_id.get().to_string())
            .collect::<Vec<_>>();
        if !enabled.is_empty() {
            existing_options.push(json!({"labor":key,"eligible_enabled_ids":enabled,
                "request_kind":"existing_labor","assignment_required":false,
                "candidates_epistemic_state":"certified_derived","productivity_proven":false}));
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
    existing_options.sort_by(|left, right| left["labor"].as_str().cmp(&right["labor"].as_str()));
    let available_existing = existing_options.len();
    if request.existing_labor_offset > available_existing {
        return Err(invalid(
            "existing-labor offset exceeds the current available options",
        ));
    }
    let existing_end = request
        .existing_labor_offset
        .saturating_add(MAX_CHOICES)
        .min(available_existing);
    let existing_options = existing_options
        .into_iter()
        .skip(request.existing_labor_offset)
        .take(MAX_CHOICES)
        .collect::<Vec<_>>();
    let next_existing_offset = (existing_end < available_existing).then_some(existing_end);
    let available_options = options.len();
    if request.option_offset > available_options {
        return Err(invalid(
            "workforce option offset exceeds the current available options",
        ));
    }
    let page_end = request
        .option_offset
        .saturating_add(MAX_CHOICES)
        .min(available_options);
    let options = options
        .into_iter()
        .skip(request.option_offset)
        .take(MAX_CHOICES)
        .collect::<Vec<_>>();
    let omitted_options = available_options.saturating_sub(options.len());
    let next_option_offset = (page_end < available_options).then_some(page_end);
    // Pin the resolved citizens even when this page used the default window.
    // A later Observe must read again and must not silently substitute a new
    // first-32 selection if the canonical roster has changed.
    let page_request = |offset: usize, existing_offset: usize| {
        let mut value = json!({"units":units.iter().map(|unit|unit.canonical_id.get().to_string()).collect::<Vec<_>>(),
            "option_offset":offset});
        if existing_offset != 0 {
            value["existing_labor_offset"] = json!(existing_offset);
        }
        if request.include_production() {
            value["include_production"] = json!(true);
        }
        value
    };
    let next_observation_request =
        next_option_offset.map(|offset| page_request(offset, request.existing_labor_offset));
    let next_existing_request =
        next_existing_offset.map(|offset| page_request(request.option_offset, offset));
    let seal = observation_seal(capsule, projection);
    // Retain the original witness bytes for the default projection. A selected
    // roster or later page binds its normalized selectors in a distinct domain.
    let witness_version = if request.existing_labor_offset != 0 {
        3
    } else if request.is_default() {
        1
    } else {
        2
    };
    let mut bytes = if request.existing_labor_offset != 0 {
        b"dfmcp-combined-workforce-choices/3\0".to_vec()
    } else if request.is_default() {
        b"dfmcp-combined-workforce-choices/1\0".to_vec()
    } else {
        b"dfmcp-combined-workforce-choices/2\0".to_vec()
    };
    bytes.extend_from_slice(&(MAX_UNITS as u32).to_be_bytes());
    bytes.extend_from_slice(&(MAX_CHOICES as u32).to_be_bytes());
    bytes.extend_from_slice(b"canonical_id_ascending\0labor_key_ascending\0");
    bytes.extend_from_slice(seal.as_bytes());
    bytes.extend_from_slice(capture.witness().as_bytes());
    if !request.is_default() {
        bytes.push(u8::from(request.units.is_some()));
        let selected = request.units.as_deref().unwrap_or(&[]);
        bytes.extend_from_slice(&(selected.len() as u32).to_be_bytes());
        for id in selected {
            bytes.extend_from_slice(&id.get().to_be_bytes());
        }
        bytes.extend_from_slice(&(request.option_offset as u32).to_be_bytes());
        if request.existing_labor_offset != 0 {
            bytes.extend_from_slice(&(request.existing_labor_offset as u32).to_be_bytes());
        }
    }
    let source = json!({"protocol":"workforce/1.17","generation":capture.generation(),
        "sequence":capture.sequence(),"capture_witness":capture.witness().to_hex(),
        "df_version":scope.df_version,"dfhack_version":scope.dfhack_version,
        "tick":capture.tick(),"paused":capture.paused(),"automatic":capture.automatic()});
    let existing_coverage = json!({"available_options":available_existing,
        "offset":request.existing_labor_offset,"returned_options":existing_options.len(),
        "omitted_before":request.existing_labor_offset,
        "omitted_after":available_existing.saturating_sub(existing_end),
        "next_offset":next_existing_offset,"order":"labor_key_ascending",
        "detail_shape_required":false,"automatic_professions_required":false});
    let coverage = json!({"canonical_citizens":total,"selected_citizens":units.len(),
            "omitted_citizens":total.saturating_sub(units.len()),"selection":request.selection(),
            "work_detail_domain":"complete_capture_partial_projection",
            "native_work_details":capture.details().len(),"all_work_details_returned":false,
            "projection":"actionable_single_labor_options_for_selected_citizens",
            "unrepresented_work_details":capture.details().len().saturating_sub(options.len()),
            "option_limit":MAX_CHOICES,
            "omitted_options":omitted_options,"option_order":"labor_key_ascending",
            "paging":"independent_fresh_observations",
            "available_options":available_options,"option_offset":request.option_offset,
            "returned_options":options.len(),"omitted_options_before":request.option_offset,
            "omitted_options_after":available_options.saturating_sub(page_end),
            "next_option_offset":next_option_offset,"existing_labor":existing_coverage});
    Ok(json!({"schema":"dfmcp.combined-workforce-choices/1",
        "canonical_anchor":anchor_json(projection.snapshot.anchor()),
        "canonical_observation_seal":seal.to_hex(),"epistemic_state":"observed",
        "selection_request":request.selection_json(),"witness_version":witness_version,
        "next_observation_request":next_observation_request,"fresh_observation_required":true,
        "next_existing_labor_observation_request":next_existing_request,
        "witness":Digest32::of_bytes(&bytes).to_hex(),"native_observed":true,
        "source":source,"labor_keys":capture.labor_keys(),"selected_citizens":selected,
        "options":options,"existing_labor_options":existing_options,"coverage":coverage,
        "revalidation_required":true,"mutation_authority":false,"mutation_precondition_evidence":false,
        "current_state_proven":false,"productivity_proven":false}))
}

#[cfg(test)]
#[path = "choices_tests.rs"]
mod tests;
