#![allow(dead_code, unused_imports)]
// Deterministic component fixtures exercise the real projection; not live-game evidence.
include!("../semantic_workforce_runtime/fixtures.rs");
use super::{ObserveRequest, project, select};
use crate::semantic_workforce_runtime::policy::{ACCEPTANCE_KIND, SourceAcceptance};
use serde_json::json;

#[test]
fn empty_canonical_roster_does_not_invent_a_native_detail_observation() -> Result<()> {
    let (capsule, projection) = observation(&[])?;
    let c = permitted(&projection.snapshot);
    let request = ObserveRequest::default();
    assert!(select(&capsule, &projection, &scope(), &request, &c)?.0.is_empty());
    let seal = super::observation_seal(&capsule, &projection);
    let value = super::empty(projection.snapshot.anchor(), seal, &request)?;
    assert_eq!(value["native_observed"], false);
    assert_eq!(value["coverage"]["work_detail_domain"], "unknown");
    assert_eq!(value["options"], json!([]));
    Ok(())
}

fn scope() -> SourceAcceptance {
    SourceAcceptance {
        kind: ACCEPTANCE_KIND.into(),
        world_folder: "region1".into(),
        site_id: 4,
        df_version: "0.51.11".into(),
        dfhack_version: "0.51.11-r1".into(),
        citizen_bridge_version: "0.1.0".into(),
        citizen_generation: CITIZEN_GENERATION,
        workforce_generation: 7,
    }
}
fn permitted(snapshot: &WorldSnapshot) -> OperationContext {
    let mut c = context(snapshot);
    c.grants.push(CapabilityGrant {
        capability: Capability::Observe,
        scope: CapabilityScope::default(),
        max_risk: RiskTier::ReadOnly,
        expires_at_tick: None,
        remaining_uses: None,
    });
    c
}

#[test]
fn choices_are_deterministic_scoped_and_advisory() -> Result<()> {
    let (capsule, projection) = observation(&[42])?;
    let c = permitted(&projection.snapshot);
    let request = ObserveRequest::default();
    let capture = WorkforceFixture::default().capture()?;
    let v = project(&capsule, &projection, &scope(), &capture, &request, &c)?;
    assert_eq!(v, project(&capsule, &projection, &scope(), &capture, &request, &c)?);
    assert_eq!(v["selected_citizens"][0]["entity_id"], "43");
    assert_eq!(v["selected_citizens"][0]["historical_id"], 142);
    assert_eq!(v["options"][0]["labor"], "MINE");
    assert_eq!(v["options"][0]["enable_candidates"], json!(["43"]));
    assert_eq!(v["mutation_authority"], false);
    assert_eq!(v["mutation_precondition_evidence"], false);
    assert_eq!(v["revalidation_required"], true);
    for missing in [Capability::Query, Capability::Observe] {
        let mut denied = c.clone();
        denied.grants.retain(|grant| grant.capability != missing);
        assert!(project(&capsule, &projection, &scope(), &capture, &request, &denied).is_err());
    }
    Ok(())
}

#[test]
fn source_and_identity_drift_refuse_instead_of_offering() -> Result<()> {
    let (capsule, projection) = observation(&[42])?;
    let c = permitted(&projection.snapshot);
    let request = ObserveRequest::default();
    let base = WorkforceFixture::default();
    for fixture in [
        WorkforceFixture {
            generation: 8,
            ..base.clone()
        },
        WorkforceFixture {
            tick: 121,
            ..base.clone()
        },
        WorkforceFixture {
            site: 5,
            ..base.clone()
        },
        WorkforceFixture {
            folder: "other".into(),
            ..base.clone()
        },
        WorkforceFixture {
            paused: false,
            ..base.clone()
        },
        WorkforceFixture {
            citizens: vec![CitizenFixture {
                id: 43,
                eligible: true,
                labors: vec![0, 0],
            }],
            ..base.clone()
        },
    ] {
        assert!(
            project(&capsule, &projection, &scope(), &fixture.capture()?, &request, &c).is_err()
        );
    }
    let mut wrong = scope();
    wrong.citizen_generation += 1;
    assert!(select(&capsule, &projection, &wrong, &request, &c).is_err());
    Ok(())
}

