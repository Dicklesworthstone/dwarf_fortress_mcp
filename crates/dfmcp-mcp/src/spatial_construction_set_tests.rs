// Included alongside the original construction tests to exercise the same
// actual spatial publisher, router and shared condition/watch entry points.
fn whole_set_operations() -> LiveOperationsObservation {
    let mut o = operations();
    let building = o.buildings[0].clone();
    let item = o.items[0].clone();
    o.buildings.clear();
    o.items.clear();
    o.next_item_id = 1000;
    for i in 0..32 {
        let mut b = building.clone();
        b.native_id = 10 + i;
        let mut v = item.clone();
        v.native_id = 100 + i;
        v.holder_building_native_id = Some(b.native_id);
        o.buildings.push(b);
        o.items.push(v);
    }
    o
}
fn whole_set_request(c: &OperationContext) -> Value {
    let mut input = request(c, true);
    input["query"]["monitor"]["mode"] = json!("all_targets");
    input["query"]["targets"] = json!((0..32).map(|i| json!({
        "building_native_id":10+i,"item_native_id":100+i
    })).collect::<Vec<_>>());
    input["query"]["limit"] = json!(1);
    input
}

#[test]
fn whole_set_proposal_registers_one_slot_and_requires_simultaneous_stability() {
    let mut o = whole_set_operations();
    o.buildings[31].build_stage = 0;
    let mut s = state(&o);
    let mut c = context(&s);
    let before = s.snapshot().unwrap().clone();
    let response = query(&s, &c, &whole_set_request(&c));
    let proposal = &response["monitoring"];
    assert_eq!(proposal["available"], true);
    assert_eq!(proposal["requested_targets"], 32);
    assert_eq!(proposal["watch_slots_required"], 1);
    assert_eq!(response["rows"].as_array().unwrap().len(), 1);
    assert!(response["rows"][0].get("monitoring").is_none());
    assert_eq!(s.snapshot().unwrap(), &before);
    let watches = json!({"schema":"dfmcp.query/1","query":{"kind":"watches"}});
    assert_eq!(run(&s, &c, &watches)["records"], json!([]));
    let watch = &proposal["watch_request"];
    assert_eq!(watch["query"]["key"], "bedrooms.all");
    assert_eq!(watch["query"]["condition"]["targets"], watch["query"]["failure_condition"]["targets"]);
    assert_eq!(watch["query"]["condition"]["targets"].as_array().unwrap().len(), 32);
    assert_eq!(evaluate(&s, &c, watch)["evaluation"]["status"], "condition_not_met");
    let registered = run(&s, &c, watch);
    assert_eq!(registered["record"]["status"], "waiting");
    assert_eq!(run(&s, &c, &watches)["records"].as_array().unwrap().len(), 1);
    let poll = json!({"schema":"dfmcp.query/1","query":{"kind":"poll_watch",
        "watch":registered["record"]["watch"]}});
    o.buildings[31].build_stage = 3;
    o.jobs.year_tick += 1;
    s.publish(spatial(&o)).unwrap();
    c.anchor = s.snapshot().unwrap().anchor();
    let candidate = run(&s, &c, &poll);
    assert_eq!(candidate["record"]["status"], "candidate");
    assert_eq!(candidate["record"], run(&s, &c, &poll)["record"]);
    o.buildings[0].build_stage = 0;
    o.jobs.year_tick += 1;
    s.publish(spatial(&o)).unwrap();
    c.anchor = s.snapshot().unwrap().anchor();
    assert_eq!(run(&s, &c, &poll)["record"]["status"], "waiting");
    o.buildings[0].build_stage = 3;
    for expected in ["candidate", "satisfied"] {
        o.jobs.year_tick += 1;
        s.publish(spatial(&o)).unwrap();
        c.anchor = s.snapshot().unwrap().anchor();
        assert_eq!(run(&s, &c, &poll)["record"]["status"], expected);
    }
    close(&c);
}

