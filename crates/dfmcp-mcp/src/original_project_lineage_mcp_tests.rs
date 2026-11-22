//! Public complete-project queries reuse separately committed continuation work.

use super::*;

fn continued(selector: &str) -> TestResult<(Owner, String, String)> {
    let owner = open(selector, "starter_fortress_resources", false)?;
    let root = commit(
        &owner,
        None,
        Some(json!({
            "template":"production","quotas":[{"item":"BED","minimum":24}],
            "acquire_resources":{"assign_labor":false},
        })),
    )?;
    let waited = parsed(&facade::fortress_wait(Some(owner.0.clone()), Some(1_000)))?;
    assert_eq!(waited["ok"], true, "{waited}");
    view(&owner, |session| {
        let digest = server::selected_plan_wait::parse_digest(&root)?;
        let original = server::plan_cancellation::select(session, digest)?;
        assert!(original.actions.iter().all(|action| {
            session
                .adapter
                .action_receipt(*action)
                .is_some_and(|receipt| receipt.state == CommitState::Verified)
        }));
        Ok(())
    })?;
    let child = commit(
        &owner,
        None,
        Some(json!({"template":"continue_goal","plan_digest":root})),
    )?;
    Ok((owner, root, child))
}

fn live_fingerprint(owner: &Owner) -> TestResult<String> {
    view(owner, |session| {
        Ok(format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            session.adapter,
            session.commit_receipts,
            session.open_actions,
            session.objectives,
            session.history,
            session
                .pending
                .as_ref()
                .map(|review| (&review.plan, &review.digest, &review.source)),
            session.leases
        ))
    })
}

#[test]
fn public_complete_project_resumes_a_committed_continuation_without_recommit() -> TestResult {
    let _serial = crate::test_serial();
    let (owner, root, child) = continued("8161301")?;
    let pending = plan(&owner, Some(order("preserve unrelated review", 1)), None)?;
    let before = live_fingerprint(&owner)?;
    let initial = complete_query(&owner, std::slice::from_ref(&root), 0)?;
    assert_eq!(initial["ok"], true, "{initial}");
    assert_eq!(initial["prediction"]["all_complete"], false);
    assert_eq!(initial["prediction"]["forecast_work"]["action_polls"], 0);
    let result = complete_query(&owner, std::slice::from_ref(&root), 4_000)?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["schema"], "dfmcp.lab-original-forecast/2");
    assert_eq!(result["prediction"]["all_complete"], true, "{result}");
    assert_eq!(
        result["prediction"]["forecast_work"]["hypothetical_stages"],
        0
    );
    assert_eq!(
        result["prediction"]["forecast_work"]["retained_continuation_stages"],
        1
    );
    assert_eq!(
        result["next_step"]["arguments"]["plan_digests"],
        json!([child])
    );
    let retained = &result["prediction"]["originals"][0]["retained_stages"][0];
    assert_eq!(retained["actual_plan_digest"], child);
    assert_eq!(retained["parent_plan_digest"], root);
    assert_eq!(retained["custody"], "already_committed_original");
    assert_eq!(retained["recommitted_by_forecast"], false);
    assert_eq!(retained["predicted_actions_complete"], true);
    let real_ids = view(&owner, |session| {
        let digest = Digest32::from_hex(&child).ok_or("child digest missing")?;
        Ok(server::plan_cancellation::select(session, digest)?
            .actions
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>())
    })?;
    assert_eq!(
        retained["steps"]
            .as_array()
            .ok_or("retained steps missing")?
            .iter()
            .filter_map(|step| step["action_id"].as_str())
            .collect::<Vec<_>>(),
        real_ids
    );
    assert_eq!(
        result["agent_turn"]["coverage"]["eligible_as_mutation_evidence"],
        false
    );
    assert_eq!(live_fingerprint(&owner)?, before);
    assert_eq!(
        view(&owner, |session| Ok(session
            .pending
            .as_ref()
            .map(|review| review.plan.digest.to_hex())))?,
        Some(pending)
    );
    Ok(())
}

