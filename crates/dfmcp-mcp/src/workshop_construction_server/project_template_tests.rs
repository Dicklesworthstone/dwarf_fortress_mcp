//! Pure preflight and closed historical-descriptor cases. Activation still has
//! to pass the real Offline journal/ACK importer before any native read.
use super::*;
use dfmcp_adapter::tree_harvest::{Capture, Plan, Progress};

fn vector(name: &str) -> Result<Vec<u8>> {
    let vectors: Value = serde_json::from_str(include_str!(
        "../../../../tests/native/tree_harvest/vectors.json"
    ))
    .map_err(|_| invalid("Tree vectors"))?;
    vectors[name]
        .as_str()
        .ok_or_else(|| invalid("Tree vector absent"))?
        .as_bytes()
        .chunks_exact(2)
        .map(|v| {
            u8::from_str_radix(std::str::from_utf8(v).map_err(|_| invalid("hex"))?, 16)
                .map_err(|_| invalid("hex"))
        })
        .collect()
}
fn input() -> Result<Value> {
    let cap = Capture::decode(&vector("capture")?)?;
    let d = cap.dimensions;
    Ok(
        json!({"schema":SCHEMA,"directory":"/tmp/project-workshop-template",
        "endpoint":"127.0.0.1:5001","initialize":true,"acknowledged":null,
        "world_folder":cap.source.folder(),"site_id":cap.source.site(),
        "df_version":"test-df","dfhack_version":"test-dfhack","generation":cap.generation+7,
        "map_dimensions":d,"center":[25,25,2],"scope":[0,0,0,d[0]-1,d[1]-1,d[2]-1],
        "protected":[],"authority_expires_tick":cap.tick+20000,"lease_ticks":1200,
        "checkpoint":"disposable-fortress-no-checkpoint"}),
    )
}
fn template() -> Result<ProjectWorkshopTemplate> {
    let cap = Capture::decode(&vector("capture")?)?;
    let mut config = Config::parse_project_template(&input()?.to_string(), true)?;
    config.parent_check = Some(Arc::new(|| Ok(())));
    let binding = TreeBinding {
        endpoint: "127.0.0.1:5000".parse().map_err(|_| invalid("endpoint"))?,
        source: cap.source.clone(),
        generation: cap.generation,
        dimensions: cap.dimensions,
        scope: cap.effect_scope()?,
        df_version: "test-df".into(),
        dfhack_version: "test-dfhack".into(),
    };
    let expected = ExpectedTree::new(
        &config,
        Path::new("/tmp/project-tree-template"),
        &binding,
        cap.root,
        cap.tick + 12000,
    )?;
    ProjectWorkshopTemplate {
        config,
        tree: Some(expected),
    }
    .with_project_deadline(cap.tick + 12000)
}
fn historical_reference(template: &ProjectWorkshopTemplate) -> Result<TreeProjectHandoff> {
    let cap = Capture::decode(&vector("capture")?)?;
    let plan = Plan::new("tree-001", cap.clone())?;
    let mut raw = vector("progress_supply")?;
    raw[24..32].copy_from_slice(&(cap.tick + 10).to_be_bytes());
    let progress = Progress::decode(&raw, &plan)?;
    let tree = template.tree.as_ref().ok_or_else(exhausted)?;
    let b = &tree.binding;
    // These pins are deliberately only descriptor syntax. No test treats them
    // as an acknowledged native origin or calls a native source with them.
    TreeProjectHandoff::from_json(
        &json!({"schema":"dfmcp.tree-project-handoff/1","directory":tree.directory,
        "binding":{"endpoint":b.endpoint.to_string(),"world_folder":b.source.folder(),"site_id":b.source.site(),
            "df_version":b.df_version,"dfhack_version":b.dfhack_version,"generation":b.generation,
            "map_dimensions":b.dimensions,"scope":[b.scope.min.x,b.scope.min.y,b.scope.min.z,b.scope.max.x,b.scope.max.y,b.scope.max.z]},
        "root":tree.root,"acknowledged":{"journal_id":"a".repeat(64),"head":"b".repeat(64),"frames":6},
        "original_key":plan.key(),"original_plan_digest":plan.digest.to_hex(),"deadline_tick":tree.deadline,
        "origin_witness":"c".repeat(64),"historical_supply_observed_tick":progress.tick,
        "progress_hex":hex(progress.bytes()),"high_tick":progress.tick,"high_generation":progress.generation,
        "high_sequence":progress.sequence,"high_next_item_id":progress.next_item}),
    )
}

#[test]
fn deferred_tree_template_is_closed_and_cannot_open_as_standalone_material() -> Result<()> {
    let value = input()?;
    assert!(Config::parse(&value.to_string()).is_err());
    let config = Config::parse_project_template(&value.to_string(), true)?;
    assert!(config.project_tree_pending);
    assert!(config.selection.is_none() && config.tree_material.is_none());
    for (field, value) in [
        ("item_id", json!(null)),
        ("item_id", json!(1000)),
        ("tree_material_source", json!(null)),
        ("approved_tree", json!(true)),
    ] {
        let mut changed = input()?;
        changed[field] = value;
        assert!(
            Config::parse_project_template(&changed.to_string(), true).is_err(),
            "{field}"
        );
    }
    let config = Config::parse_project_template(&input()?.to_string(), true)?;
    assert!(config.require_resolved_project_material().is_err());
    Ok(())
}

