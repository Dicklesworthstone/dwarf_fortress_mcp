//! Sealed policy for this combined host's optional Workshop prerequisite.
//! The marker is not a canonical world predicate and gives generic executors no
//! workshop evidence. This host must verify its association and freshly qualify
//! the original workshop at each protected native work-order boundary.
use crate::semantic_workforce_runtime::{digest, invalid};
use dfmcp_core::{Digest32, Result};
use dfmcp_intent::PreparedPlan;

const RESERVED: &str = "Requires completed original Carpenter workshop";
const PREFIX: &str =
    "Requires completed original Carpenter workshop [dfmcp-workshop-prerequisite/1] ";
const IDENTITY_DOMAIN: &[u8] = b"dfmcp-native-combined-workshop-prerequisite/1\0";

/// None leaves legacy request bytes untouched. The optional policy is bound
/// before any IntentId, step key, created entity or obligation is derived.
pub(super) fn append_identity(bytes: &mut Vec<u8>, origin: Option<Digest32>) -> Result<()> {
    if let Some(origin) = origin {
        if origin == Digest32::ZERO {
            return Err(invalid("workshop prerequisite origin witness is reserved"));
        }
        let original = Digest32::of_bytes(bytes);
        bytes.clear();
        bytes.extend_from_slice(IDENTITY_DOMAIN);
        bytes.extend_from_slice(original.as_bytes());
        bytes.extend_from_slice(origin.as_bytes());
    }
    Ok(())
}

pub(super) fn summary(summary: String, origin: Option<Digest32>) -> Result<String> {
    match origin {
        None => Ok(summary),
        Some(origin) => {
            if origin == Digest32::ZERO || summary.is_empty() || summary.contains(RESERVED) {
                return Err(invalid("invalid or repeated workshop prerequisite marker"));
            }
            Ok(format!("{PREFIX}{}\n{summary}", origin.to_hex()))
        }
    }
}

/// Read the immutable original policy. Removing both configuration and sidecar
/// cannot turn a marked archive into a legacy original under this host.
pub(super) fn requirement(plan: &PreparedPlan) -> Result<Option<Digest32>> {
    plan.validate_structure()?;
    if !plan.digest_is_valid() || !plan.id_is_valid() {
        return Err(invalid(
            "workshop prerequisite requires the sealed original plan",
        ));
    }
    parse_summary(&plan.summary)
}

fn parse_summary(summary: &str) -> Result<Option<Digest32>> {
    if !summary.contains(RESERVED) {
        return Ok(None);
    }
    let (marker, body) = summary
        .split_once('\n')
        .ok_or_else(|| invalid("incomplete workshop prerequisite marker"))?;
    let witness = marker
        .strip_prefix(PREFIX)
        .ok_or_else(|| invalid("unsupported or misplaced workshop prerequisite marker"))?;
    if body.is_empty() || body.contains(RESERVED) {
        return Err(invalid("empty or repeated workshop prerequisite summary"));
    }
    Ok(Some(digest(witness)?))
}

