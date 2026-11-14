//! Explicit original-plan sets through the real laboratory facade.
//! Owning implementation bead: df-lab-selected-plan-wait-s4q.1.
//! These regressions distinguish current group truth, exact original endpoints,
//! complete admission and independently durable endpoint publication.

use super::*;
use crate::agent_facade as facade;
use dfmcp_intent::effects;
use dfmcp_world::{Fact, FactSource, Value as WorldValue};
use serde_json::Value;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const EARLY: u64 = 1_234;
const LATE: u64 = 1_511;

struct Owner(String);

impl Drop for Owner {
    fn drop(&mut self) {
        if let Ok(id) = u128::from_str_radix(&self.0, 16) {
            sessions().remove(&SessionId::new(id));
        }
    }
}

struct StateDir(std::path::PathBuf);

impl StateDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-selected-plan-set-{label}-{}",
            std::process::id(),
        ));
        let _ = std::fs::remove_dir_all(&path);
        simulate_durable_restart(Some(path.clone()));
        Self(path)
    }

    fn restart(&self) {
        simulate_durable_restart(Some(self.0.clone()));
    }
}

impl Drop for StateDir {
    fn drop(&mut self) {
        simulate_durable_restart(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn field(value: &Value, key: &str) -> TestResult<String> {
    Ok(value[key]
        .as_str()
        .ok_or_else(|| format!("missing {key}: {value}"))?
        .to_owned())
}

fn open(selector: &str, durable: bool, effects_authority: bool) -> TestResult<Owner> {
    let mut grants = vec![
        ("observe", "read_only"),
        ("query", "read_only"),
        ("control_clock", "reversible"),
    ];
    if effects_authority {
        grants.extend([
            ("plan", "reversible"),
            ("configure_production", "reversible"),
        ]);
    }
    let opened = parsed(&facade::fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(
            grants
                .into_iter()
                .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
                .collect(),
        ),
        Some(60_000),
        Some(4_000),
        Some(32_768),
        Some(4 * 1024 * 1024),
        Some(32_768),
        Some(64),
        Some("starter_fortress".to_owned()),
        Some(false),
        Some(durable),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(Owner(field(&opened, "session_id")?))
}

fn edit<T>(owner: &Owner, body: impl FnOnce(&mut LabSession) -> TestResult<T>) -> TestResult<T> {
    let handle = resolve_session(Some(owner.0.clone()))?;
    with_session(&handle, || Err("fixture session unavailable".into()), body)
}

/// Read the actual final state without introducing a fixture-side publication.
fn view<T>(owner: &Owner, body: impl FnOnce(&LabSession) -> TestResult<T>) -> TestResult<T> {
    let handle = resolve_session(Some(owner.0.clone()))?;
    with_session_read(&handle, |error| Err(error.into()), body)
}

fn snapshot(owner: &Owner) -> TestResult<WorldSnapshot> {
    view(owner, |session| Ok(session.adapter.snapshot().clone()))
}

fn plan(owner: &Owner, actions: Option<Value>, blueprint: Option<Value>) -> TestResult<String> {
    let planned = parsed(&facade::fortress_plan(
        Some(owner.0.clone()),
        None,
        None,
        actions.map(|value| value.to_string()),
        blueprint.map(|value| value.to_string()),
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    field(&planned, "plan_digest")
}

fn commit(owner: &Owner, actions: Option<Value>, blueprint: Option<Value>) -> TestResult<String> {
    let digest = plan(owner, actions, blueprint)?;
    let committed = parsed(&facade::fortress_commit(
        Some(owner.0.clone()),
        digest.clone(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    assert_eq!(committed["plan_digest"], digest);
    Ok(digest)
}

fn production(owner: &Owner, item: &str, minimum: u64, horizon: Option<u64>) -> TestResult<String> {
    let mut request = json!({
        "template":"production",
        "quotas":[{"item":item,"minimum":minimum}],
    });
    if let Some(until) = horizon {
        request["reserve_until_tick"] = json!(until);
    }
    commit(owner, None, Some(request))
}

fn order(name: &str, job: &str, amount: u32) -> Value {
    json!({"kind":"create_work_order","name":name,"job_token":job,"amount":amount})
}

fn regular_wait(owner: &Owner, ticks: u64) -> TestResult<Value> {
    let waited = parsed(&facade::fortress_wait(Some(owner.0.clone()), Some(ticks)))?;
    assert_eq!(waited["ok"], true, "{waited}");
    Ok(waited)
}

fn reach(owner: &Owner, tick: u64) -> TestResult {
    let now = snapshot(owner)?.tick.0;
    assert!(now <= tick);
    regular_wait(owner, tick - now)?;
    assert_eq!(snapshot(owner)?.tick.0, tick);
    Ok(())
}

fn wait_set(
    owner: &Owner,
    digests: &[String],
    policy: Option<&str>,
    ticks: u64,
) -> TestResult<Value> {
    parsed(&facade::fortress_wait_for_plans(
        Some(owner.0.clone()),
        Some(ticks),
        digests.to_vec(),
        policy.map(str::to_owned),
    ))
}

fn successful_wait(
    owner: &Owner,
    digests: &[String],
    policy: Option<&str>,
    ticks: u64,
) -> TestResult<Value> {
    let waited = wait_set(owner, digests, policy, ticks)?;
    assert_eq!(waited["ok"], true, "{waited}");
    assert_eq!(
        waited["selected_plan_set_wait"]["schema"], "dfmcp.selected-plan-set-wait/1",
        "{waited}",
    );
    Ok(waited)
}

fn member<'a>(response: &'a Value, digest: &str) -> TestResult<&'a Value> {
    response["selected_plan_set_wait"]["plans"]
        .as_array()
        .and_then(|plans| plans.iter().find(|plan| plan["plan_digest"] == digest))
        .ok_or_else(|| format!("selected original {digest} missing: {response}").into())
}

fn endpoints(
    owner: &Owner,
    digests: &[String],
) -> TestResult<Vec<Option<(StateAnchor, PredicateTruth)>>> {
    view(owner, |session| {
        digests
            .iter()
            .map(|digest| {
                let selected = Digest32::from_hex(digest).ok_or("invalid fixture digest")?;
                Ok(session
                    .objectives
                    .iter()
                    .find(|original| original.plan_digest == selected)
                    .ok_or("fixture original missing")?
                    .selected_wait_reserve_observation)
            })
            .collect()
    })
}

fn durable_endpoints(owner: &Owner, digests: &[String]) -> TestResult<Vec<Option<StateAnchor>>> {
    let fortress = view(owner, |session| Ok(session.fortress_id))?;
    let selected = digests
        .iter()
        .map(|digest| Digest32::from_hex(digest).ok_or("invalid fixture digest"))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(with_durable_store(|store| {
        Ok(selected
            .iter()
            .map(|digest| store.objective_reserve_observation(fortress, *digest))
            .collect())
    })?)
}

fn stock_at(owner: &Owner, drink: Option<u64>, food: u64, tick: u64) -> TestResult {
    edit(owner, |session| {
        let mut current = session.adapter.snapshot().clone();
        assert!(current.tick.0 <= tick);
        let ledger = effects::stock_ledger(&current).ok_or("stock ledger missing")?;
        let stock = current
            .graph
            .entities
            .get_mut(&ledger)
            .ok_or("stock entity missing")?;
        let fact = |amount| {
            Fact::known(
                WorldValue::U64(amount),
                GameTick(tick),
                FactSource::Replay,
                Digest32::of_bytes(b"selected-plan-set-endpoint-fixture"),
            )
        };
        if let Some(amount) = drink {
            stock
                .fields
                .insert(effects::STOCK_DRINK_FIELD.to_owned(), fact(amount));
        } else {
            stock.fields.remove(effects::STOCK_DRINK_FIELD);
        }
        stock
            .fields
            .insert(effects::STOCK_FOOD_FIELD.to_owned(), fact(food));
        stock.revision += 1;
        current.tick = GameTick(tick);
        current.cursor = current
            .cursor
            .checked_next()
            .ok_or("fixture cursor exhausted")?;
        current.refresh_hash();
        session.adapter.inject_snapshot(current)?;
        Ok(())
    })
}

#[test]
fn one_wait_preserves_two_different_exact_original_reserve_horizons() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("7871101", false, true)?;
    let early = production(&owner, "DRINK", 40, Some(EARLY))?;
    let late = production(&owner, "FOOD", 65, Some(LATE))?;
    let requested = vec![late.clone(), early.clone()];
    let before = snapshot(&owner)?.tick.0;
    let waited = successful_wait(&owner, &requested, None, 2_000)?;
    let group = &waited["selected_plan_set_wait"];
    assert_eq!(group["plan_policy"], "all");
    assert_eq!(group["stop_reason"], "all_complete", "{waited}");
    assert_eq!(group["group_complete"], true, "{waited}");
    assert_eq!(group["inspection_complete"], true);
    assert_eq!(waited["game_tick"], LATE);
    assert_eq!(waited["advanced_game_ticks"], LATE - before);
    for (digest, horizon) in [(&early, EARLY), (&late, LATE)] {
        let original = member(&waited, digest)?;
        assert_eq!(original["complete"], true, "{waited}");
        assert_eq!(original["actions_verified"], true);
        assert_eq!(original["physical_quiescent"], true);
        assert_eq!(original["reserve_frontier"]["truth"], "true");
        assert_eq!(
            original["reserve_frontier"]["observed_anchor"]["game_tick"],
            horizon
        );
        assert_eq!(
            original["reserve_frontier"]["continuous_maintenance_proven"],
            false
        );
    }
    assert_eq!(waited["agent_turn"]["anchor"], waited["observed_anchor"]);
    let mut canonical = requested.clone();
    canonical.sort();
    assert_eq!(group["plan_digests"], json!(canonical));
    assert_eq!(
        endpoints(&owner, &requested)?
            .iter()
            .filter(|value| value.is_some())
            .count(),
        2
    );
    Ok(())
}

#[test]
fn live_and_recovered_originals_share_and_retain_the_same_exact_endpoint() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("mixed");
    let first = open("7871102", true, true)?;
    let recovered = production(&first, "DRINK", 40, Some(EARLY))?;
    regular_wait(&first, 20)?;
    directory.restart();

    let owner = open("7871102", true, true)?;
    let live = production(&owner, "FOOD", 65, Some(EARLY))?;
    let digests = vec![recovered.clone(), live.clone()];
    let waited = successful_wait(&owner, &digests, Some("all"), 2_000)?;
    assert_eq!(
        waited["selected_plan_set_wait"]["stop_reason"], "all_complete",
        "{waited}"
    );
    assert_eq!(waited["game_tick"], EARLY);
    let recovered_result = member(&waited, &recovered)?;
    let live_result = member(&waited, &live)?;
    assert_eq!(
        recovered_result["selection_scope"],
        "exact_recovered_original"
    );
    assert_eq!(recovered_result["owns_original_dispatch"], false);
    assert_eq!(recovered_result["dispatch_handles_restored"], false);
    assert_eq!(live_result["selection_scope"], "exact_original_commit");
    let endpoint = recovered_result["reserve_frontier"]["observed_anchor"].clone();
    assert_eq!(live_result["reserve_frontier"]["observed_anchor"], endpoint);
    for digest in &digests {
        assert_eq!(
            member(&waited, digest)?["reserve_frontier"]["durable"],
            true
        );
        // The published Agent Turn and ordinary objective catalogue must
        // describe the same accepted endpoint as the selected member.
        for catalogue in [
            &waited["objectives"],
            &waited["agent_turn"]["briefing"]["objectives"],
        ] {
            let original = catalogue
                .as_array()
                .and_then(|rows| rows.iter().find(|row| row["plan_digest"] == *digest))
                .ok_or("published original objective missing")?;
            assert_eq!(
                original["retained_reserve_endpoint"]["anchor"], endpoint,
                "{waited}"
            );
            assert_eq!(original["retained_reserve_endpoint"]["truth"], "true");
            assert_eq!(original["retained_reserve_endpoint"]["durable"], true);
        }
    }
    let retained = durable_endpoints(&owner, &digests)?;
    assert!(retained.iter().all(Option::is_some));
    regular_wait(&owner, 1)?;
    directory.restart();

    let reopened = open("7871102", true, false)?;
    let before = snapshot(&reopened)?;
    let repeated = successful_wait(&reopened, &digests, None, 0)?;
    assert_eq!(
        repeated["selected_plan_set_wait"]["stop_reason"], "all_complete",
        "{repeated}"
    );
    assert_eq!(repeated["advanced_game_ticks"], 0);
    assert_eq!(snapshot(&reopened)?, before);
    assert_eq!(durable_endpoints(&reopened, &digests)?, retained);
    for digest in &digests {
        let original = member(&repeated, digest)?;
        assert_eq!(original["owns_original_dispatch"], false);
        assert_eq!(original["reserve_frontier"]["reused_from_prior_wait"], true);
        assert_eq!(original["reserve_frontier"]["observed_anchor"], endpoint);
    }
    Ok(())
}

#[test]
fn any_returns_early_but_all_rechecks_consumable_goals_at_each_current_frontier() -> TestResult {
    let _serial = crate::test_serial();
    for (selector, policy) in [("7871103", "any"), ("7871104", "all")] {
        let owner = open(selector, false, true)?;
        let earlier = production(&owner, "DRINK", 50, None)?;
        let later = production(&owner, "FOOD", 65, Some(EARLY))?;
        let waited = successful_wait(
            &owner,
            &[later.clone(), earlier.clone()],
            Some(policy),
            2_000,
        )?;
        let group = &waited["selected_plan_set_wait"];
        let first = member(&waited, &earlier)?;
        let second = member(&waited, &later)?;
        if policy == "any" {
            assert_eq!(group["stop_reason"], "any_complete", "{waited}");
            assert_eq!(group["group_complete"], true);
            assert_eq!(first["complete"], true);
            assert!(
                waited["game_tick"]
                    .as_u64()
                    .is_some_and(|tick| tick < 1_201)
            );
            assert!(
                group["unfinished_plan_digests"]
                    .as_array()
                    .is_some_and(|values| values.contains(&json!(later)))
            );
        } else {
            // The earlier production completed, then its consumable quota was
            // undone by a meal while the second exact horizon was still future.
            // Historical achievement cannot make both goals currently true.
            assert_eq!(group["stop_reason"], "needs_continuation", "{waited}");
            assert_eq!(group["group_complete"], false);
            assert_eq!(first["goal_truth"], "false");
            assert_eq!(first["actions_verified"], true);
            assert_eq!(first["physical_quiescent"], true);
            assert!(
                waited["game_tick"]
                    .as_u64()
                    .is_some_and(|tick| (1_201..EARLY).contains(&tick))
            );
        }
        assert_eq!(second["reserve_frontier"]["available"], false);
        assert!(
            waited["advanced_game_ticks"]
                .as_u64()
                .is_some_and(|ticks| ticks < 2_000)
        );
    }
    Ok(())
}

#[test]
fn attention_precedes_current_success_under_both_group_policies() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("7871105", false, true)?;
    let good = production(&owner, "DRINK", 50, None)?;
    let complete = parsed(&facade::fortress_wait_for_plan(
        Some(owner.0.clone()),
        Some(1_000),
        Some(good.clone()),
    ))?;
    assert_eq!(
        complete["selected_plan_wait"]["complete"], true,
        "{complete}"
    );
    let cancelled = commit(
        &owner,
        Some(json!([{"action":order("cancelled selected original","PREPARE_MEAL",20)}])),
        None,
    )?;
    let stopped = parsed(&facade::fortress_cancel(
        Some(owner.0.clone()),
        Some("stop_future_steps".to_owned()),
        Some(format!("plan:{cancelled}")),
    ))?;
    assert_eq!(stopped["ok"], true, "{stopped}");
    let before = snapshot(&owner)?;
    for policy in ["any", "all"] {
        let waited = successful_wait(&owner, &[good.clone(), cancelled.clone()], Some(policy), 0)?;
        let group = &waited["selected_plan_set_wait"];
        assert_eq!(group["stop_reason"], "cancelled", "{waited}");
        assert_eq!(group["group_complete"], false);
        assert_eq!(member(&waited, &good)?["complete"], true);
        assert!(group["attention"].as_array().is_some_and(|causes| {
            causes.iter().any(|cause| {
                cause["plan_digest"] == cancelled && cause["stop_reason"] == "cancelled"
            })
        }));
        assert_eq!(waited["advanced_game_ticks"], 0);
        assert_eq!(snapshot(&owner)?, before);
    }
    Ok(())
}

#[test]
fn zero_tick_captures_only_the_selected_endpoints_without_dispatch_or_clock_authority() -> TestResult
{
    let _serial = crate::test_serial();
    let owner = open("7871106", false, true)?;
    let first = production(&owner, "DRINK", 40, Some(EARLY))?;
    let unselected = production(&owner, "FOOD", 65, Some(EARLY))?;
    let digests = vec![first.clone()];
    reach(&owner, EARLY)?;
    let unrelated = commit(
        &owner,
        Some(json!([
            {"action":{"kind":"pause","paused":false}},
            {"action":order("zero tick must not dispatch","BREW_DRINK",20),"depends_on":[0]},
        ])),
        None,
    )?;
    let saved = view(&owner, |session| {
        let states = session
            .last_plan_actions
            .iter()
            .map(|action| {
                (
                    *action,
                    session
                        .adapter
                        .action_receipt(*action)
                        .map(|receipt| receipt.state),
                )
            })
            .collect::<Vec<_>>();
        assert!(
            states
                .iter()
                .any(|(_, state)| *state == Some(CommitState::Prepared))
        );
        Ok((
            states,
            session.last_action,
            session.last_plan_actions.clone(),
            session.commit_receipts[&unrelated].clone(),
        ))
    })?;
    edit(&owner, |session| {
        session
            .grants
            .retain(|grant| grant.capability != Capability::ControlClock);
        Ok(())
    })?;
    let before = snapshot(&owner)?;
    let waited = successful_wait(&owner, &digests, None, 0)?;
    assert_eq!(
        waited["selected_plan_set_wait"]["stop_reason"], "all_complete",
        "{waited}"
    );
    assert_eq!(waited["advanced_game_ticks"], 0);
    assert_eq!(waited["wait_progress"]["deferred_actions_dispatched"], 0);
    assert_eq!(snapshot(&owner)?, before);
    assert!(endpoints(&owner, &digests)?.iter().all(Option::is_some));
    assert_eq!(endpoints(&owner, &[unselected])?, vec![None]);
    view(&owner, |session| {
        for (action, state) in saved.0 {
            assert_eq!(
                session
                    .adapter
                    .action_receipt(action)
                    .map(|receipt| receipt.state),
                state
            );
        }
        assert_eq!(session.last_action, saved.1);
        assert_eq!(session.last_plan_actions, saved.2);
        assert_eq!(session.commit_receipts[&unrelated], saved.3);
        Ok(())
    })?;
    Ok(())
}

#[test]
fn a_later_invalid_member_cannot_publish_an_earlier_exact_endpoint() -> TestResult {
    let _serial = serialized_durable_tests();
    let _directory = StateDir::new("whole-membership");
    let owner = open("7871107", true, true)?;
    let first = production(&owner, "DRINK", 40, Some(EARLY))?;
    let second = production(&owner, "FOOD", 65, Some(EARLY))?;
    let mut digests = vec![first, second];
    digests.sort();
    reach(&owner, EARLY)?;
    let before = snapshot(&owner)?;
    assert_eq!(endpoints(&owner, &digests)?, vec![None, None]);

    let absent = wait_set(&owner, &[digests[0].clone(), "ff".repeat(32)], None, 0)?;
    assert_eq!(absent["ok"], false, "{absent}");
    assert_eq!(snapshot(&owner)?, before);
    assert_eq!(endpoints(&owner, &digests)?, vec![None, None]);
    assert_eq!(durable_endpoints(&owner, &digests)?, vec![None, None]);

    let receipt = edit(&owner, |session| {
        let original = session
            .commit_receipts
            .get(&digests[1])
            .cloned()
            .ok_or("second receipt missing")?;
        session
            .commit_receipts
            .insert(digests[1].clone(), "{}".to_owned());
        Ok(original)
    })?;
    let malformed = wait_set(&owner, &digests, None, 0)?;
    assert_eq!(malformed["ok"], false, "{malformed}");
    assert_eq!(snapshot(&owner)?, before);
    assert_eq!(endpoints(&owner, &digests)?, vec![None, None]);
    assert_eq!(durable_endpoints(&owner, &digests)?, vec![None, None]);
    edit(&owner, |session| {
        session.commit_receipts.insert(digests[1].clone(), receipt);
        Ok(())
    })?;
    assert_eq!(
        successful_wait(&owner, &digests, None, 0)?["selected_plan_set_wait"]["group_complete"],
        true
    );
    Ok(())
}

#[test]
fn complete_authority_source_action_and_output_admission_precede_any_capture() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("7871108", false, true)?;
    let digests = vec![
        production(&owner, "DRINK", 40, Some(EARLY))?,
        production(&owner, "FOOD", 65, Some(EARLY))?,
    ];
    reach(&owner, EARLY)?;
    let before = snapshot(&owner)?;
    let (grants, budget) = view(&owner, |session| {
        Ok((session.grants.clone(), session.budget))
    })?;
    for defect in 0..9 {
        edit(&owner, |session| {
            session.grants = grants.clone();
            session.budget = budget;
            match defect {
                0 => session
                    .grants
                    .retain(|grant| grant.capability != Capability::Observe),
                1 => session
                    .grants
                    .retain(|grant| grant.capability != Capability::Query),
                2 => session
                    .grants
                    .retain(|grant| grant.capability != Capability::ControlClock),
                3 => {
                    for grant in &mut session.grants {
                        if grant.capability == Capability::Query {
                            grant.scope.entity_ids.insert(EntityId::new(1_001));
                        }
                    }
                }
                4 => {
                    for grant in &mut session.grants {
                        if grant.capability == Capability::Query {
                            grant.expires_at_tick = Some(GameTick(EARLY + 2));
                        }
                    }
                }
                5 => session.budget.max_entities = 1,
                6 => session.budget.max_bytes = 1_024,
                7 => session.budget.max_actions = 1,
                _ => session.budget.max_output_tokens = 128,
            }
            Ok(())
        })?;
        let refused = wait_set(&owner, &digests, None, 10)?;
        assert_eq!(refused["ok"], false, "defect {defect}: {refused}");
        assert_eq!(
            refused["error"]["code"],
            if defect < 5 {
                "capability_denied"
            } else {
                "budget_exceeded"
            },
            "defect {defect}: {refused}",
        );
        assert_eq!(snapshot(&owner)?, before, "defect {defect}: {refused}");
        assert_eq!(endpoints(&owner, &digests)?, vec![None, None]);
        if defect == 8 {
            assert!(refused.to_string().len() <= 128 * 4, "{refused}");
        }
    }
    edit(&owner, |session| {
        session.grants = grants;
        session.budget = budget;
        Ok(())
    })?;
    assert_eq!(
        successful_wait(&owner, &digests, None, 0)?["selected_plan_set_wait"]["group_complete"],
        true
    );
    Ok(())
}

#[test]
fn aliases_cannot_count_one_actual_original_twice_or_lose_requested_identity() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("7871109", false, true)?;
    let first = production(&owner, "DRINK", 40, Some(EARLY))?;
    let second = production(&owner, "FOOD", 65, Some(EARLY))?;
    reach(&owner, EARLY)?;
    let alias = Digest32::of_bytes(b"selected-set-reviewed-pre-rebase-alias").to_hex();
    edit(&owner, |session| {
        let receipt = session
            .commit_receipts
            .get(&first)
            .cloned()
            .ok_or("original receipt missing")?;
        session.commit_receipts.insert(alias.clone(), receipt);
        Ok(())
    })?;
    let before = snapshot(&owner)?;
    let duplicate = wait_set(&owner, &[alias.clone(), first.clone()], None, 0)?;
    assert_eq!(duplicate["ok"], false, "{duplicate}");
    assert_eq!(snapshot(&owner)?, before);
    assert_eq!(
        endpoints(&owner, &[first.clone(), second.clone()])?,
        vec![None, None]
    );

    let waited = successful_wait(&owner, &[second.clone(), alias.clone()], None, 0)?;
    assert_eq!(waited["selected_plan_set_wait"]["group_complete"], true);
    assert_eq!(member(&waited, &first)?["requested_plan_digest"], alias);
    assert_eq!(member(&waited, &first)?["plan_digest"], first);
    assert_eq!(member(&waited, &second)?["requested_plan_digest"], second);
    let group = &waited["selected_plan_set_wait"];
    let actual = group["plan_digests"]
        .as_array()
        .ok_or("actual identities missing")?;
    let requested = group["requested_plan_digests"]
        .as_array()
        .ok_or("requested identities missing")?;
    assert_eq!(actual.len(), requested.len());
    for (actual, requested) in actual.iter().zip(requested) {
        assert_eq!(
            member(&waited, actual.as_str().ok_or("invalid actual identity")?)?["requested_plan_digest"],
            *requested
        );
    }
    Ok(())
}

#[test]
fn selected_older_originals_preserve_pending_review_and_latest_unselected_work() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("7871110", false, true)?;
    let first = commit(
        &owner,
        Some(json!([{"action":order("selected brew","BREW_DRINK",1)}])),
        None,
    )?;
    let second = commit(
        &owner,
        Some(json!([{"action":order("selected meals","PREPARE_MEAL",1)}])),
        None,
    )?;
    commit(
        &owner,
        Some(json!([{"action":order("latest unselected long order","BREW_DRINK",20)}])),
        None,
    )?;
    let pending = plan(
        &owner,
        Some(json!([{"action":{"kind":"pause","paused":true}}])),
        None,
    )?;
    let latest = view(&owner, |session| {
        Ok((
            session.last_action.ok_or("latest action missing")?,
            session.last_plan_actions.clone(),
        ))
    })?;
    let waited = successful_wait(&owner, &[first, second], None, 2_000)?;
    assert_eq!(
        waited["selected_plan_set_wait"]["stop_reason"], "all_complete",
        "{waited}"
    );
    assert_eq!(waited["action_id"], latest.0.to_string());
    assert!(
        waited["advanced_game_ticks"]
            .as_u64()
            .is_some_and(|ticks| ticks > 0 && ticks < 1_000)
    );
    view(&owner, |session| {
        assert_eq!(session.last_action, Some(latest.0));
        assert_eq!(session.last_plan_actions, latest.1);
        assert_eq!(
            session
                .pending
                .as_ref()
                .map(|review| review.digest.as_str()),
            Some(pending.as_str())
        );
        assert!(session.open_actions.contains(&latest.0));
        assert!(matches!(
            session.adapter.action_work_state(latest.0)?,
            EffectWorkState::Active { .. }
        ));
        assert_ne!(
            session
                .adapter
                .action_receipt(latest.0)
                .map(|receipt| receipt.state),
            Some(CommitState::Cancelled)
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn first_false_and_unknown_reserve_members_survive_repair_and_restart() -> TestResult {
    let _serial = serialized_durable_tests();
    for (selector, label, stock, truth, reason) in [
        ("7871111", "negative", Some(0), "false", "reserve_unmet"),
        ("7871112", "unknown", None, "unknown", "reserve_unknown"),
    ] {
        let directory = StateDir::new(label);
        let first = open(selector, true, true)?;
        let drink = production(&first, "DRINK", 40, Some(EARLY))?;
        let food = production(&first, "FOOD", 65, Some(EARLY))?;
        let digests = vec![food.clone(), drink.clone()];
        reach(&first, EARLY)?;
        stock_at(&first, stock, 1_000, EARLY)?;
        // Injecting an observation retires volatile adapter handles. Reopen
        // using the archived original custody before any endpoint is selected.
        directory.restart();
        let endpoint_owner = open(selector, true, false)?;
        let observed = successful_wait(&endpoint_owner, &digests, Some("any"), 0)?;
        assert_eq!(
            observed["selected_plan_set_wait"]["stop_reason"], reason,
            "{observed}"
        );
        assert_eq!(observed["selected_plan_set_wait"]["group_complete"], false);
        assert_eq!(member(&observed, &food)?["complete"], true);
        let endpoint = member(&observed, &drink)?["reserve_frontier"].clone();
        assert_eq!(endpoint["truth"], truth);
        assert_eq!(endpoint["durable"], true);
        assert_eq!(endpoint["observed_anchor"]["game_tick"], EARLY);
        assert!(
            durable_endpoints(&endpoint_owner, &digests)?
                .iter()
                .all(Option::is_some)
        );

        stock_at(&endpoint_owner, Some(1_000), 1_000, EARLY + 1)?;
        directory.restart();
        let reopened = open(selector, true, false)?;
        let before = snapshot(&reopened)?;
        let repeated = successful_wait(&reopened, &digests, Some("any"), 100)?;
        assert_eq!(
            repeated["selected_plan_set_wait"]["stop_reason"], reason,
            "{repeated}"
        );
        assert_eq!(repeated["selected_plan_set_wait"]["group_complete"], false);
        let negative = member(&repeated, &drink)?;
        assert_eq!(negative["goal_truth"], "true");
        assert_eq!(negative["reserve_frontier"]["truth"], truth);
        assert_eq!(
            negative["reserve_frontier"]["observed_anchor"],
            endpoint["observed_anchor"]
        );
        assert_eq!(negative["reserve_frontier"]["durable"], true);
        assert_eq!(repeated["advanced_game_ticks"], 0);
        assert_eq!(snapshot(&reopened)?, before);
    }
    Ok(())
}

#[test]
fn endpoint_append_failure_retains_prior_members_and_halts_before_later_time() -> TestResult {
    let _serial = serialized_durable_tests();
    let mut saw_partial_endpoint = false;
    let mut saw_success = false;
    for allowance in (0..=12).chain(std::iter::once(100_000)) {
        let directory = StateDir::new("append-cutoff");
        let first = open("7871113", true, true)?;
        let early_drink = production(&first, "DRINK", 40, Some(EARLY))?;
        let early_food = production(&first, "FOOD", 65, Some(EARLY))?;
        reach(&first, EARLY)?;
        let later_work = commit(
            &first,
            Some(json!([{"action":order("selected work after shared endpoint","BREW_DRINK",20)}])),
            None,
        )?;
        let digests = vec![early_drink.clone(), early_food.clone(), later_work];
        let before = snapshot(&first)?.anchor();
        inject_durable_crash_after(allowance);
        let waited = wait_set(&first, &digests, None, 2_000)?;
        let durable = durable_endpoints(&first, &digests)?;
        let retained = endpoints(&first, &digests)?;
        for (cached, durable) in retained.iter().zip(&durable) {
            assert_eq!(
                cached.map(|(anchor, _)| anchor),
                *durable,
                "allowance {allowance}: {waited}"
            );
        }
        if waited["ok"] == true {
            saw_success = true;
            assert_eq!(
                waited["selected_plan_set_wait"]["stop_reason"], "all_complete",
                "{waited}"
            );
            assert!(
                waited["game_tick"]
                    .as_u64()
                    .is_some_and(|tick| tick > EARLY && tick <= EARLY + 2_000)
            );
            assert!(durable[..2].iter().all(Option::is_some));
            assert!(durable[2].is_none());
            view(&first, |session| {
                assert!(session.durability_fault.is_none(), "{waited}");
                Ok(())
            })?;
        } else if durable.iter().filter(|endpoint| endpoint.is_some()).count() == 1 {
            // Endpoint records are independently durable. Preserve the first
            // accepted original; no rollback or claim of set-atomicity.
            saw_partial_endpoint = true;
            assert_eq!(waited["advanced_game_ticks"], 0, "{waited}");
            assert_eq!(snapshot(&first)?.anchor(), before);
            assert!(
                durable[2].is_none(),
                "an ordinary work plan must not acquire a reserve endpoint"
            );
            assert_ne!(waited["selected_plan_set_wait"]["group_complete"], true);
            directory.restart();
            let reopened = open("7871113", true, false)?;
            assert_eq!(durable_endpoints(&reopened, &digests)?, durable);
            let completed = successful_wait(&reopened, &digests, None, 2_000)?;
            assert_eq!(
                completed["selected_plan_set_wait"]["stop_reason"], "all_complete",
                "{completed}"
            );
            let after = durable_endpoints(&reopened, &digests)?;
            for (prior, current) in durable.iter().zip(&after) {
                if prior.is_some() {
                    assert_eq!(
                        prior, current,
                        "a prior accepted endpoint must remain immutable"
                    );
                }
            }
        }
    }
    assert!(
        saw_partial_endpoint,
        "append cutoffs must cross a real per-original endpoint boundary"
    );
    assert!(
        saw_success,
        "high-allowance control must finish the later selected work"
    );
    Ok(())
}

#[test]
fn replay_retains_the_exact_requested_set_policy_and_early_frontier() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("7871114", false, true)?;
    let first = commit(
        &owner,
        Some(json!([{"action":order("replayed short brew","BREW_DRINK",1)}])),
        None,
    )?;
    let second = commit(
        &owner,
        Some(json!([{"action":order("replayed longer meal order","PREPARE_MEAL",5)}])),
        None,
    )?;
    let requested = vec![second, first];
    let waited = successful_wait(&owner, &requested, Some("any"), 2_000)?;
    assert_eq!(
        waited["selected_plan_set_wait"]["stop_reason"], "any_complete",
        "{waited}"
    );
    let bundle = replay_bundle_for(&owner.0)?;
    let call = bundle["calls"]
        .as_array()
        .and_then(|calls| calls.last())
        .ok_or("wait replay missing")?;
    assert_eq!(call["tool"], "fortress.wait");
    assert_eq!(call["arguments"]["plan_digests"], json!(requested));
    assert_eq!(call["arguments"]["plan_policy"], "any");
    assert_eq!(call["arguments"]["max_game_ticks"], 2_000);
    let report = crate::replay::replay_bundle(&bundle);
    assert_eq!(report["ok"], true, "{report}");
    let replayed = field(&report, "replay_session_id")?;
    assert_eq!(replay_bundle_for(&replayed)?, bundle);
    Ok(())
}

#[test]
fn bounded_set_guidance_keeps_the_full_requested_alias_set_and_policy() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("7871115", false, true)?;
    let first = commit(
        &owner,
        Some(json!([{"action":order("bounded brew","BREW_DRINK",8)}])),
        None,
    )?;
    let second = commit(
        &owner,
        Some(json!([{"action":order("bounded meals","PREPARE_MEAL",8)}])),
        None,
    )?;
    let alias = Digest32::of_bytes(b"bounded-selected-set-reviewed-alias").to_hex();
    edit(&owner, |session| {
        let receipt = session
            .commit_receipts
            .get(&first)
            .cloned()
            .ok_or("original receipt missing")?;
        session.commit_receipts.insert(alias.clone(), receipt);
        Ok(())
    })?;
    let waited = successful_wait(&owner, &[second, alias.clone()], Some("any"), 1)?;
    let group = &waited["selected_plan_set_wait"];
    assert_eq!(group["stop_reason"], "tick_limit", "{waited}");
    assert_eq!(group["group_complete"], false);
    assert_eq!(waited["advanced_game_ticks"], 1);
    assert_eq!(member(&waited, &first)?["requested_plan_digest"], alias);
    let next = waited["agent_turn"]["recommendations"]
        .as_array()
        .and_then(|steps| {
            steps
                .iter()
                .find(|step| step["recommendation_id"] == "continue-selected-plan-set-wait")
        })
        .ok_or_else(|| format!("bounded set continuation guidance missing: {waited}"))?;
    assert_eq!(next["tool"], "fortress.wait");
    assert_eq!(
        next["arguments"]["plan_digests"],
        group["requested_plan_digests"]
    );
    assert_eq!(next["arguments"]["plan_policy"], "any");
    assert!(
        next["arguments"]["max_game_ticks"]
            .as_u64()
            .is_some_and(|ticks| ticks > 0)
    );
    assert!(next["arguments"].get("plan_digest").is_none());
    Ok(())
}
