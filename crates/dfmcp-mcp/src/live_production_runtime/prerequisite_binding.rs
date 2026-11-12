//! Closed, ordered original prerequisite envelope. These are host dispatch
//! policies, never canonical world evidence or a certificate of production.
use crate::semantic_workforce_runtime::{digest, invalid};
use dfmcp_core::{Digest32, Result};
use dfmcp_intent::PreparedPlan;

const WORKSHOP_RESERVED: &str = "Requires completed original Carpenter workshop";
const WORKSHOP_PREFIX: &str =
    "Requires completed original Carpenter workshop [dfmcp-workshop-prerequisite/1] ";
const MANAGER_RESERVED: &str = "Requires original production Manager";
const MANAGER_PREFIX: &str = "Requires original production Manager [dfmcp-manager-prerequisite/1] ";
const MANAGER_DOMAIN: &[u8] = b"dfmcp-native-combined-manager-prerequisite/1\0";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Origins {
    pub(crate) workshop: Option<Digest32>,
    pub(crate) manager: Option<Digest32>,
}

impl From<Option<Digest32>> for Origins {
    fn from(workshop: Option<Digest32>) -> Self {
        Self {
            workshop,
            manager: None,
        }
    }
}

/// Preserve every existing byte when manager policy is absent. Both donors are
/// included before deriving intent IDs, keys, created entities and obligations.
pub(super) fn append_identity(bytes: &mut Vec<u8>, origins: Origins) -> Result<()> {
    super::workshop_binding::append_identity(bytes, origins.workshop)?;
    if let Some(manager) = origins.manager {
        if manager == Digest32::ZERO {
            return Err(invalid("manager prerequisite origin witness is reserved"));
        }
        let original = Digest32::of_bytes(bytes);
        bytes.clear();
        bytes.extend_from_slice(MANAGER_DOMAIN);
        bytes.extend_from_slice(original.as_bytes());
        bytes.extend_from_slice(manager.as_bytes());
    }
    Ok(())
}

pub(super) fn summary(body: String, origins: Origins) -> Result<String> {
    if body.is_empty() || reserved(&body) {
        return Err(invalid("empty or repeated production prerequisite summary"));
    }
    let body = match origins.manager {
        Some(manager) => {
            if manager == Digest32::ZERO {
                return Err(invalid("manager prerequisite origin witness is reserved"));
            }
            format!("{MANAGER_PREFIX}{}\n{body}", manager.to_hex())
        }
        None => body,
    };
    super::workshop_binding::summary(body, origins.workshop)
}

fn reserved(value: &str) -> bool {
    value.contains(WORKSHOP_RESERVED)
        || value.contains(MANAGER_RESERVED)
        || value.contains("dfmcp-workshop-prerequisite")
        || value.contains("dfmcp-manager-prerequisite")
}

/// Only workshop then manager then the original body is accepted. A duplicate,
/// reordered, unsupported or hidden marker cannot silently become legacy work.
pub(super) fn parse_summary(summary: &str) -> Result<(Origins, &str)> {
    let mut body = summary;
    let mut origins = Origins::default();
    for (prefix, destination) in [
        (WORKSHOP_PREFIX, &mut origins.workshop),
        (MANAGER_PREFIX, &mut origins.manager),
    ] {
        if let Some(rest) = body.strip_prefix(prefix) {
            let (witness, remainder) = rest
                .split_once('\n')
                .ok_or_else(|| invalid("incomplete production prerequisite marker"))?;
            *destination = Some(digest(witness)?);
            body = remainder;
        }
    }
    if body.is_empty() || reserved(body) {
        return Err(invalid(
            "unsupported, misplaced or repeated production prerequisite marker",
        ));
    }
    Ok((origins, body))
}

fn envelope(plan: &PreparedPlan) -> Result<(Origins, &str)> {
    plan.validate_structure()?;
    if !plan.digest_is_valid() || !plan.id_is_valid() {
        return Err(invalid(
            "production prerequisite requires the sealed original plan",
        ));
    }
    parse_summary(&plan.summary)
}

pub(super) fn requirements(plan: &PreparedPlan) -> Result<Origins> {
    envelope(plan).map(|(origins, _)| origins)
}

pub(super) fn body(plan: &PreparedPlan) -> Result<&str> {
    envelope(plan).map(|(_, body)| body)
}

