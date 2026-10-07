//! Regression fixtures are decoded by the real versioned native codecs. Their
//! process-local evidence grants are test-owned; no compatibility admission is made.

use std::collections::BTreeSet;

use dfmcp_core::{GameTick, IntentId, MapCoord, MapCuboid, ObservationCursor};
use dfmcp_intent::{BuildingKind, ObligationSpec, derive_step_idempotency_key};
use dfmcp_world::{EvidenceCoverage, EvidencePolicy, EvidenceSource, WorldSnapshot};

use super::*;
use crate::build_placement::BuildKind;
use crate::live_map::LiveMapObservation;
use crate::live_operations::{LiveItem, LiveOperationsObservation, OperationsProfile};
use crate::live_projection::{project_live_capsule, raw_unit_id_to_entity_id};
use crate::live_routing::{LiveResolution, route_plan, tile_dig_area};
use crate::live_spatial::{LiveSpatialObservation, citizens::LiveSpatialCitizenObservation};
use crate::{BridgeManifest, CitizenRecord, ObservationAssembler, ObservationPage};

fn text(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}

fn unhex(value: &str) -> Result<Vec<u8>> {
    (0..value.len())
        .step_by(2)
        .map(|at| {
            u8::from_str_radix(&value[at..at + 2], 16).map_err(|_| invalid("invalid fixture hex"))
        })
        .collect()
}

fn observation(ids: &[i32]) -> Result<(LiveObservationCapsule, LiveWorldProjection)> {
    let mut assembler = ObservationAssembler::new(BridgeManifest {
        bridge_version: "0.1.0".to_owned(),
        df_version: "0.51.11".to_owned(),
        dfhack_version: "0.51.11-r1".to_owned(),
        world_loaded: true,
        fortress_mode: true,
        bridge_generation: 7,
        supported_methods: BTreeSet::from(["Handshake".to_owned(), "ReadObservation".to_owned()]),
    });
    assembler.push_page(ObservationPage {
        bridge_generation: 7,
        world_loaded: true,
        fortress_mode: true,
        paused: true,
        current_year: 0,
        current_year_tick: 120,
        world_name: "Routing fixture".to_owned(),
        world_folder: "region1".to_owned(),
        site_id: 4,
        citizen_count_total: ids.len() as u32,
        citizen_offset: 0,
        complete: true,
        citizens: ids
            .iter()
            .map(|id| CitizenRecord {
                unit_id: *id,
                name: format!("Urist {id}"),
                race: "dwarf".to_owned(),
                profession: 4,
                x: 10,
                y: 11,
                z: 2,
                alive: true,
                sane: true,
                active: true,
                visible: true,
                citizen: true,
                resident: false,
                baby: false,
                child: false,
                adult: true,
            })
            .collect(),
    })?;
    let capsule = assembler.finalize()?;
    let projection = project_live_capsule(
        &capsule,
        crate::workforce_control::fortress_id("region1", 4),
        ObservationCursor::ORIGIN,
    )?;
    Ok((capsule, projection))
}

fn policy(snapshot: &WorldSnapshot) -> EvidencePolicy {
    let mut policy = EvidencePolicy::at(snapshot.anchor());
    policy.all_entities = EvidenceCoverage::Observed;
    policy.paused = true;
    for entity in snapshot.graph.entities.values() {
        for fact in entity.fields.values() {
            if let FactSource::DfhackField(field) = &fact.source {
                policy.sources.insert(EvidenceSource::Observed {
                    field: field.clone(),
                    source_digest: fact.source_digest,
                });
            }
        }
    }
    policy
}

fn observed(snapshot: &WorldSnapshot) -> Result<PredicateEvidence<'_>> {
    PredicateEvidence::scoped(snapshot, policy(snapshot))
}