#[test]
fn deferred_tree_preflight_preserves_independent_generation_and_common_fortress() -> Result<()> {
    let template = template()?;
    let expected = template.tree.as_ref().ok_or_else(exhausted)?;
    assert_ne!(
        expected.binding.generation,
        template.config.input.generation
    );
    assert_ne!(expected.binding.endpoint, template.config.endpoint);
    for changed in 0..4 {
        let mut binding = expected.binding.clone();
        match changed {
            0 => binding.source = FortressIdentity::new("other-world", binding.source.site())?,
            1 => binding.df_version = "other-df".into(),
            2 => binding.dfhack_version = "other-dfhack".into(),
            _ => binding.dimensions[2] += 1,
        }
        assert!(
            ExpectedTree::new(
                &template.config,
                &expected.directory,
                &binding,
                expected.root,
                expected.deadline
            )
            .is_err()
        );
    }
    assert!(
        ExpectedTree::new(
            &template.config,
            template.project_directory(),
            &expected.binding,
            expected.root,
            expected.deadline
        )
        .is_err()
    );
    assert!(
        template
            .clone()
            .with_project_deadline(expected.deadline + 1)
            .is_err()
    );
    Ok(())
}

#[test]
fn deferred_material_activation_keeps_policy_identity_and_requires_exact_typed_tree() -> Result<()>
{
    let template = template()?;
    assert!(template.activate_verified_policy(None).is_err());
    let handoff = historical_reference(&template)?;
    let activated = template.activate_verified_policy(Some(&handoff))?;
    assert_eq!(activated.project_identity(), template.project_identity());
    assert!(!activated.project_tree_pending);
    assert!(activated.selection.is_none());
    assert_eq!(
        activated
            .tree_material
            .as_ref()
            .ok_or_else(exhausted)?
            .directory(),
        handoff.directory()
    );
    for (field, changed) in [
        ("root", json!([14, 15, 2])),
        ("deadline_tick", json!(handoff.project_deadline_tick() + 1)),
        ("directory", json!("/tmp/another-tree-project")),
    ] {
        let mut value = handoff.to_json();
        value[field] = changed;
        let changed = TreeProjectHandoff::from_json(&value)?;
        assert!(
            template.activate_verified_policy(Some(&changed)).is_err(),
            "{field}"
        );
    }
    Ok(())
}

#[test]
fn legacy_project_material_preserves_existing_pinned_input_behavior() -> Result<()> {
    let mut value = input()?;
    value["schema"] = json!("dfmcp.workshop-mcp-development/1");
    value["item_id"] = json!(42);
    let cap = Capture::decode(&vector("capture")?)?;
    let legacy = ProjectWorkshopTemplate::parse(&value.to_string(), None, Arc::new(|| Ok(())))?
        .with_project_deadline(cap.tick + 12000)?;
    let activated = legacy.activate_verified_policy(None)?;
    assert_eq!(activated.selection.ok_or_else(exhausted)?.item_id(), 42);
    assert!(activated.tree_material.is_none());
    let harvesting = template()?;
    assert!(
        legacy
            .activate_verified_policy(Some(&historical_reference(&harvesting)?))
            .is_err()
    );
    Ok(())
}

#[test]
fn project_workshop_clock_cannot_initialize_during_existing_child_recovery() -> Result<()> {
    let cap = Capture::decode(&vector("capture")?)?;
    let mut value = input()?;
    value["schema"] = json!("dfmcp.workshop-mcp-development/1");
    value["item_id"] = json!(42);
    value["clock"] = json!({"initialize":true,"generation":12,"recovery_generation":null,
        "deadline_tick":cap.tick+12000,"recovery":null});
    ProjectWorkshopTemplate::parse(&value.to_string(), None, Arc::new(|| Ok(())))?;
    value["initialize"] = json!(false);
    value["acknowledged"] = json!({"journal_id":"a".repeat(64),"head":"b".repeat(64),"frames":0});
    assert!(
        Config::parse(&value.to_string()).is_ok(),
        "standalone independent clock semantics remain unchanged"
    );
    assert!(ProjectWorkshopTemplate::parse(&value.to_string(), None, Arc::new(|| Ok(()))).is_err());
    value["clock"]["initialize"] = json!(false);
    value["clock"]["recovery"] = json!({"journal_id":"c".repeat(64),"head":"d".repeat(64)});
    ProjectWorkshopTemplate::parse(&value.to_string(), None, Arc::new(|| Ok(())))?;
    value["initialize"] = json!(true);
    value["acknowledged"] = Value::Null;
    assert!(ProjectWorkshopTemplate::parse(&value.to_string(), None, Arc::new(|| Ok(()))).is_err());
    Ok(())
}