#[test]
fn unsafe_or_ambiguous_details_never_offer_candidates() -> Result<()> {
    let (capsule, projection) = observation(&[42])?;
    let c = permitted(&projection.snapshot);
    let request = ObserveRequest::default();
    let base = WorkforceFixture::default();
    let mut ambiguous = base.clone();
    ambiguous.details.push(base.details[0].clone());
    let mut mixed = base.clone();
    mixed.details[0].labors = vec![1, 1];
    let mut ineligible = base.clone();
    ineligible.citizens[0].eligible = false;
    let mut manual = base.clone();
    manual.automatic = false;
    for fixture in [ambiguous, mixed, ineligible, manual] {
        let v = project(&capsule, &projection, &scope(), &fixture.capture()?, &request, &c)?;
        assert_eq!(v["options"], json!([]));
    }
    let mut overlapping = base.clone();
    overlapping.details[0].members = vec![42];
    overlapping.citizens[0].labors = vec![1, 0];
    overlapping.details.push(DetailFixture {
        name: "Other grant".into(),
        selected_only: false,
        labors: vec![1, 0],
        members: vec![],
    });
    let v = project(&capsule, &projection, &scope(), &overlapping.capture()?, &request, &c)?;
    assert_eq!(v["options"], json!([]));
    Ok(())
}

#[test]
fn selected_roster_is_bounded_and_omissions_are_explicit() -> Result<()> {
    let ids = (0..33).collect::<Vec<i32>>();
    let (capsule, projection) = observation(&ids)?;
    let c = permitted(&projection.snapshot);
    let request = ObserveRequest::default();
    let selected = ids[..32]
        .iter()
        .map(|id| CitizenFixture {
            id: *id as u32,
            eligible: true,
            labors: vec![0, 0],
        })
        .collect();
    let fixture = WorkforceFixture {
        citizens: selected,
        ..WorkforceFixture::default()
    };
    let v = project(&capsule, &projection, &scope(), &fixture.capture()?, &request, &c)?;
    assert_eq!(v["coverage"]["canonical_citizens"], 33);
    assert_eq!(v["coverage"]["selected_citizens"], 32);
    assert_eq!(v["coverage"]["omitted_citizens"], 1);
    assert_eq!(v["selected_citizens"].as_array().map(Vec::len), Some(32));
    Ok(())
}

fn ten_option_capture() -> Result<WorkforceCapture> {
    let keys = (0..10)
        .rev()
        .map(|i| format!("LABOR_{i}"))
        .collect::<Vec<_>>();
    let mut raw = b"DFMWF017".to_vec();
    for n in [7_u64, 3, 120] {
        raw.extend_from_slice(&n.to_be_bytes());
    }
    raw.extend_from_slice(&4_u32.to_be_bytes());
    text(&mut raw, "region1");
    raw.extend_from_slice(&[1, 1]);
    raw.extend_from_slice(&10_u16.to_be_bytes());
    for key in &keys {
        text(&mut raw, key);
    }
    raw.extend_from_slice(&10_u16.to_be_bytes());
    for i in 0..10 {
        text(&mut raw, &format!("detail {i}"));
        raw.extend_from_slice(&0_u32.to_be_bytes());
        raw.push(1);
        for col in 0..10 {
            raw.push(u8::from(col == i));
        }
        raw.extend_from_slice(&0_u16.to_be_bytes());
    }
    raw.extend_from_slice(&1_u16.to_be_bytes());
    raw.extend_from_slice(&42_u32.to_be_bytes());
    raw.extend_from_slice(&142_u32.to_be_bytes());
    raw.push(1);
    raw.extend_from_slice(&[0; 10]);
    WorkforceCapture::decode(&raw)
}