fn plan_with_preconditions(
    snapshot: &WorldSnapshot,
    action: Action,
    preconditions: Vec<Predicate>,
) -> PreparedPlan {
    let action = action.normalized();
    let terminal = Predicate::Paused(true);
    let intent = IntentId::new(1);
    let step = StepId::new(0);
    let deadline = GameTick(snapshot.tick.get() + 100);
    let obligation = action.naturally_temporal().then(|| ObligationSpec {
        terminal: terminal.clone(),
        failure: None,
        deadline_tick: deadline,
        poll_interval_ticks: 1,
        stable_for_observations: 1,
    });
    PreparedPlan::builder(
        intent,
        snapshot.anchor(),
        "semantic routing fixture",
        terminal.clone(),
    )
    .max_risk(action.risk())
    .required_capabilities(BTreeSet::from([action.capability()]))
    .expires_at_tick(deadline)
    .steps(vec![PlanStep {
        id: step,
        idempotency_key: derive_step_idempotency_key(intent, snapshot.anchor(), step, &action),
        required_capability: action.capability(),
        risk: action.risk(),
        action,
        preconditions,
        postconditions: vec![terminal],
        compensation: None,
        obligation,
        depends_on: Vec::new(),
    }])
    .build()
}

fn plan(snapshot: &WorldSnapshot, action: Action) -> PreparedPlan {
    plan_with_preconditions(snapshot, action, Vec::new())
}

fn labor(units: Vec<EntityId>, enabled: bool) -> Action {
    Action::SetLabor {
        units,
        labor: "MINE".to_owned(),
        enabled,
    }
}

fn bed(target: [u32; 3], material: MaterialSelector) -> Action {
    let location = MapCoord::new(target[0] as i32, target[1] as i32, target[2] as i32);
    Action::Build {
        kind: BuildingKind::Furniture("bed".to_owned()),
        location,
        footprint: MapCuboid {
            min: location,
            max: location,
        },
        material,
    }
}

#[derive(Clone)]
struct DetailFixture {
    name: String,
    selected_only: bool,
    labors: Vec<u8>,
    members: Vec<u32>,
}

#[derive(Clone)]
struct CitizenFixture {
    id: u32,
    eligible: bool,
    labors: Vec<u8>,
}

impl CitizenFixture {
    fn append(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.id.to_be_bytes());
        out.extend_from_slice(&(self.id + 100).to_be_bytes());
        out.push(u8::from(self.eligible));
        out.extend_from_slice(&self.labors);
    }
}

#[derive(Clone)]
struct WorkforceFixture {
    generation: u64,
    sequence: u64,
    tick: u64,
    site: u32,
    folder: String,
    paused: bool,
    automatic: bool,
    details: Vec<DetailFixture>,
    citizens: Vec<CitizenFixture>,
}

impl Default for WorkforceFixture {
    fn default() -> Self {
        Self {
            generation: 7,
            sequence: 3,
            tick: 120,
            site: 4,
            folder: "region1".to_owned(),
            paused: true,
            automatic: true,
            details: vec![DetailFixture {
                name: "Mining only".to_owned(),
                selected_only: true,
                labors: vec![1, 0],
                members: Vec::new(),
            }],
            citizens: vec![CitizenFixture {
                id: 42,
                eligible: true,
                labors: vec![0, 0],
            }],
        }
    }
}

impl WorkforceFixture {
    fn capture(&self) -> Result<WorkforceCapture> {
        let mut out = b"DFMWF017".to_vec();
        for n in [self.generation, self.sequence, self.tick] {
            out.extend_from_slice(&n.to_be_bytes());
        }
        out.extend_from_slice(&self.site.to_be_bytes());
        text(&mut out, &self.folder);
        out.extend_from_slice(&[u8::from(self.paused), u8::from(self.automatic)]);
        out.extend_from_slice(&2u16.to_be_bytes());
        text(&mut out, "MINE");
        text(&mut out, "CARPENTRY");
        out.extend_from_slice(&(self.details.len() as u16).to_be_bytes());
        for detail in &self.details {
            text(&mut out, &detail.name);
            out.extend_from_slice(&0u32.to_be_bytes());
            out.push(u8::from(detail.selected_only));
            out.extend_from_slice(&detail.labors);
            out.extend_from_slice(&(detail.members.len() as u16).to_be_bytes());
            for id in &detail.members {
                out.extend_from_slice(&id.to_be_bytes());
            }
        }
        out.extend_from_slice(&(self.citizens.len() as u16).to_be_bytes());
        for citizen in &self.citizens {
            citizen.append(&mut out);
        }
        WorkforceCapture::decode(&out)
    }

