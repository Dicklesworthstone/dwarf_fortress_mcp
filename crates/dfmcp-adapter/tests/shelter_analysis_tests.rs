#![forbid(unsafe_code)]
use dfmcp_adapter::live_spatial::SpatialStateView;
use dfmcp_adapter::shelter_analysis::{
    ArrivalStatus, MAX_CITIZENS, MAX_SHELTERS, MAX_WORK, ShelterSelection, assess,
};
use dfmcp_core::{EntityId, ErrorCode, MapCoord, MapCuboid, Result};
use dfmcp_world::PredicateTruth;
#[path = "support/shelter_fixture.rs"]
mod fixture;
use fixture::*;

const ADULT: u16 = 0x11f;

#[test]
fn evaluates_all_three_axes_and_inclusive_boundaries_from_native_capture() -> Result<()> {
    let positions = [[0,0,5], [1,1,5], [2,0,5], [0,2,5], [0,0,4], [0,0,6]];
    let rows: Vec<_> = positions.into_iter().enumerate().map(|(i,p)| (10 + i as u32,p,ADULT)).collect();
    let state = state(&rows)?;
    let before = state.snapshot().cloned();
    let out = assess(&state, &context(&state)?, &selection(&[10,11,12,13,14,15])?, MAX_WORK)?;
    assert_eq!((out.inside, out.outside, out.unresolved), (2,4,0));
    assert_eq!(out.arrival_truth, PredicateTruth::False);
    assert_eq!(state.snapshot(), before.as_ref());
    assert_eq!(out.source_digest, state.source_digest()?);
    assert!(out.rows.iter().all(|row| row.position.is_some()));
    Ok(())
}

#[test]
fn missing_dead_and_unavailable_citizens_never_shrink_the_goal() -> Result<()> {
    let state = state(&[(10,[0,0,5],ADULT), (11,[0,0,5],ADULT & !1),
        (12,[-1,0,5],ADULT), (13,[0,0,5],ADULT & !8), (14,[0,0,5],ADULT & !4)])?;
    let out = assess(&state, &context(&state)?, &selection(&[10,11,12,13,14,15])?, MAX_WORK)?;
    assert_eq!(out.rows.len(), 6);
    assert_eq!((out.inside, out.outside, out.not_alive, out.unresolved), (1,0,1,4));
    assert_eq!(out.arrival_truth, PredicateTruth::False);
    assert_eq!(out.rows[5].status, ArrivalStatus::NotObserved);
    assert!(out.rows[1..].iter().all(|row| row.position.is_none()));
    let out = assess(&state, &context(&state)?, &selection(&[10,12,15])?, MAX_WORK)?;
    assert_eq!(out.arrival_truth, PredicateTruth::Unknown);
    Ok(())
}

#[test]
fn children_babies_busy_and_stressed_citizens_are_not_filtered_as_workers() -> Result<()> {
    let state = state(&[(10,[0,0,5],0x05f), (11,[0,0,5],0x09f), (12,[0,0,5],ADULT & !2)])?;
    let out = assess(&state, &context(&state)?, &selection(&[10,11,12])?, MAX_WORK)?;
    assert_eq!(out.inside, 3);
    assert_eq!(out.arrival_truth, PredicateTruth::True);
    Ok(())
}

#[test]
fn overlapping_shelters_are_a_union_not_duplicate_arrivals() -> Result<()> {
    let state = state(&[(10,[0,0,5],ADULT), (11,[2,0,5],ADULT)])?;
    let a = shelter()?;
    let b = MapCuboid::new(MapCoord::new(0,0,5), MapCoord::new(2,0,5))?;
    let selection = ShelterSelection::new(&[target(11,1),target(10,1)], &[b,a])?;
    assert_eq!(selection, ShelterSelection::new(&[target(10,1),target(11,1)], &[a,b])?);
    let out = assess(&state, &context(&state)?, &selection, MAX_WORK)?;
    assert_eq!(out.inside, 2);
    assert_eq!(out.rows[0].shelter_index, Some(0));
    assert_ne!(selection.digest(), ShelterSelection::new(&[target(10,2),target(11,1)], &[a,b])?.digest());
    Ok(())
}

#[test]
fn a_reappearing_native_id_cannot_satisfy_an_original_incarnation() -> Result<()> {
    let mut state = state(&[(10,[0,0,5],ADULT)])?;
    let goal = selection(&[10])?;
    assert_eq!(assess(&state, &context(&state)?, &goal, MAX_WORK)?.arrival_truth, PredicateTruth::True);
    state.publish(observation(&[], 1, 7)?)?;
    assert_eq!(assess(&state, &context(&state)?, &goal, MAX_WORK)?.rows[0].status, ArrivalStatus::NotObserved);
    state.publish(observation(&[(10,[0,0,5],ADULT)], 2, 7)?)?;
    let out = assess(&state, &context(&state)?, &goal, MAX_WORK)?;
    assert_eq!(out.arrival_truth, PredicateTruth::Unknown);
    assert_eq!(out.rows[0].status, ArrivalStatus::IncarnationChanged);
    assert_eq!(out.rows[0].observed_generation, Some(2));
    Ok(())
}