#[test]
fn root_aliases_work_but_selected_descendants_cannot_double_count_a_project() -> TestResult {
    let _serial = crate::test_serial();
    let (owner, root, child) = continued("8161302")?;
    let alias = Digest32::of_bytes(b"retained-project-root-alias").to_hex();
    edit(&owner, |session| {
        let receipt = session
            .commit_receipts
            .get(&root)
            .ok_or("root receipt missing")?
            .clone();
        session.commit_receipts.insert(alias.clone(), receipt);
        Ok(())
    })?;
    let result = complete_query(&owner, std::slice::from_ref(&alias), 4_000)?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(
        result["prediction"]["originals"][0]["requested_plan_digest"],
        alias
    );
    assert_eq!(
        result["prediction"]["originals"][0]["actual_plan_digest"],
        root
    );
    for selection in [vec![root.clone(), child.clone()], vec![child.clone()]] {
        let rejected = complete_query(&owner, &selection, 4_000)?;
        assert_eq!(rejected["ok"], false, "{rejected}");
        assert_eq!(rejected["error"]["code"], "invalid_request");
    }
    let default = query(&owner, std::slice::from_ref(&root), 4_000)?;
    let explicit = parsed(&facade::fortress_query(
        Some(owner.0.clone()),
        Some(
            json!({
                "mode":"forecast_originals","plan_digests":[root],"horizon_ticks":4_000,
                "forecast_scope":"current_stage",
            })
            .to_string(),
        ),
    ))?;
    assert_eq!(default["schema"], "dfmcp.lab-original-forecast/1");
    assert_eq!(default["prediction"], explicit["prediction"]);
    assert!(
        default["prediction"]["originals"][0]
            .get("retained_stages")
            .is_none()
    );
    assert_eq!(query(&owner, &[child], 4_000)?["ok"], true);
    Ok(())
}

#[test]
fn every_retained_stage_is_admitted_before_the_private_model() -> TestResult {
    let _serial = crate::test_serial();
    for case in 0..7 {
        let (owner, root, child) = continued(&format!("81613{}", 10 + case))?;
        edit(&owner, |session| {
            let digest = Digest32::from_hex(&child).ok_or("child digest invalid")?;
            let actions = server::plan_cancellation::select(session, digest)?.actions;
            match case {
                0 => {
                    session.commit_receipts.remove(&child);
                }
                1 => {
                    let raw = session
                        .commit_receipts
                        .get(&child)
                        .ok_or("child receipt missing")?;
                    let mut receipt: Value = serde_json::from_str(raw)?;
                    receipt["session_id"] = json!("wrong-owner");
                    session
                        .commit_receipts
                        .insert(child.clone(), receipt.to_string());
                }
                2 => {
                    let sealed = session
                        .objectives
                        .iter()
                        .find(|objective| objective.plan_digest == digest)
                        .and_then(|objective| objective.resource_plan())
                        .ok_or("child plan missing")?
                        .anchor
                        .state_hash;
                    session.history.remove(&sealed);
                }
                3 => {
                    session.open_actions.retain(|id| !actions.contains(id));
                }
                4 => {
                    session.budget.max_actions = 1;
                }
                5 => {
                    session.budget.max_output_tokens = 600;
                }
                6 => {
                    session
                        .grants
                        .retain(|grant| grant.capability != Capability::Query);
                }
                _ => return Err("unexpected admission case".into()),
            }
            Ok(())
        })?;
        let before = live_fingerprint(&owner)?;
        let rejected = complete_query(&owner, &[root], 4_000)?;
        assert_eq!(rejected["ok"], false, "case {case}: {rejected}");
        assert!(rejected["prediction"].is_null(), "{rejected}");
        if case == 4 || case == 5 {
            assert_eq!(rejected["error"]["code"], "budget_exceeded", "{rejected}");
        }
        if case == 6 {
            assert_eq!(rejected["error"]["code"], "capability_denied", "{rejected}");
        }
        assert_eq!(live_fingerprint(&owner)?, before);
    }
    Ok(())
}

#[test]
fn interrupted_continuation_is_not_silently_replaced_by_a_hypothetical_stage() -> TestResult {
    let _serial = crate::test_serial();
    let (owner, root, child) = continued("8161303")?;
    let cancelled = parsed(&facade::fortress_cancel(
        Some(owner.0.clone()),
        Some("stop_future_steps".to_owned()),
        Some(format!("plan:{child}")),
    ))?;
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    let before = live_fingerprint(&owner)?;
    let rejected = complete_query(&owner, &[root], 4_000)?;
    assert_eq!(rejected["ok"], false, "{rejected}");
    assert_eq!(rejected["error"]["code"], "conflict");
    assert!(rejected["prediction"].is_null());
    assert_eq!(live_fingerprint(&owner)?, before);
    Ok(())
}
