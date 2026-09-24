//! Reuse the real private-file/capture fixtures from spatial_watch_batch_tests.
use super::*;

fn definitions() -> Vec<Value> {
    ["material-stock", "safe-pause"]
        .into_iter()
        .map(|key| {
            json!({
                "key":key,"condition":{"op":"paused","value":true},
                "deadline_tick":105u64*403200+100,"stable_observations":2
            })
        })
        .collect()
}
fn install(s: &Registered, watches: Vec<Value>) -> Result<Value> {
    ask(s, json!({"kind":"register_watches","watches":watches}))
}
fn follow(s: &Registered, response: &Value) -> Result<Value> {
    let query = response["next_step"]["arguments"]["query"].clone();
    assert!(query.is_object());
    decode(&fortress_query(s.handle(), None, Some(query)))
}

#[test]
fn registration_and_followup_each_publish_one_complete_checkpoint() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, 3, &[4], false)?;
    let before = ask(&s, json!({"kind":"watches"}))?;
    let observations = fs::read(&files.observations).map_err(io_error)?;
    let installed = install(&s, definitions())?;
    require_success(&installed)?;
    assert_eq!(installed["created"], 2);
    assert_eq!(installed["replayed"], 0);
    assert_eq!(installed["durable"], true);
    assert_eq!(installed["atomic_watch_publication"], true);
    assert_eq!(checkpoint(&installed)?, checkpoint(&before)? + 1);
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    for row in installed["records"]
        .as_array()
        .ok_or_else(|| io_error(std::io::Error::other("rows")))?
    {
        assert_eq!(row["last_evaluated_anchor"], installed["anchor"]);
        assert_eq!(row["sample_count"], 1);
    }
    assert_eq!(
        fs::read(&files.observations).map_err(io_error)?,
        observations
    );
    let bytes = fs::read(&files.watches).map_err(io_error)?;
    let mut reordered = definitions();
    reordered.reverse();
    let replay = install(&s, reordered)?;
    require_success(&replay)?;
    assert_eq!(replay["created"], 0);
    assert_eq!(replay["replayed"], 2);
    assert_eq!(
        replay["configuration_digest"],
        installed["configuration_digest"]
    );
    assert_eq!(checkpoint(&replay)?, checkpoint(&installed)?);
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, bytes);
    let sampled = follow(&s, &replay)?;
    require_success(&sampled)?;
    assert_eq!(sampled["sampled"], 2);
    assert_eq!(sampled["all_satisfied"], true);
    assert_eq!(checkpoint(&sampled)?, checkpoint(&installed)? + 1);
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn conflicts_and_response_refusal_leave_both_archives_and_the_watch_set_unchanged() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, 3, &[], false)?;
    let existing = definitions()[0].clone();
    require_success(&install(&s, vec![existing.clone()])?)?;
    let before = fs::read(&files.watches).map_err(io_error)?;
    let observations = fs::read(&files.observations).map_err(io_error)?;
    let mut changed = existing;
    changed["stable_observations"] = json!(3);
    let conflict = install(&s, vec![definitions()[1].clone(), changed])?;
    assert_eq!(conflict["error"]["code"], "conflict");
    {
        let h = resolve(s.handle())?;
        lock(&h)?.budget.max_output_tokens = 64;
    }
    let refused = install(&s, definitions())?;
    assert_eq!(refused["error"]["code"], "budget_exceeded");
    {
        let h = resolve(s.handle())?;
        lock(&h)?.budget.max_output_tokens = 65536;
    }
    let retained = ask(&s, json!({"kind":"watches"}))?;
    require_success(&retained)?;
    assert_eq!(retained["records"].as_array().map(Vec::len), Some(1));
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, before);
    assert_eq!(
        fs::read(&files.observations).map_err(io_error)?,
        observations
    );
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn durable_retry_recovers_new_handles_without_sampling_or_resetting_definitions() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, 3, &[], false)?;
    let first = install(&s, definitions())?;
    require_success(&first)?;
    drop(s);
    let s = register(&files, 4, &[5, 6], false)?;
    let before = fs::read(&files.watches).map_err(io_error)?;
    let replay = install(&s, definitions())?;
    require_success(&replay)?;
    assert_eq!(
        replay["configuration_digest"],
        first["configuration_digest"]
    );
    assert_eq!(replay["created"], 0);
    assert_eq!(replay["replayed"], 2);
    for index in 0..2 {
        assert_ne!(
            replay["records"][index]["watch"],
            first["records"][index]["watch"]
        );
        assert_eq!(replay["records"][index]["fresh_observation_required"], true);
        assert_eq!(replay["records"][index]["stable_observations"], 0);
    }
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, before);
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    let fresh = follow(&s, &replay)?;
    require_success(&fresh)?;
    assert_eq!(fresh["all_satisfied"], false);
    let done = follow(&s, &fresh)?;
    require_success(&done)?;
    assert_eq!(done["all_satisfied"], true);
    assert_eq!(s.calls.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn registration_requires_query_but_does_not_acquire_observe_authority() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, 3, &[4], false)?;
    {
        let h = resolve(s.handle())?;
        lock(&h)?
            .grants
            .retain(|g| g.capability == Capability::Query);
    }
    let installed = install(&s, definitions())?;
    require_success(&installed)?;
    let before = fs::read(&files.watches).map_err(io_error)?;
    assert_eq!(
        follow(&s, &installed)?["error"]["code"],
        "capability_denied"
    );
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    {
        let h = resolve(s.handle())?;
        lock(&h)?.grants.clear();
    }
    assert_eq!(
        install(&s, definitions())?["error"]["code"],
        "capability_denied"
    );
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, before);
    Ok(())
}

