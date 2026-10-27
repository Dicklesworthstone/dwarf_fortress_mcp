//! Public intake, reviewed stale-plan replay and the explicit reserve forecast.

use super::*;

fn reserve_plan(session: &str, minimum: u32, until: u64) -> TestResult<Json> {
    parsed(&facade::fortress_plan(
        Some(session.to_owned()),
        None,
        None,
        None,
        Some(
            json!({
                "template": "production",
                "quotas": [{"item": "DRINK", "minimum": minimum}, {"item": "FOOD", "minimum": 60}],
                "reserve_until_tick": until,
            })
            .to_string(),
        ),
    ))
}

#[test]
fn public_reserve_request_funds_initially_satisfied_quotas_through_future_tick() -> TestResult {
    let session = open("743031")?;
    let planned = reserve_plan(&session, 40, 2401)?;
    assert_eq!(planned["ok"], true, "{planned}");
    assert_eq!(
        planned["production"]["consumption"]["planner"],
        "consumption_reserve_v1"
    );
    assert_eq!(
        planned["production"]["consumption"]["reserve_until_tick"],
        2401
    );
    assert_eq!(
        planned["forecast"]["reserve_forecast"]["predicted_tick"],
        2401
    );
    assert_eq!(
        planned["forecast"]["reserve_forecast"]["predicted_goal_truth"],
        "true"
    );
    {
        let owner = resolve_session(Some(session.clone()))?;
        let guard = owner.lock().map_err(|_| "session poisoned")?;
        let pending = guard.pending.as_ref().ok_or("pending plan missing")?;
        let PlanSource::Production { raw, .. } = &pending.source else {
            return Err("production source missing".into());
        };
        assert_eq!(parsed(raw)?["reserve_until_tick"], 2401);
        assert_eq!(parsed(raw)?["planner"], "consumption_reserve_v1");
        assert!(
            pending
                .plan
                .steps
                .iter()
                .filter_map(|step| step.obligation.as_ref())
                .all(|o| o.deadline_tick.0 < 2401)
        );
    }
    let committed = commit(&session, &planned)?;
    assert_eq!(committed["ok"], true, "{committed}");
    wait(&session, 400)?;
    let future = wait(&session, 2000)?;
    assert!(stock(&session, effects::STOCK_DRINK_FIELD)? >= 40);
    assert!(stock(&session, effects::STOCK_FOOD_FIELD)? >= 60);
    assert_eq!(
        future["objectives"][0]["predicate_truth"], "true",
        "{future}"
    );
    Ok(())
}

#[test]
fn stale_public_reserve_plan_keeps_original_absolute_horizon_and_requires_review() -> TestResult {
    let session = open("743032")?;
    let original = reserve_plan(&session, 60, 2401)?;
    assert_eq!(original["ok"], true, "{original}");
    wait(&session, 1)?;
    let stale = commit(&session, &original)?;
    assert_eq!(stale["ok"], false, "{stale}");
    assert_eq!(stale["rebase"]["method"], "intent_replay");
    assert_ne!(
        stale["rebased_plan"]["plan_digest"],
        original["plan_digest"]
    );
    assert_eq!(
        stale["rebased_plan"]["production"]["consumption"]["reserve_until_tick"],
        2401
    );
    assert_eq!(
        stale["rebased_plan"]["forecast"]["reserve_forecast"]["reserve_until_tick"],
        2401
    );
    assert_eq!(stock(&session, effects::STOCK_DRINK_FIELD)?, 40);
    let reviewed = commit(&session, &stale["rebased_plan"])?;
    assert_eq!(reviewed["ok"], true, "{reviewed}");
    Ok(())
}

#[test]
fn expired_public_reserve_request_never_creates_a_replacement_horizon() -> TestResult {
    let session = open("743033")?;
    wait(&session, 100)?;
    for until in [0, 1, 100, 101, u64::MAX] {
        let result = reserve_plan(&session, 60, until)?;
        assert_eq!(result["ok"], false, "{result}");
        assert!(result["plan_digest"].is_null());
    }
    assert_eq!(stock(&session, effects::STOCK_DRINK_FIELD)?, 40);
    Ok(())
}