#[test]
fn options_are_sorted_and_bounded_with_explicit_omissions() -> Result<()> {
    let (capsule, projection) = observation(&[42])?;
    let c = permitted(&projection.snapshot);
    let request = ObserveRequest::default();
    let capture = ten_option_capture()?;
    let v = project(&capsule, &projection, &scope(), &capture, &request, &c)?;
    assert_eq!(v["coverage"]["omitted_options"], 2);
    assert_eq!(v["coverage"]["available_options"], 10);
    assert_eq!(v["coverage"]["option_offset"], 0);
    assert_eq!(v["coverage"]["returned_options"], 8);
    assert_eq!(v["coverage"]["next_option_offset"], 8);
    assert_eq!(
        v["next_observation_request"],
        json!({"units":["43"],"option_offset":8})
    );
    let options = v["options"]
        .as_array()
        .ok_or_else(|| invalid("missing options"))?;
    assert_eq!(options.len(), 8);
    assert_eq!(
        options
            .iter()
            .map(|v| v["labor"].as_str())
            .collect::<Vec<_>>(),
        [
            "LABOR_0", "LABOR_1", "LABOR_2", "LABOR_3", "LABOR_4", "LABOR_5", "LABOR_6",
            "LABOR_7",
        ]
        .into_iter()
        .map(Some)
        .collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn observe_requests_are_closed_bounded_and_normalized() -> Result<()> {
    let default = ObserveRequest::default();
    assert_eq!(ObserveRequest::parse("{}")?, default);
    assert_eq!(ObserveRequest::parse(r#"{"option_offset":0}"#)?, default);
    assert!(default.is_default());
    let a = ObserveRequest::parse(r#"{"units":["33","1"],"option_offset":8}"#)?;
    let b = ObserveRequest::parse(r#"{"option_offset":8,"units":["1","33"]}"#)?;
    assert_eq!(a, b);
    assert!(!a.is_default());
    assert_eq!(
        a.selection_json(),
        json!({"units":["1","33"],"option_offset":8})
    );
    assert!(ObserveRequest::parse(r#"{"option_offset":128}"#).is_ok());
    for raw in [
        "",
        "[]",
        "null",
        r#"{"units":[]}"#,
        r#"{"units":["1","1"]}"#,
        r#"{"units":["0"]}"#,
        r#"{"units":["01"]}"#,
        r#"{"units":["+1"]}"#,
        r#"{"units":[" 1"]}"#,
        r#"{"units":["-1"]}"#,
        r#"{"units":["18446744073709551616"]}"#,
        r#"{"units":[1]}"#,
        r#"{"units":["1"],"units":["2"]}"#,
        r#"{"option_offset":129}"#,
        r#"{"option_offset":-1}"#,
        r#"{"option_offset":1.5}"#,
        r#"{"option_offset":0,"option_offset":8}"#,
        r#"{"detail_index":0}"#,
        r#"{"source_generation":7}"#,
        r#"{"native_ids":[42]}"#,
    ] {
        assert!(ObserveRequest::parse(raw).is_err(), "{raw}");
    }
    let oversized = json!({"units":(1..=33).map(|id|id.to_string()).collect::<Vec<_>>()});
    assert!(ObserveRequest::parse(&oversized.to_string()).is_err());
    assert!(ObserveRequest::parse(&" ".repeat(4097)).is_err());
    Ok(())
}

#[test]
fn explicit_selection_reaches_citizens_beyond_the_default_window() -> Result<()> {
    let ids = (0..33).collect::<Vec<i32>>();
    let (capsule, projection) = observation(&ids)?;
    let c = permitted(&projection.snapshot);
    let request = ObserveRequest::parse(r#"{"units":["33"]}"#)?;
    let (selected, total) = select(&capsule, &projection, &scope(), &request, &c)?;
    assert_eq!(total, 33);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].native_id, 32);
    assert_eq!(selected[0].canonical_id.get(), 33);
    let capture = WorkforceFixture {
        citizens: vec![CitizenFixture {
            id: 32,
            eligible: true,
            labors: vec![0, 0],
        }],
        ..WorkforceFixture::default()
    }
    .capture()?;
    let v = project(&capsule, &projection, &scope(), &capture, &request, &c)?;
    assert_eq!(v["selected_citizens"][0]["entity_id"], "33");
    assert_eq!(v["options"][0]["enable_candidates"], json!(["33"]));
    assert_eq!(v["coverage"]["canonical_citizens"], 33);
    assert_eq!(v["coverage"]["selected_citizens"], 1);
    assert_eq!(v["coverage"]["omitted_citizens"], 32);
    assert_eq!(v["coverage"]["selection"], "canonical_id_ascending_explicit");
    assert_eq!(v["selection_request"], json!({"units":["33"],"option_offset":0}));
    assert_eq!(v["witness_version"], 2);
    assert_eq!(v["mutation_authority"], false);
    assert_eq!(v["mutation_precondition_evidence"], false);
    assert!(project(
        &capsule,
        &projection,
        &scope(),
        &capture,
        &ObserveRequest::default(),
        &c
    )
    .is_err());
    Ok(())
}

#[test]
fn selected_identity_and_source_failures_never_fall_back_to_default_citizens() -> Result<()> {
    let (capsule, projection) = observation(&[42, 43])?;
    let c = permitted(&projection.snapshot);
    let foreign = ObserveRequest::parse(r#"{"units":["43","999"]}"#)?;
    assert!(select(&capsule, &projection, &scope(), &foreign, &c).is_err());
    let request = ObserveRequest::parse(r#"{"units":["44"]}"#)?;
    let capture = WorkforceFixture::default().capture()?;
    assert!(project(&capsule, &projection, &scope(), &capture, &request, &c).is_err());
    let selected = WorkforceFixture {
        citizens: vec![CitizenFixture {
            id: 43,
            eligible: true,
            labors: vec![0, 0],
        }],
        ..WorkforceFixture::default()
    };
    let valid = selected.capture()?;
    assert!(project(&capsule, &projection, &scope(), &valid, &request, &c).is_ok());
    let changed = WorkforceFixture {
        generation: 8,
        ..selected
    }
    .capture()?;
    assert!(project(&capsule, &projection, &scope(), &changed, &request, &c).is_err());
    for missing in [Capability::Query, Capability::Observe] {
        let mut denied = c.clone();
        denied.grants.retain(|grant| grant.capability != missing);
        assert!(project(&capsule, &projection, &scope(), &valid, &request, &denied).is_err());
    }
    Ok(())
}

#[test]
fn option_pages_expose_remaining_choices_and_bind_the_exact_projection() -> Result<()> {
    let (capsule, projection) = observation(&[42])?;
    let c = permitted(&projection.snapshot);
    let capture = ten_option_capture()?;
    let first = project(
        &capsule,
        &projection,
        &scope(),
        &capture,
        &ObserveRequest::default(),
        &c,
    )?;
    let request = ObserveRequest::parse(r#"{"option_offset":8}"#)?;
    let later = project(&capsule, &projection, &scope(), &capture, &request, &c)?;
    assert_eq!(later["options"].as_array().map(Vec::len), Some(2));
    assert_eq!(later["options"][0]["labor"], "LABOR_8");
    assert_eq!(later["options"][1]["labor"], "LABOR_9");
    assert_eq!(later["coverage"]["available_options"], 10);
    assert_eq!(later["coverage"]["omitted_options"], 8);
    assert_eq!(later["coverage"]["omitted_options_before"], 8);
    assert_eq!(later["coverage"]["omitted_options_after"], 0);
    assert!(later["coverage"]["next_option_offset"].is_null());
    assert!(later["next_observation_request"].is_null());
    assert_eq!(later["fresh_observation_required"], true);
    assert_eq!(later["coverage"]["paging"], "independent_fresh_observations");
    assert_eq!(first["witness_version"], 1);
    assert_eq!(later["witness_version"], 2);
    assert_ne!(first["witness"], later["witness"]);
    assert_eq!(
        later["source"]["capture_witness"],
        first["source"]["capture_witness"]
    );
    assert_eq!(
        later["canonical_observation_seal"],
        first["canonical_observation_seal"]
    );
    assert_eq!(later["selection_request"], json!({"option_offset":8}));
    assert_eq!(later["mutation_authority"], false);
    assert_eq!(later["mutation_precondition_evidence"], false);

    // A terminal page is explicitly empty within the captured option domain.
    // An offset beyond that domain is a request error, never evidence of absence.
    let end = ObserveRequest::parse(r#"{"option_offset":10}"#)?;
    let empty = project(&capsule, &projection, &scope(), &capture, &end, &c)?;
    assert_eq!(empty["options"], json!([]));
    assert_eq!(empty["coverage"]["available_options"], 10);
    assert_eq!(empty["coverage"]["returned_options"], 0);
    assert_eq!(empty["coverage"]["omitted_options_before"], 10);
    let beyond = ObserveRequest::parse(r#"{"option_offset":11}"#)?;
    assert!(project(&capsule, &projection, &scope(), &capture, &beyond, &c).is_err());
    Ok(())
}

#[test]
fn normalized_selection_is_in_the_witness_and_default_bytes_are_preserved() -> Result<()> {
    let (capsule, projection) = observation(&[42])?;
    let c = permitted(&projection.snapshot);
    let capture = WorkforceFixture::default().capture()?;
    let default = project(
        &capsule,
        &projection,
        &scope(),
        &capture,
        &ObserveRequest::default(),
        &c,
    )?;
    let mut legacy = b"dfmcp-combined-workforce-choices/1\0".to_vec();
    legacy.extend_from_slice(&32_u32.to_be_bytes());
    legacy.extend_from_slice(&8_u32.to_be_bytes());
    legacy.extend_from_slice(b"canonical_id_ascending\0labor_key_ascending\0");
    legacy.extend_from_slice(super::observation_seal(&capsule, &projection).as_bytes());
    legacy.extend_from_slice(capture.witness().as_bytes());
    assert_eq!(default["witness"], Digest32::of_bytes(&legacy).to_hex());
    let explicit = ObserveRequest::parse(r#"{"units":["43"]}"#)?;
    let selected = project(&capsule, &projection, &scope(), &capture, &explicit, &c)?;
    assert_eq!(selected["options"], default["options"]);
    assert_ne!(selected["witness"], default["witness"]);

    let (capsule, projection) = observation(&[42, 43])?;
    let c = permitted(&projection.snapshot);
    let fixture = WorkforceFixture {
        citizens: vec![
            CitizenFixture {
                id: 42,
                eligible: true,
                labors: vec![0, 0],
            },
            CitizenFixture {
                id: 43,
                eligible: true,
                labors: vec![0, 0],
            },
        ],
        ..WorkforceFixture::default()
    };
    let capture = fixture.capture()?;
    let a = ObserveRequest::parse(r#"{"units":["44","43"]}"#)?;
    let b = ObserveRequest::parse(r#"{"units":["43","44"]}"#)?;
    assert_eq!(
        project(&capsule, &projection, &scope(), &capture, &a, &c)?,
        project(&capsule, &projection, &scope(), &capture, &b, &c)?
    );
    Ok(())
}

#[test]
fn empty_rosters_cannot_be_selected_or_paged_as_observed_native_options() -> Result<()> {
    let (capsule, projection) = observation(&[])?;
    let c = permitted(&projection.snapshot);
    let selected = ObserveRequest::parse(r#"{"units":["1"]}"#)?;
    assert!(select(&capsule, &projection, &scope(), &selected, &c).is_err());
    let seal = super::observation_seal(&capsule, &projection);
    let page = ObserveRequest::parse(r#"{"option_offset":8}"#)?;
    assert!(super::empty(projection.snapshot.anchor(), seal, &page).is_err());
    let value = super::empty(
        projection.snapshot.anchor(),
        seal,
        &ObserveRequest::default(),
    )?;
    assert!(value["coverage"]["available_options"].is_null());
    assert!(value["coverage"]["next_option_offset"].is_null());
    assert_eq!(value["native_observed"], false);
    assert!(value["next_observation_request"].is_null());
    Ok(())
}

#[test]
fn next_option_request_pins_resolved_citizens_when_default_roster_order_changes() -> Result<()> {
    let (capsule, projection) = observation(&[42])?;
    let c = permitted(&projection.snapshot);
    let capture = ten_option_capture()?;
    let first = project(
        &capsule,
        &projection,
        &scope(),
        &capture,
        &ObserveRequest::default(),
        &c,
    )?;
    let next = ObserveRequest::parse(&first["next_observation_request"].to_string())?;
    let (later_capsule, later_projection) = observation(&[1, 42])?;
    let later_context = permitted(&later_projection.snapshot);
    let (selected, total) =
        select(&later_capsule, &later_projection, &scope(), &next, &later_context)?;
    assert_eq!(total, 2);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].canonical_id.get(), 43);
    let later = project(
        &later_capsule,
        &later_projection,
        &scope(),
        &capture,
        &next,
        &later_context,
    )?;
    assert_eq!(later["options"][0]["labor"], "LABOR_8");
    assert_eq!(later["selected_citizens"].as_array().map(Vec::len), Some(1));
    assert_eq!(later["selected_citizens"][0]["entity_id"], "43");
    assert_ne!(
        later["canonical_observation_seal"],
        first["canonical_observation_seal"]
    );
    assert_ne!(later["witness"], first["witness"]);

    let (missing_capsule, missing_projection) = observation(&[1])?;
    let missing_context = permitted(&missing_projection.snapshot);
    assert!(select(
        &missing_capsule,
        &missing_projection,
        &scope(),
        &next,
        &missing_context,
    )
    .is_err());
    Ok(())
}