#[test]
fn live_schema_reuses_watch_definitions_and_archive_reads_cannot_register() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, 3, &[], false)?;
    let result = decode(&fortress_query(s.handle(), Some("schema".into()), None))?;
    require_success(&result)?;
    let schema = &result["query_schema"];
    let variants = schema["$defs"]["query"]["oneOf"]
        .as_array()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "variants"))?;
    assert_eq!(
        variants
            .iter()
            .filter(|v| v["properties"]["kind"]["const"] == "register_watches")
            .count(),
        1
    );
    let mut expected = variants
        .iter()
        .find(|v| v["properties"]["kind"]["const"] == "watch")
        .cloned()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "watch schema"))?;
    expected["properties"]
        .as_object_mut()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "properties"))?
        .remove("kind");
    expected["required"]
        .as_array_mut()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "required"))?
        .retain(|v| v != "kind");
    assert_eq!(schema["$defs"]["watch_set_member"], expected);
    let history = ask(&s, json!({"kind":"history"}))?;
    let entry = &history["rows"][0];
    assert_eq!(
        ask(
            &s,
            json!({"kind":"historical_query","record":entry["record"],"record_digest":entry["record_digest"],
        "query":{"kind":"register_watches","watches":definitions()}})
        )?["ok"],
        false
    );
    let (limits, budget) = {
        let h = resolve(s.handle())?;
        let session = lock(&h)?;
        (session.limits, session.budget)
    };
    drop(s);
    let mut session = archive::open(
        next_id()?,
        Slot::reserve()?,
        &files.observations,
        limits,
        budget,
        &[Capability::Query],
    )?;
    let c = session.context()?;
    let request = json!({"schema":"dfmcp.query/1","query":{"kind":"register_watches","watches":definitions()}});
    assert!(
        matches!(execute(&mut session,&c,&request),Err(e)if e.code==ErrorCode::CapabilityDenied)
    );
    assert!(archive::query(&mut session, &c, &request, false).is_err());
    Ok(())
}

#[test]
fn corrupt_watch_storage_prevents_partial_installation_without_any_capture() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, 3, &[], false)?;
    fs::OpenOptions::new()
        .append(true)
        .open(&files.watches)
        .and_then(|mut f| f.write_all(b"x"))
        .map_err(io_error)?;
    let before = fs::read(&files.watches).map_err(io_error)?;
    assert_eq!(
        install(&s, definitions())?["error"]["code"],
        "corrupt_ledger"
    );
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, before);
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}
