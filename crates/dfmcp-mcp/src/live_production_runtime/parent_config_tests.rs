//! Pure seams used by the staged parent; no environment mutation or native I/O.
use super::*;
use serde_json::{Value, json};

fn configuration() -> String {
    json!({"schema":"dfmcp.combined-production-development/1",
        "directory":"/tmp/staged-combined","initialize":true})
    .to_string()
}

fn donor() -> Value {
    let hash = Digest32::of_bytes(b"verified staged Workshop").to_hex();
    json!({
        "directory":"/tmp/staged-workshop","endpoint":"127.0.0.1:5001",
        "world_folder":"region1","site_id":7,"df_version":"df","dfhack_version":"hack",
        "generation":30,"map_dimensions":[64,64,4],
        "acknowledged":{"journal_id":hash,"head":hash,"frames":7},
        "original_key":"original-carpenters.1","original_plan_digest":hash
    })
}

#[test]
fn staged_donor_injection_preserves_original_raw_input_and_exact_donor() -> Result<()> {
    let raw = configuration();
    let original = raw.clone();
    assert_eq!(parent_configuration(&raw, None)?, raw);
    assert!(parse(&raw)?.5.is_none());
    let joined = parent_configuration(&raw, Some(&donor()))?;
    let workshop = parse(&joined)?.5.ok_or_else(|| invalid("missing donor"))?;
    assert_eq!(raw, original);
    assert_eq!(workshop.directory, Path::new("/tmp/staged-workshop"));
    assert_eq!(workshop.binding.generation(), 30);
    assert_eq!(workshop.original_key, "original-carpenters.1");
    assert_eq!(workshop.acknowledged.frames, 7);
    assert!(!Config::parent_metadata(&raw)?.workshop_checkpoint);
    Ok(())
}

#[test]
fn staged_injection_cannot_hide_duplicate_unknown_or_competing_operator_fields() -> Result<()> {
    let raw = configuration();
    let mut input: Value = serde_json::from_str(&raw).map_err(|_| invalid("fixture"))?;
    for supplied in [Value::Null, json!({}), donor()] {
        input["workshop_prerequisite"] = supplied;
        assert!(parent_configuration(&input.to_string(), Some(&donor())).is_err());
        assert!(parent_configuration(&input.to_string(), None).is_err());
    }
    for bad in [
        raw.replacen("{", r#"{"initialize":true,"#, 1),
        raw.replacen("{", r#"{"protocol":"workshop/1.20","#, 1),
        " ".repeat(MAX_CONFIG_BYTES + 1),
    ] {
        assert!(parent_configuration(&bad, Some(&donor())).is_err());
    }
    let mut invalid_donor = donor();
    invalid_donor["directory"] = json!("/tmp/staged-combined/nested");
    assert!(parse(&parent_configuration(&raw, Some(&invalid_donor))?).is_err());
    Ok(())
}

#[test]
fn complete_original_recovery_is_checked_after_verified_donor_injection() -> Result<()> {
    let mut input: Value =
        serde_json::from_str(&configuration()).map_err(|_| invalid("fixture"))?;
    let hash = Digest32::of_bytes(b"retained combined original").to_hex();
    input["initialize"] = json!(false);
    input["recovery"] = json!({
        "plan_digest":hash,"object_digest":hash,"identity_digest":hash,
        "anchor":{"fortress_id":"1","epoch":0,"sequence":2,"tick":120,"state_hash":hash},
        "frontier":{"journal_id":hash,"head":hash,"authority_tick_floor":140},
        "workshop_prerequisite_digest":hash,
        "workshop_prerequisite_floor":{"tick":130,"generation":30,"next_building_id":91,"fenced":false}
    });
    let raw = input.to_string();
    let metadata = Config::parent_metadata(&raw)?;
    assert!(metadata.has_original && !metadata.initialize);
    assert_eq!(metadata.authority_tick_floor, 140);
    assert!(parse(&parent_configuration(&raw, None)?).is_err());
    let configured = parse(&parent_configuration(&raw, Some(&donor()))?)?;
    assert_eq!(
        configured.2.and_then(|r| r.workshop_prerequisite_digest),
        Some(Digest32::of_bytes(b"retained combined original"))
    );
    input["recovery"]["workshop_prerequisite_floor"]["tick"] = json!(141);
    assert!(parse(&parent_configuration(&input.to_string(), Some(&donor()))?).is_err());
    Ok(())
}

#[test]
fn parent_fingerprint_is_exact_ordered_and_separate_from_injected_handoff() -> Result<()> {
    let raw = [
        configuration(),
        "workforce source".to_owned(),
        "orders source".to_owned(),
    ];
    let original = parent_fingerprint(&raw);
    let _ = parent_configuration(&raw[0], Some(&donor()))?;
    assert_eq!(parent_fingerprint(&raw), original);
    for index in 0..3 {
        let mut changed = raw.clone();
        changed[index].push(' ');
        assert_ne!(parent_fingerprint(&changed), original);
    }
    assert_ne!(
        parent_fingerprint(&["ab".to_owned(), "c".to_owned(), "d".to_owned()]),
        parent_fingerprint(&["a".to_owned(), "bc".to_owned(), "d".to_owned()])
    );
    assert!(Config::parent_allowed_key(
        workshop_config::TOKEN,
        false,
        false,
        false
    ));
    assert!(
        !allowed_key(workshop_config::TOKEN, false, false, false, false),
        "standalone production still refuses an unconfigured donor credential"
    );
    Ok(())
}
