//! Bounded projection of the exact selected native preparation, not a new read.
use crate::semantic_workforce_runtime::denied;
use dfmcp_adapter::live_production::StepEvidence;
use dfmcp_adapter::live_projection::raw_unit_id_to_entity_id;
use dfmcp_adapter::work_order_control::CreationState;
use dfmcp_adapter::work_orders::WorkOrderRecipe;
use dfmcp_adapter::workforce_control::journal::AssignmentState;
use dfmcp_core::Result;
use dfmcp_intent::Action;
use serde_json::{Value, json};

pub(super) fn project(evidence: &StepEvidence) -> Result<Value> {
    match evidence {
        StepEvidence::Labor(result) => {
            let review = result.review();
            let plan = review.native_plan();
            if result.native_record().state() != AssignmentState::Prepared
                || result.native_record().plan() != plan
                || plan.key() != review.selected_step().idempotency_key
            {
                return Err(denied(
                    "labor review no longer matches its original prepared record",
                ));
            }
            let Action::SetLabor {
                units,
                labor,
                enabled,
            } = &review.selected_step().action
            else {
                return Err(denied("labor review names another semantic action"));
            };
            let capture = plan.before();
            let detail = capture
                .details()
                .get(plan.spec().detail() as usize)
                .ok_or_else(|| denied("selected labor detail is absent"))?;
            let column = capture
                .labor_keys()
                .iter()
                .position(|key| key == labor)
                .ok_or_else(|| denied("requested labor is absent from native capture"))?;
            if units.len() != capture.citizens().len()
                || *enabled != plan.spec().assigned()
                || !detail.selected_only()
                || detail.labors().get(column).copied() != Some(1)
            {
                return Err(denied(
                    "native selected labor does not match the original action",
                ));
            }
            let mut citizens = Vec::with_capacity(units.len());
            for citizen in capture.citizens() {
                let raw = i32::try_from(citizen.id())
                    .map_err(|_| denied("native citizen ID is out of range"))?;
                let canonical = raw_unit_id_to_entity_id(raw)?;
                if !units.contains(&canonical) {
                    return Err(denied(
                        "native citizen does not match the canonical selection",
                    ));
                }
                let before = citizen
                    .labors()
                    .get(column)
                    .copied()
                    .ok_or_else(|| denied("native labor column is absent"))?
                    != 0;
                let member = detail.members().binary_search(&citizen.id()).is_ok();
                citizens.push(json!({"canonical_id":canonical.get().to_string(),"native_id":citizen.id(),
                    "historical_id":citizen.historical_id(),"eligible_observed":citizen.eligible(),
                    "member_observed":member,"member_intended":enabled,"membership_changes":member != *enabled,
                    "labor_observed":before,"labor_intended":enabled}));
            }
            Ok(
                json!({"step_id":review.step_id().get(),"family":"workforce/1.17",
                "preparation_digest":review.seal().to_hex(),"original_plan_digest":review.original_plan().digest.to_hex(),
                "idempotency_key":plan.key(),"native_plan_digest":plan.digest().to_hex(),
                "native_review":{"capture_witness":capture.witness().to_hex(),"source_generation":capture.generation(),
                    "tick":capture.tick(),"paused":capture.paused(),"detail_index":plan.spec().detail(),
                    "detail_name":detail.name(),"selected_only":true,"labor":labor,"labor_column":column,
                    "citizens":citizens,"scope":"selected_citizens_and_requested_labor_only",
                    "other_members_and_labor_columns":"not_shown","other_recomputed_labor_effects":"unknown",
                    "capture_is_current_state":false,"intended_is_observed":false}}),
            )
        }
        StepEvidence::WorkOrder(result) => {
            let review = result.review();
            let plan = review.native_plan();
            if result.native_record().state() != CreationState::Prepared
                || result.native_record().plan() != plan
                || plan.key() != review.selected_step().idempotency_key
            {
                return Err(denied(
                    "order review no longer matches its original prepared record",
                ));
            }
            let Action::CreateWorkOrder {
                job_token,
                amount,
                conditions,
                ..
            } = &review.selected_step().action
            else {
                return Err(denied("order review names another semantic action"));
            };
            if !order_matches(
                plan.spec().recipe(),
                plan.spec().amount(),
                job_token,
                *amount,
                conditions.is_empty(),
            ) {
                return Err(denied(
                    "native finite order does not match the original action",
                ));
            }
            let before = plan.observation();
            Ok(
                json!({"step_id":review.step_id().get(),"family":"work-orders/1.10",
                "preparation_digest":review.seal().to_hex(),"original_plan_digest":review.original_plan().digest.to_hex(),
                "idempotency_key":plan.key(),"native_plan_digest":plan.digest().to_hex(),
                "native_review":{"recipe":plan.spec().recipe().as_str(),"amount":plan.spec().amount(),
                    "queue_witness":before.witness().to_hex(),"source_generation":before.generation(),
                    "tick":before.tick(),"paused":before.paused(),"next_order_id":before.next_order_id(),
                    "capture_is_current_state":false,"insertion_is_completion":false,"physical_output":"unknown"}}),
            )
        }
    }
}

fn order_matches(
    recipe: WorkOrderRecipe,
    native_amount: u32,
    token: &str,
    amount: u32,
    no_conditions: bool,
) -> bool {
    let expected = match recipe {
        WorkOrderRecipe::WoodenBed => "CONSTRUCT_BED",
        WorkOrderRecipe::WoodenDoor => "CONSTRUCT_DOOR",
        WorkOrderRecipe::WoodenTable => "CONSTRUCT_TABLE",
        WorkOrderRecipe::WoodenChair => "CONSTRUCT_CHAIR",
    };
    token == expected && amount == native_amount && no_conditions
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finite_order_review_rejects_wrong_recipe_amount_and_conditions() {
        assert!(order_matches(
            WorkOrderRecipe::WoodenBed,
            1,
            "CONSTRUCT_BED",
            1,
            true
        ));
        assert!(!order_matches(
            WorkOrderRecipe::WoodenBed,
            1,
            "CONSTRUCT_DOOR",
            1,
            true
        ));
        assert!(!order_matches(
            WorkOrderRecipe::WoodenBed,
            1,
            "CONSTRUCT_BED",
            2,
            true
        ));
        assert!(!order_matches(
            WorkOrderRecipe::WoodenBed,
            1,
            "CONSTRUCT_BED",
            1,
            false
        ));
    }
}