    fn effect(
        &self,
        plan: &AssignmentPlan,
        labors: &[u8],
        applied: bool,
    ) -> Result<AssignmentEffect> {
        let mut out = b"DFMWE017".to_vec();
        text(&mut out, plan.key());
        out.extend_from_slice(plan.digest().as_bytes());
        out.extend_from_slice(plan.token());
        out.extend_from_slice(&plan.spec().detail().to_be_bytes());
        out.push(u8::from(plan.spec().assigned()));
        out.extend_from_slice(plan.before().witness().as_bytes());
        for n in [self.generation, self.sequence, self.tick] {
            out.extend_from_slice(&n.to_be_bytes());
        }
        out.push(if applied { 2 } else { 1 });
        let mut after = self.clone();
        if applied {
            after.sequence += 1;
            let members = &mut after.details[plan.spec().detail() as usize].members;
            for citizen in &self.citizens {
                if plan.spec().assigned() {
                    members.push(citizen.id);
                } else {
                    members.retain(|id| *id != citizen.id);
                }
            }
            members.sort_unstable();
            members.dedup();
            for citizen in &mut after.citizens {
                citizen.labors = labors.to_vec();
            }
            out.extend_from_slice(after.capture()?.witness().as_bytes());
        } else {
            out.extend_from_slice(Digest32::ZERO.as_bytes());
        }
        out.extend_from_slice(&2u16.to_be_bytes());
        out.extend_from_slice(
            &(if applied {
                after.citizens.len() as u16
            } else {
                0
            })
            .to_be_bytes(),
        );
        if applied {
            for citizen in &after.citizens {
                citizen.append(&mut out);
            }
        }
        let receipt = crate::bounded_run::hash(b"dfmcp-workforce-receipt/1", &out);
        out.extend_from_slice(receipt.as_bytes());
        AssignmentEffect::decode(&out, plan)
    }
}

fn spatial_state(build: Option<&BuildCapture>) -> Result<LiveSpatialCitizenState> {
    spatial_state_with(build, |_, _| {})
}

fn spatial_state_with(
    build: Option<&BuildCapture>,
    change: fn(&mut LiveOperationsObservation, &mut LiveMapObservation),
) -> Result<LiveSpatialCitizenState> {
    let base = LiveSpatialObservation::decode_payload(
        &unhex(include_str!("../tests/fixtures/spatial_v1_6.hex").trim())?,
        7,
        "df".to_owned(),
        "dfhack".to_owned(),
    )?;
    let mut operations = base.operations().clone();
    let mut terrain = base.terrain().clone();
    operations.jobs.jobs.clear();
    operations.buildings.clear();
    operations.attachments.clear();
    operations.items.clear();
    if let Some(capture) = build {
        terrain.bridge_generation = capture.generation();
        terrain.year = (capture.tick() / 403_200) as u32;
        terrain.year_tick = (capture.tick() % 403_200) as u32;
        terrain.site_id = capture.site();
        terrain.world_folder = capture.folder().to_owned();
        terrain.paused = capture.paused();
        terrain.map_dimensions = capture.dimensions();
        operations.next_building_id = capture.next_building_id();
        operations.next_item_id = capture.selection().item_id() + 1;
        operations.jobs.next_job_id = capture.next_job_id();
        let BuildItem::Visible(item) = capture.item() else {
            return Err(invalid("fixture item"));
        };
        operations.items.push(LiveItem {
            native_id: capture.selection().item_id(),
            item_type: item.native_type() as i32,
            type_key: "Bed".to_owned(),
            subtype: item.subtype(),
            material_type: item.material(),
            material_index: item.material_index(),
            stack_size: 1,
            raw_position: MapCoord::new(
                item.position()[0] as i32,
                item.position()[1] as i32,
                item.position()[2] as i32,
            ),
            flags: 1 << 6,
            container_native_id: None,
            holder_building_native_id: None,
        });
    }
    operations.jobs.bridge_generation = terrain.bridge_generation;
    operations.jobs.year = terrain.year;
    operations.jobs.year_tick = terrain.year_tick;
    operations.jobs.site_id = terrain.site_id as i32;
    operations
        .jobs
        .world_folder
        .clone_from(&terrain.world_folder);
    operations.jobs.paused = terrain.paused;
    change(&mut operations, &mut terrain);
    let op = operations.encode_profile(OperationsProfile::PagedV1_4)?;
    let map = terrain.encode_payload()?;
    let mut spatial = b"DFMS1600".to_vec();
    for part in [&op, &map] {
        spatial.extend_from_slice(&(part.len() as u32).to_be_bytes());
        spatial.extend_from_slice(part);
    }
    let mut citizens = b"DFMC1800".to_vec();
    citizens.extend_from_slice(&1u32.to_be_bytes());
    citizens.extend_from_slice(&42u32.to_be_bytes());
    text(&mut citizens, "Urist");
    text(&mut citizens, "dwarf");
    citizens.extend_from_slice(&3i32.to_be_bytes());
    for n in [1i32, 1, 5] {
        citizens.extend_from_slice(&n.to_be_bytes());
    }
    citizens.extend_from_slice(&0b1_0101_1111u16.to_be_bytes());
    citizens.extend_from_slice(&0i32.to_be_bytes());
    citizens.extend_from_slice(&[1, 1]);
    citizens.extend_from_slice(&0u16.to_be_bytes());
    let mut payload = b"DFMS1800".to_vec();
    for part in [&spatial, &citizens] {
        payload.extend_from_slice(&(part.len() as u32).to_be_bytes());
        payload.extend_from_slice(part);
    }
    let observation = LiveSpatialCitizenObservation::decode_payload(
        &payload,
        terrain.bridge_generation,
        terrain.df_version,
        terrain.dfhack_version,
    )?;
    let mut state = LiveSpatialCitizenState::default();
    state.publish(observation)?;
    Ok(state)
}

