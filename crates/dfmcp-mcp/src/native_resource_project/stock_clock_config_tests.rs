//! Closed opt-in parsing and independent native clock authority.
use super::*;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parent_input() -> Value {
    json!({
        "schema":"dfmcp.native-resource-project-development/1",
        "directory":"/custody/resource-project",
        "initialize":true,
        "goal":{
            "recipe":"wooden_bed","minimum_stack_units":2,"max_order_amount":2,
            "deadline_tick":1000,"poll_interval_ticks":10,"stable_samples":2,
            "assign_labor":true
        }
    })
}

#[test]
fn stock_watch_clock_is_optional_but_never_accepts_null_or_an_open_extension() -> TestResult {
    let legacy: Input = serde_json::from_value(parent_input())?;
    assert!(!legacy.stock_watch_clock);
    for enabled in [false, true] {
        let mut value = parent_input();
        value["stock_watch_clock"] = json!(enabled);
        let input: Input = serde_json::from_value(value)?;
        assert_eq!(input.stock_watch_clock, enabled);
        input.goal.validate()?;
    }
    for invalid in [
        Value::Null,
        json!(0),
        json!(1),
        json!("true"),
        json!({}),
        json!([]),
    ] {
        let mut value = parent_input();
        value["stock_watch_clock"] = invalid;
        assert!(serde_json::from_value::<Input>(value).is_err());
    }
    for field in [
        "stock_clock_journal",
        "stock_clock_path",
        "automatic_stock_clock",
    ] {
        let mut value = parent_input();
        value["stock_watch_clock"] = json!(true);
        value[field] = json!(true);
        assert!(serde_json::from_value::<Input>(value).is_err(), "{field}");
    }
    Ok(())
}

#[test]
fn stock_watch_opt_in_does_not_grant_an_unconfigured_native_clock_family() -> TestResult {
    for enabled in [false, true] {
        for configured in [false, true] {
            let parent = json!({"stock_watch_clock":enabled}).to_string();
            let production = if configured {
                json!({"inventory_clock":{"generation":7}})
            } else {
                json!({})
            }
            .to_string();
            let features = Features::from_raw("{}", &production, Some(&parent), None)?;
            for key in [
                "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK",
                "DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN",
            ] {
                assert_eq!(features.allows(key), configured, "{key}");
            }
            for key in [
                "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_CLOCK",
                "DFMCP_SEMANTIC_WORK_ORDER_CLOCK_TOKEN",
                "DFMCP_RUN_ALLOW_CLOCK",
                "DFMCP_RUN_TOKEN",
            ] {
                assert!(!features.allows(key), "{key}");
            }
        }
    }
    Ok(())
}