#[test]
fn whole_set_never_drops_an_unavailable_target_outside_the_visible_page() {
    for case in 0..3 {
        let mut o = whole_set_operations();
        match case {
            0 => { o.buildings.pop(); o.items[31].holder_building_native_id = None; }
            1 => { o.items.pop(); }
            _ => { o.buildings[31].type_key = "Workshop".into(); }
        }
        let s = state(&o);
        let c = context(&s);
        let result = query(&s, &c, &whole_set_request(&c));
        assert_eq!(result["total_rows"], 32);
        assert_eq!(result["rows"][0]["selection"]["building_native_id"], 10);
        assert_eq!(result["monitoring"]["available"], false);
        assert_eq!(result["monitoring"]["unavailable_building_native_ids"], json!([41]));
        assert!(result["monitoring"].get("watch_request").is_none());
        close(&c);
    }
}

#[test]
fn whole_set_pagination_retains_the_identical_complete_proposal() {
    let s = state(&whole_set_operations());
    let c = context(&s);
    let mut input = whole_set_request(&c);
    let first = query(&s, &c, &input);
    let monitoring = first["monitoring"].clone();
    let mut cursor = first["continuation"].clone();
    let mut returned = 1;
    while !cursor.is_null() {
        input["query"]["continuation"] = cursor;
        input["query"]["limit"] = json!(8);
        input["query"]["targets"].as_array_mut().unwrap().reverse();
        let page = query(&s, &c, &input);
        assert_eq!(page["monitoring"], monitoring);
        assert_eq!(page["analysis_digest"], first["analysis_digest"]);
        returned += page["returned"].as_u64().unwrap();
        cursor = page["continuation"].clone();
    }
    assert_eq!(returned, 32);
    for case in 0..3 {
        let mut changed = whole_set_request(&c);
        changed["query"]["continuation"] = first["continuation"].clone();
        match case {
            0 => { changed["query"]["monitor"]["mode"] = Value::Null; }
            1 => { changed["query"]["monitor"]["key_prefix"] = json!("another"); }
            _ => { changed["query"]["monitor"]["poll_interval_ticks"] = json!(2); }
        }
        assert!(super::super::execute(&s, &c, &changed).is_err());
    }
    close(&c);
}

#[test]
fn whole_set_mode_is_opt_in_and_old_proposal_identity_is_unchanged() {
    let s = state(&operations());
    let c = context(&s);
    let old = request(&c, true);
    let expected = query(&s, &c, &old);
    let mut null = old.clone();
    null["query"]["monitor"]["mode"] = Value::Null;
    assert_eq!(query(&s, &c, &null), expected);
    assert!(expected.get("monitoring").is_none());
    assert_eq!(expected["rows"][0]["monitoring"]["available"], true);
    for value in [json!("per_target"), json!("any_target"), json!(true), json!({})] {
        let mut input = old.clone();
        input["query"]["monitor"]["mode"] = value;
        assert!(super::super::execute(&s, &c, &input).is_err());
    }
    close(&c);
}

#[test]
fn whole_set_full_response_refusal_does_not_create_work() {
    let s = state(&whole_set_operations());
    let c = context(&s);
    let input = whole_set_request(&c);
    let result = query(&s, &c, &input);
    let mut small = c.clone();
    small.budget.max_bytes = 256;
    assert!(super::super::execute(&s, &small, &input).is_err());
    assert!(semantic_query::execute_with_publisher(s.snapshot().unwrap(), &c,
        &result["monitoring"]["watch_request"], |_|Err(budget("injected render failure"))).is_err());
    assert_eq!(run(&s, &c, &json!({"schema":"dfmcp.query/1","query":{"kind":"watches"}}))["records"], json!([]));
    let schema = super::super::schema().unwrap();
    let schema = schema["$defs"]["query"]["oneOf"].as_array().unwrap().iter()
        .find(|v|v["properties"]["kind"]["const"]=="construction_progress").unwrap();
    assert_eq!(schema["properties"]["monitor"]["oneOf"][1]["properties"]["mode"]["oneOf"][1]["const"], "all_targets");
    close(&c);
}