fn build_capture() -> Result<BuildCapture> {
    let line = include_str!("../../../bridge/common/tests/fixtures/build_placement_v1_19.json")
        .lines()
        .find_map(|line| line.trim().strip_prefix("\"capture\": \""))
        .and_then(|line| line.strip_suffix("\","))
        .ok_or_else(|| invalid("missing golden build capture"))?;
    BuildCapture::decode(&unhex(line)?)
}

#[test]
fn v1_identity_decodes_zero_plus_one_and_i32_max_without_native_casts() -> Result<()> {
    let (capsule, projection) = observation(&[0, 42, i32::MAX])?;
    let evidence =
        LiveRoutingEvidence::citizens_v1(observed(&projection.snapshot)?, &projection, &capsule)?;
    let canonical = [
        EntityId::new(1),
        EntityId::new(43),
        EntityId::new(i32::MAX as u64 + 1),
    ];
    let units = evidence.resolve_units(&canonical)?;
    assert_eq!(
        units.iter().map(|unit| unit.native_id).collect::<Vec<_>>(),
        vec![0, 42, i32::MAX as u32]
    );
    assert_eq!(
        units
            .iter()
            .map(|unit| unit.canonical_id)
            .collect::<Vec<_>>(),
        canonical
    );
    assert!(units.iter().all(|unit| unit.canonical_generation == 1));
    assert!(evidence.resolve_units(&[EntityId::NIL]).is_err());
    assert!(evidence.resolve_units(&[EntityId::new(42)]).is_err());
    Ok(())
}

#[test]
fn untrusted_laboratory_and_ungranted_sources_cannot_resolve_live_units() -> Result<()> {
    let (capsule, projection) = observation(&[42])?;
    let snapshot = &projection.snapshot;
    let mut missing_source = policy(snapshot);
    missing_source.sources.clear();
    let mut missing_domain = policy(snapshot);
    missing_domain.all_entities = EvidenceCoverage::Unknown;
    for scope in [
        PredicateEvidence::untrusted(snapshot)?,
        PredicateEvidence::laboratory(snapshot)?,
        PredicateEvidence::scoped(snapshot, missing_source)?,
        PredicateEvidence::scoped(snapshot, missing_domain)?,
    ] {
        let bound = LiveRoutingEvidence::citizens_v1(scope, &projection, &capsule)?;
        assert!(bound.resolve_units(&[EntityId::new(43)]).is_err());
    }
    let (_, other) = observation(&[43])?;
    assert!(
        LiveRoutingEvidence::citizens_v1(observed(&other.snapshot)?, &projection, &capsule)
            .is_err()
    );
    assert!(LiveRoutingEvidence::citizens_v1(observed(snapshot)?, &other, &capsule).is_err());
    Ok(())
}

