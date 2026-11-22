//! Malformed retained lineage cannot become predicted replacement authority.

use super::super::super as server;
use super::*;
use crate::agent_facade as facade;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Owner(String);

impl Drop for Owner {
    fn drop(&mut self) {
        if let Ok(id) = u128::from_str_radix(&self.0, 16) {
            server::sessions().remove(&SessionId::new(id));
        }
    }
}

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn text(value: &Value, key: &str) -> TestResult<String> {
    Ok(value[key]
        .as_str()
        .ok_or_else(|| format!("missing {key}: {value}"))?
        .to_owned())
}

fn prepared(owner: &Owner, request: Value) -> TestResult<Digest32> {
    let planned = parsed(&facade::fortress_plan(
        Some(owner.0.clone()),
        None,
        None,
        None,
        Some(request.to_string()),
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = text(&planned, "plan_digest")?;
    let committed = parsed(&facade::fortress_commit(Some(owner.0.clone()), digest))?;
    assert_eq!(committed["ok"], true, "{committed}");
    Digest32::from_hex(&text(&committed, "plan_digest")?).ok_or_else(|| "bad fixture digest".into())
}

fn fixture(selector: &str) -> TestResult<(Owner, Digest32, Digest32)> {
    let opened = parsed(&facade::fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "read_only"),
                ("configure_production", "reversible"),
                ("designate", "guarded"),
                ("checkpoint", "guarded"),
                ("control_clock", "reversible"),
            ]
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
        Some("starter_fortress_resources".to_owned()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    let owner = Owner(text(&opened, "session_id")?);
    let root = prepared(
        &owner,
        json!({"template":"production","quotas":[{"item":"BED","minimum":24}],
        "acquire_resources":{"assign_labor":false}}),
    )?;
    let waited = parsed(&facade::fortress_wait(Some(owner.0.clone()), Some(1_000)))?;
    assert_eq!(waited["ok"], true, "{waited}");
    let child = prepared(
        &owner,
        json!({"template":"continue_goal","plan_digest":root.to_hex()}),
    )?;
    Ok((owner, root, child))
}

fn inspect<T>(owner: &Owner, body: impl FnOnce(&mut LabSession) -> TestResult<T>) -> TestResult<T> {
    let handle = server::resolve_session(Some(owner.0.clone()))?;
    server::with_session(&handle, || Err("fixture session unavailable".into()), body)
}

#[test]
fn lineage_order_comes_from_exact_parents_and_preserves_the_root_request() -> TestResult {
    let _serial = crate::test_serial();
    let (owner, root, child) = fixture("8161401")?;
    inspect(&owner, |session| {
        session.objectives.reverse();
        let original = session
            .objectives
            .iter()
            .find(|objective| objective.plan_digest == root)
            .ok_or("root disappeared")?;
        let (project, chain) = source(original, session)?;
        assert_eq!(chain, vec![(child, root)]);
        let PlanSource::Production { raw, .. } = &original.source else {
            return Err("bad fixture root".into());
        };
        assert_eq!(project, ProjectSource::parse_production(raw)?);
        assert!(
            source(
                session
                    .objectives
                    .iter()
                    .find(|objective| objective.plan_digest == child)
                    .ok_or("child disappeared")?,
                session
            )
            .is_err()
        );
        Ok(())
    })
}

#[test]
fn gaps_cycles_changed_requests_and_nonlive_lineage_are_refused() -> TestResult {
    let _serial = crate::test_serial();
    let (owner, root, child) = fixture("8161402")?;
    inspect(&owner, |session| {
        let retained = session.objectives.clone();
        let original = retained
            .iter()
            .find(|objective| objective.plan_digest == root)
            .ok_or("root disappeared")?
            .clone();
        let child_index = retained
            .iter()
            .position(|objective| objective.plan_digest == child)
            .ok_or("child disappeared")?;
        let before = format!("{:?}", session.adapter);
        for case in 0..11 {
            session.objectives = retained.clone();
            let objective = &mut session.objectives[child_index];
            match case {
                0 | 1 | 2 => {
                    let PlanSource::ProductionContinuation { raw, .. } = &mut objective.source
                    else {
                        return Err("bad fixture child".into());
                    };
                    let mut value: Value = serde_json::from_str(raw)?;
                    match case {
                        0 => {
                            value["parent_plan_digest"] =
                                json!(Digest32::of_bytes(b"missing predecessor").to_hex())
                        }
                        1 => value["parent_plan_digest"] = json!(child.to_hex()),
                        2 => value["production"]["quotas"][0]["minimum"] = json!(25),
                        _ => return Err("bad mutation case".into()),
                    }
                    *raw = value.to_string();
                }
                3 => objective.owner_session_id = SessionId::new(81_999),
                4 => objective.originating_process = false,
                5 => objective.original_handles_live = false,
                6 => objective.restore_abandoned_anchor = Some(session.adapter.snapshot().anchor()),
                7 => objective.verification_error = Some("missing archived proof".to_owned()),
                8 => {
                    let duplicate = objective.clone();
                    session.objectives.push(duplicate);
                }
                9 => {
                    let mut branch = objective.clone();
                    branch.plan_digest = Digest32::of_bytes(b"second pursuit of one parent");
                    session.objectives.push(branch);
                }
                10 => {
                    let plan = objective.plan.as_mut().ok_or("child plan missing")?;
                    plan.anchor.cursor.epoch += 1;
                }
                _ => return Err("bad mutation case".into()),
            }
            let failure = source(&original, session)
                .err()
                .ok_or_else(|| format!("lineage mutation {case} was accepted"))?;
            assert_eq!(
                failure.code,
                ErrorCode::Conflict,
                "case {case}: {failure:?}"
            );
            assert_eq!(format!("{:?}", session.adapter), before);
        }
        session.objectives = retained;
        Ok(())
    })
}

#[test]
fn committed_stage_limit_bounds_lineage_admission_before_model_work() -> TestResult {
    let _serial = crate::test_serial();
    let (owner, root, child) = fixture("8161403")?;
    inspect(&owner, |session| {
        let retained = session.objectives.clone();
        let original = retained
            .iter()
            .find(|objective| objective.plan_digest == root)
            .ok_or("root disappeared")?
            .clone();
        let child = retained
            .iter()
            .find(|objective| objective.plan_digest == child)
            .ok_or("child disappeared")?
            .clone();
        let mut parent = child.plan_digest;
        for index in 2..=MAX_COMMITTED_PROJECT_STAGES {
            let mut extra = child.clone();
            extra.plan_digest = Digest32::of_bytes(format!("over-bound-stage-{index}").as_bytes());
            let PlanSource::ProductionContinuation { raw, .. } = &mut extra.source else {
                return Err("bad child source".into());
            };
            let mut value: Value = serde_json::from_str(raw)?;
            value["parent_plan_digest"] = json!(parent.to_hex());
            *raw = value.to_string();
            parent = extra.plan_digest;
            session.objectives.push(extra);
        }
        let result = source(&original, session);
        session.objectives = retained;
        assert_eq!(
            result.err().ok_or("over-bound lineage accepted")?.code,
            ErrorCode::BudgetExceeded
        );
        Ok(())
    })
}