#[cfg(test)]
pub(super) fn assert_compiled_binding(
    legacy: &PreparedPlan,
    first: &PreparedPlan,
    second: &PreparedPlan,
    first_origin: Digest32,
    second_origin: Digest32,
) -> Result<()> {
    use dfmcp_adapter::semantic_work_orders::RetainedOrderObjective;
    use dfmcp_intent::{Action, derive_step_idempotency_key, effects::created_entity_id};
    use dfmcp_world::Predicate;

    fn includes(parent: &Predicate, goal: &Predicate) -> bool {
        if let Predicate::All(children) = goal {
            return children.iter().all(|child| includes(parent, child));
        }
        parent == goal
            || matches!(parent, Predicate::All(children) if children.iter().any(|child| includes(child, goal)))
    }

    assert_eq!(requirement(legacy)?, None);
    for (plan, origin) in [(first, first_origin), (second, second_origin)] {
        assert_eq!(requirement(plan)?, Some(origin));
        assert_ne!(plan.intent_id, legacy.intent_id);
        assert_ne!(plan.id, legacy.id);
        assert_ne!(plan.digest, legacy.digest);
        assert_eq!(plan.anchor, legacy.anchor);
        assert_eq!(plan.expires_at_tick, legacy.expires_at_tick);
        assert_eq!(plan.steps.len(), legacy.steps.len());
        dfmcp_adapter::live_production::ProductionLayout::from_plan(plan)?;
        for (step, old) in plan.steps.iter().zip(&legacy.steps) {
            assert_eq!(step.id, old.id);
            assert_eq!(step.action, old.action);
            assert_eq!(step.depends_on, old.depends_on);
            assert_ne!(step.idempotency_key, old.idempotency_key);
            assert_eq!(
                step.idempotency_key,
                derive_step_idempotency_key(plan.intent_id, plan.anchor, step.id, &step.action)
            );
            if matches!(step.action, Action::CreateWorkOrder { .. }) {
                let obligation = step
                    .obligation
                    .as_ref()
                    .ok_or_else(|| invalid("order obligation missing"))?;
                let old_obligation = old
                    .obligation
                    .as_ref()
                    .ok_or_else(|| invalid("legacy obligation missing"))?;
                assert_eq!(step.postconditions, [obligation.terminal.clone()]);
                assert!(includes(&plan.terminal_condition, &obligation.terminal));
                assert_eq!(obligation.deadline_tick, old_obligation.deadline_tick);
                assert_eq!(
                    obligation.poll_interval_ticks,
                    old_obligation.poll_interval_ticks
                );
                assert_eq!(
                    obligation.stable_for_observations,
                    old_obligation.stable_for_observations
                );
                if RetainedOrderObjective::from_selected_plan(legacy, old.id).is_some() {
                    let goal = RetainedOrderObjective::from_selected_plan(plan, step.id)
                        .ok_or_else(|| invalid("rebound retained order is not recognized"))?;
                    assert_eq!(
                        goal.order_entity_id,
                        created_entity_id(&step.idempotency_key, 0)
                    );
                    assert_ne!(
                        goal.order_entity_id,
                        created_entity_id(&old.idempotency_key, 0)
                    );
                } else {
                    assert_eq!(obligation.terminal, old_obligation.terminal);
                }
            }
        }
        let limits = dfmcp_adapter::live_production::archive_limits();
        let (bytes, seal) = dfmcp_intent::plan_archive::encode(plan, limits)?;
        let decoded = dfmcp_intent::plan_archive::decode(&bytes, seal, limits)?;
        assert_eq!(&decoded, plan);
        assert_eq!(requirement(&decoded)?, Some(origin));
        let mut altered = plan.clone();
        altered.summary = legacy.summary.clone();
        assert!(
            requirement(&altered).is_err(),
            "summary removal invalidates the original seal"
        );
    }
    assert_ne!(first.intent_id, second.intent_id);
    assert_ne!(first.id, second.id);
    assert_ne!(first.digest, second.digest);
    assert!(
        first
            .steps
            .iter()
            .zip(&second.steps)
            .all(|(a, b)| a.idempotency_key != b.idempotency_key)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workshop_identity_preserves_legacy_bytes_and_separates_originals() -> Result<()> {
        let legacy = b"original closed production request".to_vec();
        let mut unchanged = legacy.clone();
        append_identity(&mut unchanged, None)?;
        assert_eq!(unchanged, legacy);
        assert_eq!(summary("Original goal".into(), None)?, "Original goal");
        let origin = Digest32::of_bytes(b"original acknowledged workshop");
        let mut bound = legacy.clone();
        append_identity(&mut bound, Some(origin))?;
        assert_ne!(bound, legacy);
        let mut other = legacy;
        append_identity(&mut other, Some(Digest32::of_bytes(b"another workshop")))?;
        assert_ne!(bound, other);
        assert!(append_identity(&mut other, Some(Digest32::ZERO)).is_err());
        Ok(())
    }

    #[test]
    fn workshop_marker_is_versioned_exact_and_cannot_be_hidden_as_legacy() -> Result<()> {
        let origin = Digest32::of_bytes(b"exact original workshop");
        let marked = summary("Original finite furniture goal".into(), Some(origin))?;
        assert_eq!(parse_summary(&marked)?, Some(origin));
        assert_eq!(parse_summary("Original finite furniture goal")?, None);
        for malformed in [
            format!("{PREFIX}{}", origin.to_hex()),
            format!("{PREFIX}{}\n", origin.to_hex()),
            format!("{PREFIX}{}\r\nOriginal", origin.to_hex()),
            format!("{PREFIX}{} \nOriginal", origin.to_hex()),
            format!("{PREFIX}{}\nOriginal", origin.to_hex().to_uppercase()),
            format!("{PREFIX}{}\nOriginal", "0".repeat(64)),
            marked.replace("prerequisite/1", "prerequisite/2"),
            format!("Other summary\n{marked}"),
            format!(" {marked}"),
            format!("{marked}\n{marked}"),
            format!("{RESERVED} {}\nOriginal", origin.to_hex()),
        ] {
            assert!(parse_summary(&malformed).is_err(), "{malformed:?}");
        }
        assert!(summary(marked, Some(origin)).is_err());
        assert!(summary("Original".into(), Some(Digest32::ZERO)).is_err());
        Ok(())
    }
}