#[cfg(test)]
pub(super) fn assert_manager_variants(
    legacy: &PreparedPlan,
    compile: impl Fn(Origins) -> Result<PreparedPlan>,
) -> Result<()> {
    use dfmcp_adapter::semantic_work_orders::RetainedOrderObjective;
    use dfmcp_intent::{derive_step_idempotency_key, effects::created_entity_id};
    assert_eq!(&compile(Origins::default())?, legacy);
    let mut identities = vec![legacy.intent_id];
    for workshop in [
        None,
        Some(Digest32::of_bytes(b"original completed workshop")),
    ] {
        for manager in [
            Digest32::of_bytes(b"original manager"),
            Digest32::of_bytes(b"other manager"),
        ] {
            let origins = Origins {
                workshop,
                manager: Some(manager),
            };
            let plan = compile(origins)?;
            assert_eq!(requirements(&plan)?, origins);
            assert_eq!(super::workshop_binding::requirement(&plan)?, workshop);
            assert_eq!(plan.anchor, legacy.anchor);
            assert_eq!(plan.expires_at_tick, legacy.expires_at_tick);
            assert_eq!(plan.steps.len(), legacy.steps.len());
            assert!(!identities.contains(&plan.intent_id));
            identities.push(plan.intent_id);
            dfmcp_adapter::live_production::ProductionLayout::from_plan(&plan)?;
            for (step, prior) in plan.steps.iter().zip(&legacy.steps) {
                assert_eq!(step.id, prior.id);
                assert_eq!(step.action, prior.action);
                assert_eq!(step.depends_on, prior.depends_on);
                assert_ne!(step.idempotency_key, prior.idempotency_key);
                assert_eq!(
                    step.idempotency_key,
                    derive_step_idempotency_key(plan.intent_id, plan.anchor, step.id, &step.action)
                );
                if let Some(obligation) = &step.obligation {
                    let prior = prior
                        .obligation
                        .as_ref()
                        .ok_or_else(|| invalid("original obligation absent"))?;
                    assert_eq!(obligation.deadline_tick, prior.deadline_tick);
                    assert_eq!(obligation.poll_interval_ticks, prior.poll_interval_ticks);
                    assert_eq!(
                        obligation.stable_for_observations,
                        prior.stable_for_observations
                    );
                }
                if RetainedOrderObjective::from_selected_plan(legacy, prior.id).is_some() {
                    let objective = RetainedOrderObjective::from_selected_plan(&plan, step.id)
                        .ok_or_else(|| invalid("manager-bound original lost its order goal"))?;
                    assert_eq!(
                        objective.order_entity_id,
                        created_entity_id(&step.idempotency_key, 0)
                    );
                }
            }
            let limits = dfmcp_adapter::live_production::archive_limits();
            let (bytes, seal) = dfmcp_intent::plan_archive::encode(&plan, limits)?;
            let recovered = dfmcp_intent::plan_archive::decode(&bytes, seal, limits)?;
            assert_eq!(recovered, plan);
            assert_eq!(requirements(&recovered)?, origins);
            let mut dropped = plan.clone();
            dropped.summary = legacy.summary.clone();
            assert!(requirements(&dropped).is_err());
        }
    }
    assert!(
        compile(Origins {
            workshop: None,
            manager: Some(Digest32::ZERO)
        })
        .is_err()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_envelope_preserves_legacy_and_binds_each_exact_donor() -> Result<()> {
        let workshop = Digest32::of_bytes(b"original workshop");
        let manager = Digest32::of_bytes(b"original manager");
        let original = b"original finite production".to_vec();
        let mut keys = Vec::new();
        for origins in [
            Origins::default(),
            Origins::from(Some(workshop)),
            Origins {
                workshop: None,
                manager: Some(manager),
            },
            Origins {
                workshop: Some(workshop),
                manager: Some(manager),
            },
        ] {
            let marked = summary("Original goal".into(), origins)?;
            assert_eq!(parse_summary(&marked)?, (origins, "Original goal"));
            let mut identity = original.clone();
            append_identity(&mut identity, origins)?;
            if origins.manager.is_none() {
                let mut legacy = original.clone();
                super::super::workshop_binding::append_identity(&mut legacy, origins.workshop)?;
                assert_eq!(identity, legacy);
                assert_eq!(
                    marked,
                    super::super::workshop_binding::summary(
                        "Original goal".into(),
                        origins.workshop
                    )?
                );
            }
            assert!(!keys.contains(&identity));
            keys.push(identity);
        }
        Ok(())
    }

    #[test]
    fn malformed_manager_or_mixed_markers_cannot_be_hidden_during_recovery() -> Result<()> {
        let witness = Digest32::of_bytes(b"manager").to_hex();
        let manager = format!("{MANAGER_PREFIX}{witness}\n");
        let workshop = format!("{WORKSHOP_PREFIX}{witness}\n");
        for malformed in [
            format!("{manager}{workshop}Goal"),
            format!("{manager}{manager}Goal"),
            format!("{workshop}{manager}{workshop}Goal"),
            format!("Body\n{manager}Goal"),
            format!(" {manager}Goal"),
            format!("{manager}"),
            format!("{MANAGER_PREFIX}{witness}"),
            format!("{MANAGER_PREFIX}{witness} \nGoal"),
            format!("{MANAGER_PREFIX}{witness}\r\nGoal"),
            format!("{MANAGER_PREFIX}{}\nGoal", "0".repeat(64)),
            format!("{MANAGER_PREFIX}{}\nGoal", witness.to_uppercase()),
            format!("{manager}Goal").replace("prerequisite/1", "prerequisite/2"),
        ] {
            assert!(parse_summary(&malformed).is_err(), "{malformed:?}");
        }
        assert!(summary(format!("{manager}Goal"), Origins::default()).is_err());
        assert!(
            append_identity(
                &mut vec![],
                Origins {
                    workshop: None,
                    manager: Some(Digest32::ZERO)
                }
            )
            .is_err()
        );
        Ok(())
    }
}