#[test]
fn workforce_candidate_keeps_semantic_anchor_key_and_exact_native_units() -> Result<()> {
    let (capsule, projection) = observation(&[42])?;
    let evidence =
        LiveRoutingEvidence::citizens_v1(observed(&projection.snapshot)?, &projection, &capsule)?;
    let plan = plan(&projection.snapshot, labor(vec![EntityId::new(43)], true));
    let route = route_plan(&plan)?;
    let advisory = route.steps[0]
        .outcome
        .as_ref()
        .map_err(|r| invalid(&r.reason))?;
    assert_eq!(
        advisory.request,
        LiveRequest::WorkDetail {
            units: vec![EntityId::new(43)],
            labor: "MINE".to_owned(),
            assigned: true,
        }
    );
    let resolved = resolve_workforce_step(
        &plan,
        StepId::new(0),
        &evidence,
        &WorkforceFixture::default().capture()?,
    )?;
    assert_eq!(resolved.native_plan().before().ids(), vec![42]);
    assert_eq!(resolved.native_plan().spec().detail(), 0);
    assert_eq!(resolved.native_plan().key(), plan.steps[0].idempotency_key);
    assert_eq!(resolved.anchor(), plan.anchor);
    assert_eq!(resolved.semantic_plan_digest(), plan.digest);
    assert_eq!(resolved.source_digest(), capsule.content_digest);
    assert_eq!(resolved.units()[0].canonical_id, EntityId::new(43));
    Ok(())
}

#[test]
fn spatial_namespaced_citizen_is_resolved_through_its_source_schema() -> Result<()> {
    let state = spatial_state(None)?;
    let snapshot = state
        .snapshot()
        .ok_or_else(|| invalid("fixture snapshot"))?;
    let evidence = LiveRoutingEvidence::spatial_v1_8(observed(snapshot)?, &state)?;
    let canonical = citizen_entity_id(42);
    assert!(canonical.get() > u64::from(u32::MAX));
    let plan = plan(snapshot, labor(vec![canonical], true));
    let source = state
        .observation_full()
        .ok_or_else(|| invalid("fixture observation"))?;
    let terrain = source.spatial().terrain();
    let fixture = WorkforceFixture {
        generation: terrain.bridge_generation,
        tick: snapshot.tick.get(),
        site: terrain.site_id,
        folder: terrain.world_folder.clone(),
        paused: terrain.paused,
        ..WorkforceFixture::default()
    };
    let resolved = resolve_workforce_step(&plan, StepId::new(0), &evidence, &fixture.capture()?)?;
    assert_eq!(resolved.native_plan().before().ids(), vec![42]);
    assert_eq!(resolved.units()[0].canonical_id, canonical);
    assert_eq!(evidence.schema(), LiveIdentitySchema::SpatialV1_8);
    assert!(evidence.resolve_units(&[EntityId::new(42)]).is_err());
    assert!(evidence.resolve_units(&[EntityId::new(43)]).is_err());
    Ok(())
}

#[test]
fn changed_native_capture_identity_selection_or_eligibility_is_refused() -> Result<()> {
    let (capsule, projection) = observation(&[42, 43])?;
    let evidence =
        LiveRoutingEvidence::citizens_v1(observed(&projection.snapshot)?, &projection, &capsule)?;
    let plan = plan(&projection.snapshot, labor(vec![EntityId::new(43)], true));
    let base = WorkforceFixture::default();
    let mut wrong_units = base.clone();
    wrong_units.citizens[0].id = 43;
    let mut extra_units = base.clone();
    extra_units.citizens.push(CitizenFixture {
        id: 43,
        eligible: true,
        labors: vec![0, 0],
    });
    let mut ineligible = base.clone();
    ineligible.citizens[0].eligible = false;
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
            folder: "region2".to_owned(),
            ..base.clone()
        },
        WorkforceFixture {
            paused: false,
            ..base.clone()
        },
        WorkforceFixture {
            automatic: false,
            ..base.clone()
        },
        wrong_units,
        extra_units,
        ineligible,
    ] {
        assert!(
            resolve_workforce_step(&plan, StepId::new(0), &evidence, &fixture.capture()?).is_err()
        );
    }
    let mut no_pause = policy(&projection.snapshot);
    no_pause.paused = false;
    let evidence = LiveRoutingEvidence::citizens_v1(
        PredicateEvidence::scoped(&projection.snapshot, no_pause)?,
        &projection,
        &capsule,
    )?;
    assert!(resolve_workforce_step(&plan, StepId::new(0), &evidence, &base.capture()?).is_err());
    Ok(())
}

