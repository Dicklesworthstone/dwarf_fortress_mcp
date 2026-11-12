//! Compact immutable input assessment in the existing original-plan archive.
//! Decoding this commitment restores historical policy, never current supply.
use super::input_config::{InputConfig, InputFloor};
use super::{digest, invalid};
use dfmcp_adapter::semantic_work_orders::{
    FurnitureInputRequirements, InputSupplyAssessment, InputSupplySource,
};
use dfmcp_adapter::work_orders::{WorkOrderRecipe, WorkOrderSpec};
use dfmcp_core::{Digest32, Result};
use dfmcp_intent::{Action, PreparedPlan};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InputBinding {
    pub(super) reserve: u64,
    pub(super) required: u64,
    pub(super) eligible: u64,
    pub(super) generation: u64,
    pub(super) tick: u64,
    pub(super) counters: [u64; 3],
    pub(super) rows: u64,
    pub(super) source: Digest32,
    pub(super) requirements: Digest32,
    pub(super) capture: Digest32,
    pub(super) assessment: Digest32,
}

impl InputBinding {
    pub(super) fn new(assessment: &InputSupplyAssessment) -> Result<Self> {
        if !assessment.ready() {
            return Err(invalid(
                "input supply shortage cannot seal an executable original",
            ));
        }
        Ok(Self {
            reserve: assessment.requirements().reserve_stack_units(),
            required: assessment.required_stack_units(),
            eligible: assessment.eligible_stack_units(),
            generation: assessment.source_generation(),
            tick: assessment.original_anchor().tick.get(),
            counters: assessment.source_counters(),
            rows: u64::from(assessment.item_rows()),
            source: assessment.source_witness(),
            requirements: assessment.requirements().witness(),
            capture: assessment.source_digest(),
            assessment: assessment.witness(),
        })
    }

    pub(super) fn encode(self) -> String {
        let mut text = String::with_capacity(400);
        for value in [
            self.reserve,
            self.required,
            self.eligible,
            self.generation,
            self.tick,
            self.counters[0],
            self.counters[1],
            self.counters[2],
            self.rows,
        ] {
            text.push_str(&format!("{value:016x}"));
        }
        for value in [
            self.source,
            self.requirements,
            self.capture,
            self.assessment,
        ] {
            text.push_str(&value.to_hex());
        }
        text
    }

    pub(super) fn parse(text: &str) -> Result<Self> {
        if text.len() != 400
            || !text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid(
                "input supply marker requires exact canonical lowercase bytes",
            ));
        }
        let mut fields = [0; 9];
        for (index, field) in fields.iter_mut().enumerate() {
            *field = u64::from_str_radix(&text[index * 16..(index + 1) * 16], 16)
                .map_err(|_| invalid("invalid input supply marker integer"))?;
        }
        let value = Self {
            reserve: fields[0],
            required: fields[1],
            eligible: fields[2],
            generation: fields[3],
            tick: fields[4],
            counters: [fields[5], fields[6], fields[7]],
            rows: fields[8],
            source: digest(&text[144..208])?,
            requirements: digest(&text[208..272])?,
            capture: digest(&text[272..336])?,
            assessment: digest(&text[336..400])?,
        };
        if value.reserve > 1_000_000
            || value.required <= value.reserve
            || value.required - value.reserve > 400
            || value.eligible < value.required
            || value.generation == 0
            || value.generation == u64::MAX
            || value.tick > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
            || value.counters.into_iter().any(|id| id > i32::MAX as u64)
            || value.rows > 4096
        {
            return Err(invalid(
                "input supply marker exceeds its complete source or demand bounds",
            ));
        }
        Ok(value)
    }

    pub(super) fn witness(self) -> Digest32 {
        let mut bytes = b"dfmcp-combined-input-supply-binding/1\0".to_vec();
        bytes.extend_from_slice(self.encode().as_bytes());
        Digest32::of_bytes(&bytes)
    }

    pub(super) fn floor(self) -> InputFloor {
        InputFloor {
            tick: self.tick,
            generation: self.generation,
            next_job_id: self.counters[0] as u32,
            next_building_id: self.counters[1] as u32,
            next_item_id: self.counters[2] as u32,
            fenced: false,
        }
    }

    pub(super) fn validate_original(
        self,
        plan: &PreparedPlan,
        config: InputConfig,
        source: &InputSupplySource,
    ) -> Result<FurnitureInputRequirements> {
        let requirements =
            FurnitureInputRequirements::new(&plan_orders(plan)?, config.reserve_wood_stack_units)?;
        if self.source != source.witness()
            || self.generation != source.generation()
            || self.tick != plan.anchor.tick.get()
            || self.reserve != config.reserve_wood_stack_units
            || self.required != requirements.required_stack_units()
            || self.requirements != requirements.witness()
        {
            return Err(invalid(
                "original input supply policy, source or complete demand was substituted",
            ));
        }
        Ok(requirements)
    }
}

