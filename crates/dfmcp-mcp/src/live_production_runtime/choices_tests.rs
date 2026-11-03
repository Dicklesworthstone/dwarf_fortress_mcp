#![allow(dead_code, unused_imports)]
// Deterministic component fixtures exercise the real projection; not live-game evidence.
include!("../semantic_workforce_runtime/fixtures.rs");
use super::{project, select};
use crate::semantic_workforce_runtime::policy::{ACCEPTANCE_KIND, SourceAcceptance};
use serde_json::json;

#[test]
fn empty_canonical_roster_does_not_invent_a_native_detail_observation() -> Result<()> {
    let (capsule, projection) = observation(&[])?;
    let c = permitted(&projection.snapshot);
    assert!(select(&capsule, &projection, &scope(), &c)?.0.is_empty());
    let seal = super::observation_seal(&capsule, &projection);
    let value = super::empty(projection.snapshot.anchor(), seal);
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
    let capture = WorkforceFixture::default().capture()?;
    let v = project(&capsule, &projection, &scope(), &capture, &c)?;
    assert_eq!(v, project(&capsule, &projection, &scope(), &capture, &c)?);
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
        assert!(project(&capsule, &projection, &scope(), &capture, &denied).is_err());
    }
    Ok(())
}

#[test]
fn source_and_identity_drift_refuse_instead_of_offering() -> Result<()> {
    let (capsule, projection) = observation(&[42])?;
    let c = permitted(&projection.snapshot);
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
        assert!(project(&capsule, &projection, &scope(), &fixture.capture()?, &c).is_err());
    }
    let mut wrong = scope();
    wrong.citizen_generation += 1;
    assert!(select(&capsule, &projection, &wrong, &c).is_err());
    Ok(())
}

#[test]
fn unsafe_or_ambiguous_details_never_offer_candidates() -> Result<()> {
    let (capsule, projection) = observation(&[42])?;
    let c = permitted(&projection.snapshot);
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
        let v = project(&capsule, &projection, &scope(), &fixture.capture()?, &c)?;
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
    let v = project(&capsule, &projection, &scope(), &overlapping.capture()?, &c)?;
    assert_eq!(v["options"], json!([]));
    Ok(())
}

#[test]
fn selected_roster_is_bounded_and_omissions_are_explicit() -> Result<()> {
    let ids = (0..33).collect::<Vec<i32>>();
    let (capsule, projection) = observation(&ids)?;
    let c = permitted(&projection.snapshot);
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
    let v = project(&capsule, &projection, &scope(), &fixture.capture()?, &c)?;
    assert_eq!(v["coverage"]["canonical_citizens"], 33);
    assert_eq!(v["coverage"]["selected_citizens"], 32);
    assert_eq!(v["coverage"]["omitted_citizens"], 1);
    assert_eq!(v["selected_citizens"].as_array().map(Vec::len), Some(32));
    Ok(())
}

#[test]
fn options_are_sorted_and_bounded_with_explicit_omissions() -> Result<()> {
    let (capsule, projection) = observation(&[42])?;
    let c = permitted(&projection.snapshot);
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
    let capture = WorkforceCapture::decode(&raw)?;
    let v = project(&capsule, &projection, &scope(), &capture, &c)?;
    assert_eq!(v["coverage"]["omitted_options"], 2);
    let options = v["options"]
        .as_array()
        .ok_or_else(|| invalid("missing options"))?;
    assert_eq!(options.len(), 8);
    assert_eq!(
        options
            .iter()
            .map(|v| v["labor"].as_str())
            .collect::<Vec<_>>(),
        (0..8)
            .map(|i| Some(keys[9 - i].as_str()))
            .collect::<Vec<_>>()
    );
    Ok(())
}