#[test]
fn multiple_labor_or_ambiguous_details_are_not_single_labor_resolution() -> Result<()> {
    let (capsule, projection) = observation(&[42])?;
    let evidence =
        LiveRoutingEvidence::citizens_v1(observed(&projection.snapshot)?, &projection, &capsule)?;
    let plan = plan(&projection.snapshot, labor(vec![EntityId::new(43)], true));
    let mut broad = WorkforceFixture::default();
    broad.details[0].labors = vec![1, 1];
    let mut ambiguous = WorkforceFixture::default();
    ambiguous.details.push(ambiguous.details[0].clone());
    let mut everyone = WorkforceFixture::default();
    everyone.details[0].selected_only = false;
    for fixture in [broad, ambiguous, everyone] {
        assert!(
            resolve_workforce_step(&plan, StepId::new(0), &evidence, &fixture.capture()?).is_err()
        );
    }
    let unsupported = plan_with_preconditions(
        &projection.snapshot,
        Action::SetLabor {
            units: vec![EntityId::new(43)],
            labor: "UNKNOWN_LABOR".to_owned(),
            enabled: true,
        },
        Vec::new(),
    );
    assert!(
        resolve_workforce_step(
            &unsupported,
            StepId::new(0),
            &evidence,
            &WorkforceFixture::default().capture()?
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn disabling_labor_refuses_other_details_that_may_still_grant_it() -> Result<()> {
    let (capsule, projection) = observation(&[42])?;
    let evidence =
        LiveRoutingEvidence::citizens_v1(observed(&projection.snapshot)?, &projection, &capsule)?;
    let plan = plan(&projection.snapshot, labor(vec![EntityId::new(43)], false));
    let mut before = WorkforceFixture::default();
    before.details[0].members = vec![42];
    before.citizens[0].labors = vec![1, 0];
    let resolved = resolve_workforce_step(&plan, StepId::new(0), &evidence, &before.capture()?)?;
    resolved.verify_labor_effect(&before.effect(resolved.native_plan(), &[0, 0], true)?)?;
    let still_enabled = before.effect(resolved.native_plan(), &[1, 0], true)?;
    assert!(resolved.verify_labor_effect(&still_enabled).is_err());
    for (selected_only, members) in [(true, vec![42]), (false, Vec::new())] {
        let mut overlapping = before.clone();
        overlapping.details.push(DetailFixture {
            name: "Other permissions".to_owned(),
            selected_only,
            labors: vec![1, 1],
            members,
        });
        assert!(
            resolve_workforce_step(&plan, StepId::new(0), &evidence, &overlapping.capture()?)
                .is_err()
        );
    }
    Ok(())
}

#[test]
fn native_applied_readback_must_preserve_other_labor_columns() -> Result<()> {
    let (capsule, projection) = observation(&[42])?;
    let evidence =
        LiveRoutingEvidence::citizens_v1(observed(&projection.snapshot)?, &projection, &capsule)?;
    let plan = plan(&projection.snapshot, labor(vec![EntityId::new(43)], true));
    let fixture = WorkforceFixture::default();
    let resolved = resolve_workforce_step(&plan, StepId::new(0), &evidence, &fixture.capture()?)?;
    let exact = fixture.effect(resolved.native_plan(), &[1, 0], true)?;
    resolved.verify_labor_effect(&exact)?;
    let broader = fixture.effect(resolved.native_plan(), &[1, 1], true)?;
    assert_eq!(broader.phase(), AssignmentPhase::Applied);
    assert!(resolved.verify_labor_effect(&broader).is_err());
    let unknown = fixture.effect(resolved.native_plan(), &[], false)?;
    assert!(resolved.verify_labor_effect(&unknown).is_err());
    Ok(())
}

#[test]
fn semantic_digest_anchor_and_unknown_preconditions_are_never_replaced() -> Result<()> {
    let (capsule, projection) = observation(&[42])?;
    let evidence =
        LiveRoutingEvidence::citizens_v1(observed(&projection.snapshot)?, &projection, &capsule)?;
    let action = labor(vec![raw_unit_id_to_entity_id(42)?], true);
    let capture = WorkforceFixture::default().capture()?;
    let mut tampered = plan(&projection.snapshot, action.clone());
    tampered.summary.push_str(" changed");
    assert!(resolve_workforce_step(&tampered, StepId::new(0), &evidence, &capture).is_err());
    for predicate in [
        Predicate::False,
        Predicate::FieldCompare {
            entity_id: EntityId::new(43),
            field: "unobserved_eligibility".to_owned(),
            op: CompareOp::Eq,
            value: Value::Bool(true),
        },
    ] {
        let plan = plan_with_preconditions(&projection.snapshot, action.clone(), vec![predicate]);
        assert!(resolve_workforce_step(&plan, StepId::new(0), &evidence, &capture).is_err());
    }
    let (_, other) = observation(&[42, 43])?;
    let other_plan = plan(&other.snapshot, action);
    assert!(resolve_workforce_step(&other_plan, StepId::new(0), &evidence, &capture).is_err());
    Ok(())
}

#[test]
fn furniture_resolution_binds_exact_observed_item_and_original_target() -> Result<()> {
    let capture = build_capture()?;
    let state = spatial_state(Some(&capture))?;
    let snapshot = state
        .snapshot()
        .ok_or_else(|| invalid("fixture snapshot"))?;
    let evidence = LiveRoutingEvidence::spatial_v1_8(observed(snapshot)?, &state)?;
    let plan = plan(
        snapshot,
        bed(capture.selection().target(), MaterialSelector::default()),
    );
    let resolved = resolve_furniture_step(&plan, StepId::new(0), &evidence, &capture)?;
    assert_eq!(resolved.native_plan().before().selection().item_id(), 42);
    assert_eq!(
        resolved.native_plan().before().selection().kind(),
        BuildKind::Bed
    );
    assert_eq!(resolved.native_plan().before().witness(), capture.witness());
    assert_eq!(resolved.native_plan().key(), plan.steps[0].idempotency_key);
    assert_eq!(resolved.semantic_plan_digest(), plan.digest);
    assert_eq!(resolved.anchor(), plan.anchor);
    let mut target = capture.selection().target();
    target[0] += 1;
    let other = plan_with_preconditions(
        snapshot,
        bed(target, MaterialSelector::default()),
        Vec::new(),
    );
    assert!(resolve_furniture_step(&other, StepId::new(0), &evidence, &capture).is_err());
    let mut missing = policy(snapshot);
    missing.sources.retain(|source| {
        !matches!(source,
        EvidenceSource::Observed { field, .. } if field.ends_with("item.getMaterial"))
    });
    let ungranted =
        LiveRoutingEvidence::spatial_v1_8(PredicateEvidence::scoped(snapshot, missing)?, &state)?;
    assert!(resolve_furniture_step(&plan, StepId::new(0), &ungranted, &capture).is_err());
    Ok(())
}

#[test]
fn all_material_constraints_survive_advisory_routing_and_are_explicitly_unresolved() -> Result<()> {
    let capture = build_capture()?;
    let state = spatial_state(Some(&capture))?;
    let snapshot = state
        .snapshot()
        .ok_or_else(|| invalid("fixture snapshot"))?;
    let evidence = LiveRoutingEvidence::spatial_v1_8(observed(snapshot)?, &state)?;
    for material in [
        MaterialSelector {
            required_tokens: BTreeSet::from(["WOOD:OAK".to_owned()]),
            ..MaterialSelector::default()
        },
        MaterialSelector {
            forbidden_tokens: BTreeSet::from(["INORGANIC:COAL".to_owned()]),
            ..MaterialSelector::default()
        },
        MaterialSelector {
            prefer_nearest: true,
            ..MaterialSelector::default()
        },
        MaterialSelector {
            reserve_count: 2,
            ..MaterialSelector::default()
        },
    ] {
        let plan = plan(
            snapshot,
            bed(capture.selection().target(), material.clone()),
        );
        let route = route_plan(&plan)?;
        assert!(route.fully_routable()); // Advisory family coverage, never readiness.
        let step = route.steps[0]
            .outcome
            .as_ref()
            .map_err(|r| invalid(&r.reason))?;
        assert!(
            matches!(&step.request, LiveRequest::Furniture { material: actual, .. } if actual == &material)
        );
        assert!(
            matches!(&step.requires[0], LiveResolution::FurnitureItem { material: actual, .. } if actual == &material)
        );
        let error = resolve_furniture_step(&plan, StepId::new(0), &evidence, &capture)
            .err()
            .ok_or_else(|| invalid("unsupported selector resolved"))?;
        assert!(
            error
                .message
                .contains("original selector is retained and unresolved")
        );
    }
    Ok(())
}

#[test]
fn furniture_and_dig_advisories_validate_native_geometry_before_tiling() -> Result<()> {
    let (_, projection) = observation(&[42])?;
    for target in [
        [0, 2, 0],
        [2, 0, 0],
        [32767, 2, 0],
        [2, 32767, 0],
        [2, 2, 32768],
    ] {
        let plan = plan(
            &projection.snapshot,
            bed(target, MaterialSelector::default()),
        );
        assert!(route_plan(&plan)?.steps[0].outcome.is_err());
    }
    for target in [[1, 1, 0], [32766, 32766, 32767]] {
        let plan = plan(
            &projection.snapshot,
            bed(target, MaterialSelector::default()),
        );
        assert!(route_plan(&plan)?.steps[0].outcome.is_ok());
    }
    for area in [
        MapCuboid {
            min: MapCoord::new(i32::MIN, 1, 1),
            max: MapCoord::new(i32::MAX, 1, 1),
        },
        MapCuboid {
            min: MapCoord::new(2, 1, 1),
            max: MapCoord::new(1, 1, 1),
        },
        MapCuboid {
            min: MapCoord::new(1, 1, 0),
            max: MapCoord::new(1, 1, 0),
        },
        MapCuboid {
            min: MapCoord::new(1, 1, 1),
            max: MapCoord::new(32767, 1, 1),
        },
    ] {
        assert!(tile_dig_area(area).is_err());
    }
    assert_eq!(
        tile_dig_area(MapCuboid {
            min: MapCoord::new(1, 1, 1),
            max: MapCoord::new(64, 64, 1)
        })
        .map_err(|r| invalid(r.reason))?
        .len(),
        64
    );
    assert!(
        tile_dig_area(MapCuboid {
            min: MapCoord::new(1, 1, 1),
            max: MapCoord::new(513, 1, 1)
        })
        .is_err()
    );
    Ok(())
}

#[test]
fn canonical_furniture_material_identity_flags_and_map_dimensions_must_match_native_capture()
-> Result<()> {
    let capture = build_capture()?;
    type Change = fn(&mut LiveOperationsObservation, &mut LiveMapObservation);
    let changes: [Change; 5] = [
        |operations, _| operations.items[0].material_index += 1,
        |operations, _| operations.items[0].flags |= 1 << 1,
        |operations, _| {
            operations.items[0].native_id += 1;
            operations.next_item_id += 1;
        },
        |operations, _| operations.items[0].raw_position.x += 1,
        |_, terrain| terrain.map_dimensions[0] += 1,
    ];
    for change in changes {
        let state = spatial_state_with(Some(&capture), change)?;
        let snapshot = state
            .snapshot()
            .ok_or_else(|| invalid("fixture snapshot"))?;
        let evidence = LiveRoutingEvidence::spatial_v1_8(observed(snapshot)?, &state)?;
        let plan = plan(
            snapshot,
            bed(capture.selection().target(), MaterialSelector::default()),
        );
        assert!(resolve_furniture_step(&plan, StepId::new(0), &evidence, &capture).is_err());
    }
    Ok(())
}