pub(super) fn plan_orders(plan: &PreparedPlan) -> Result<Vec<WorkOrderSpec>> {
    let layout = dfmcp_adapter::live_production::ProductionLayout::from_plan(plan)?;
    let mut orders = Vec::with_capacity(layout.order_steps().len());
    for id in layout.order_steps() {
        let step = plan
            .steps
            .iter()
            .find(|step| step.id == *id)
            .ok_or_else(|| invalid("input supply lost an original order step"))?;
        let Action::CreateWorkOrder {
            job_token,
            amount,
            conditions,
            ..
        } = &step.action
        else {
            return Err(invalid(
                "input supply requires original finite furniture orders",
            ));
        };
        if !conditions.is_empty() {
            return Err(invalid(
                "input supply does not admit conditional native recipes",
            ));
        }
        let recipe = match job_token.as_str() {
            "CONSTRUCT_BED" => WorkOrderRecipe::WoodenBed,
            "CONSTRUCT_DOOR" => WorkOrderRecipe::WoodenDoor,
            "CONSTRUCT_TABLE" => WorkOrderRecipe::WoodenTable,
            "CONSTRUCT_CHAIR" => WorkOrderRecipe::WoodenChair,
            _ => {
                return Err(invalid(
                    "input supply original has an unknown furniture recipe",
                ));
            }
        };
        orders.push(WorkOrderSpec::new(recipe, *amount)?);
    }
    Ok(orders)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> InputBinding {
        InputBinding {
            reserve: 1,
            required: 5,
            eligible: 10,
            generation: 27,
            tick: 120,
            counters: [80, 90, 100],
            rows: 4,
            source: Digest32::of_bytes(b"input source pins"),
            requirements: Digest32::of_bytes(b"all four original furniture orders and reserve"),
            capture: Digest32::of_bytes(b"complete original operations census"),
            assessment: Digest32::of_bytes(b"original joint allocation"),
        }
    }

    #[test]
    fn exact_input_marker_commits_all_source_demand_and_initial_floor_bytes() -> Result<()> {
        let original = binding();
        let encoded = original.encode();
        assert_eq!(encoded.len(), 400);
        assert_eq!(InputBinding::parse(&encoded)?, original);
        assert_eq!(original.floor().counters(), [80, 90, 100]);
        let mut variants = Vec::new();
        for field in 0..9 {
            let mut changed = original;
            match field {
                0 => changed.reserve = 2,
                1 => changed.required = 6,
                2 => changed.eligible = 11,
                3 => changed.generation = 28,
                4 => changed.tick = 121,
                5 => changed.counters[0] = 81,
                6 => changed.counters[1] = 91,
                7 => changed.counters[2] = 101,
                _ => changed.rows = 5,
            }
            variants.push(changed);
        }
        for field in 0..4 {
            let mut changed = original;
            let foreign = Digest32::of_bytes(b"different independent original input evidence");
            match field {
                0 => changed.source = foreign,
                1 => changed.requirements = foreign,
                2 => changed.capture = foreign,
                _ => changed.assessment = foreign,
            }
            variants.push(changed);
        }
        for changed in variants {
            assert_eq!(InputBinding::parse(&changed.encode())?, changed);
            assert_ne!(changed.witness(), original.witness());
        }
        for malformed in [
            encoded[..399].to_owned(),
            format!("{encoded}0"),
            encoded.to_uppercase(),
            format!(" {encoded}"),
            format!("{encoded}\n"),
            encoded.replacen('0', "g", 1),
        ] {
            assert!(InputBinding::parse(&malformed).is_err());
        }
        Ok(())
    }

    #[test]
    fn input_marker_refuses_shortage_illegal_source_and_unbounded_requirement() {
        let original = binding();
        for mutation in 0..9 {
            let mut bad = original;
            match mutation {
                0 => bad.reserve = 1_000_001,
                1 => bad.required = bad.reserve,
                2 => {
                    bad.required = bad.reserve + 401;
                    bad.eligible = bad.required;
                }
                3 => bad.eligible = bad.required - 1,
                4 => bad.generation = 0,
                5 => bad.generation = u64::MAX,
                6 => bad.tick = dfmcp_adapter::bounded_run::MAX_NATIVE_TICK + 1,
                7 => bad.counters[2] = i32::MAX as u64 + 1,
                _ => bad.rows = 4097,
            }
            assert!(
                InputBinding::parse(&bad.encode()).is_err(),
                "mutation {mutation}"
            );
        }
    }

    #[test]
    fn input_marker_is_ordered_after_both_donors_and_cannot_hide_in_a_body() -> Result<()> {
        use super::super::prerequisite_binding::{self, Origins};
        let input = binding();
        let origin = Digest32::of_bytes(b"completed donor");
        let original = b"unchanged original request bytes".to_vec();
        for workshop in [None, Some(origin)] {
            for manager in [None, Some(origin)] {
                let origins = Origins {
                    workshop,
                    manager,
                    input_supply: Some(input),
                };
                let summary = prerequisite_binding::summary("Furniture goal".into(), origins)?;
                assert_eq!(
                    prerequisite_binding::parse_summary(&summary)?,
                    (origins, "Furniture goal")
                );
                let mut identity = original.clone();
                prerequisite_binding::append_identity(&mut identity, origins)?;
                let mut legacy = original.clone();
                prerequisite_binding::append_identity(
                    &mut legacy,
                    Origins {
                        input_supply: None,
                        ..origins
                    },
                )?;
                assert_ne!(identity, legacy);
                assert!(prerequisite_binding::parse_summary(&format!("Body\n{summary}")).is_err());
                assert!(
                    prerequisite_binding::parse_summary(&format!("{summary}\n{summary}")).is_err()
                );
                assert!(
                    prerequisite_binding::parse_summary(
                        &summary.replace("dfmcp-input-supply/1", "dfmcp-input-supply/2")
                    )
                    .is_err()
                );
                assert!(
                    prerequisite_binding::parse_summary(
                        &summary.replace(&input.encode(), &input.encode().to_uppercase())
                    )
                    .is_err()
                );
            }
        }
        Ok(())
    }
}