#[test]
fn each_capture_recomputes_arrival_instead_of_latching_old_success() -> Result<()> {
    let mut state = state(&[(10,[0,0,5],ADULT)])?;
    let goal = selection(&[10])?;
    let first = assess(&state, &context(&state)?, &goal, MAX_WORK)?;
    state.publish(observation(&[(10,[2,0,5],ADULT)], 1, 7)?)?;
    let later = assess(&state, &context(&state)?, &goal, MAX_WORK)?;
    assert_eq!(first.arrival_truth, PredicateTruth::True);
    assert_eq!(later.arrival_truth, PredicateTruth::False);
    assert_eq!(first.selection.digest(), later.selection.digest());
    assert_ne!(first.source_digest, later.source_digest);
    assert_ne!(first.anchor, later.anchor);
    Ok(())
}

#[test]
fn requires_current_read_authority_anchor_and_all_independent_budgets() -> Result<()> {
    let state = state(&[(10,[0,0,5],ADULT)])?;
    let goal = selection(&[10])?;
    let mut c = context(&state)?;
    let good = assess(&state, &c, &goal, MAX_WORK)?;
    assert_eq!(assess(&state, &c, &goal, good.work_units)?, good);
    assert!(matches!(assess(&state, &c, &goal, good.work_units-1), Err(e) if e.code==ErrorCode::BudgetExceeded));
    for n in [0, MAX_WORK+1] { assert!(assess(&state, &c, &goal, n).is_err()); }
    c.budget.max_bytes = goal.required_bytes() - 1;
    assert!(matches!(assess(&state, &c, &goal, MAX_WORK), Err(e) if e.code==ErrorCode::BudgetExceeded));
    c = context(&state)?; c.budget.max_entities = 1;
    assert!(assess(&state, &c, &goal, MAX_WORK).is_err());
    c = context(&state)?; c.anchor.cursor.sequence += 1;
    assert!(matches!(assess(&state, &c, &goal, MAX_WORK), Err(e) if e.code==ErrorCode::StaleAnchor));
    c = context(&state)?; c.grants.clear();
    assert!(matches!(assess(&state, &c, &goal, MAX_WORK), Err(e) if e.code==ErrorCode::CapabilityDenied));
    c = context(&state)?; c.cancellation_requested = true;
    assert!(matches!(assess(&state, &c, &goal, MAX_WORK), Err(e) if e.code==ErrorCode::CancellationRequested));
    Ok(())
}

#[test]
fn selection_refuses_vacuous_aliasing_overflow_and_oversized_goals() -> Result<()> {
    let a = shelter()?;
    assert!(ShelterSelection::new(&[], &[a]).is_err());
    assert!(ShelterSelection::new(&[target(10,1)], &[]).is_err());
    assert!(ShelterSelection::new(&[target(10,1),target(10,2)], &[a]).is_err());
    assert!(ShelterSelection::new(&[target(10,0)], &[a]).is_err());
    let mut wrong = target(10,1); wrong.entity_id = EntityId::new(10);
    assert!(ShelterSelection::new(&[wrong], &[a]).is_err());
    assert!(ShelterSelection::new(&[target(10,1)], &[a,a]).is_err());
    assert!(ShelterSelection::new(&[target(10,1)], &vec![a;MAX_SHELTERS+1]).is_err());
    let ids: Vec<_> = (0..=MAX_CITIZENS as u32).map(|i| target(i,1)).collect();
    assert!(ShelterSelection::new(&ids,&[a]).is_err());
    let reversed = MapCuboid { min:a.max, max:MapCoord::new(-1,0,0) };
    assert!(ShelterSelection::new(&[target(10,1)], &[reversed]).is_err());
    let giant = MapCuboid::new(MapCoord::new(0,0,0),MapCoord::new(32767,32767,32767))?;
    assert!(ShelterSelection::new(&[target(10,1)], &[giant]).is_err());
    Ok(())
}

#[test]
fn maximum_selection_is_complete_and_out_of_map_positions_stay_unknown() -> Result<()> {
    let rows: Vec<_> = (0..128).map(|i| (i,[0,0,5],ADULT)).collect();
    let state = state(&rows)?;
    let ids: Vec<_> = (0..128).collect();
    let out = assess(&state, &context(&state)?, &selection(&ids)?, MAX_WORK)?;
    assert_eq!(out.inside, 128);
    assert_eq!(out.rows.len(), 128);
    let state = fixture::state(&[(0,[32768,0,5],ADULT)])?;
    assert_eq!(assess(&state, &context(&state)?, &selection(&[0])?, MAX_WORK)?.rows[0].status, ArrivalStatus::PositionUnknown);
    let outside = MapCuboid::new(MapCoord::new(32767,32767,32767),MapCoord::new(32767,32767,32767))?;
    let goal = ShelterSelection::new(&[target(0,1)], &[outside])?;
    assert!(assess(&state, &context(&state)?, &goal, MAX_WORK).is_err());
    Ok(())
}
