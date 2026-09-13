#![forbid(unsafe_code)]

use dfmcp_adapter::live_jobs::{LiveJobObservation, LiveJobsState};
use dfmcp_core::{DfmcpError, EntityId, ErrorCode, Result};
use dfmcp_world::{CompareOp, EntityKind, FactPresence, Predicate, QueryOrder, Value, WorldQuery, execute_query};

fn native_payload() -> Result<Vec<u8>> {
    let hex = include_str!("fixtures/jobs_v1_2.hex").trim();
    if hex.len() % 2 != 0 { return Err(DfmcpError::new(ErrorCode::InvalidRequest,"invalid golden hex")); }
    hex.as_bytes().chunks_exact(2).map(|chunk| {
        let text = std::str::from_utf8(chunk).map_err(|_|DfmcpError::new(ErrorCode::InvalidRequest,"invalid golden UTF-8"))?;
        u8::from_str_radix(text,16).map_err(|_|DfmcpError::new(ErrorCode::InvalidRequest,"invalid golden hex"))
    }).collect()
}

#[test]
fn actual_native_serializer_golden_decodes_and_roundtrips() -> Result<()> {
    let bytes = native_payload()?;
    let observation = LiveJobObservation::decode_payload(&bytes,7,"test-df".to_owned(),"test-dfhack".to_owned())?;
    assert_eq!(observation.jobs.len(),2);
    assert_eq!(observation.jobs[0].native_id,0);
    assert!(observation.jobs[0].suspended);
    assert_eq!(observation.jobs[0].worker_native_id,None);
    assert_eq!(observation.jobs[0].holder_native_id,Some(4));
    assert_eq!(observation.jobs[1].native_id,2);
    assert_eq!(observation.jobs[1].reaction,"MAKE_STEEL");
    assert_eq!(observation.jobs[1].worker_native_id,Some(7));
    assert_eq!(observation.encode_payload()?,bytes);
    for end in 0..bytes.len() {
        assert!(LiveJobObservation::decode_payload(&bytes[..end],7,"test-df".to_owned(),"test-dfhack".to_owned()).is_err());
    }
    Ok(())
}

#[test]
fn native_roster_reaches_existing_typed_query_without_fake_related_entities() -> Result<()> {
    let observation = LiveJobObservation::decode_payload(&native_payload()?,7,"test-df".to_owned(),"test-dfhack".to_owned())?;
    let mut state = LiveJobsState::default();
    state.publish(observation)?;
    let snapshot = state.snapshot().ok_or_else(||DfmcpError::new(ErrorCode::InternalInvariantViolation,"missing fixture snapshot"))?;
    let query = WorldQuery { kinds:vec![EntityKind::Job], predicate:Some(Predicate::FieldCompare {
        entity_id:EntityId::NIL, field:"suspended".to_owned(), op:CompareOp::Eq, value:Value::Bool(true)
    }), order:QueryOrder::EntityIdAscending, limit:4, continuation:None };
    let result = execute_query(snapshot,&query,10)?;
    assert_eq!(result.matched,1);
    assert_eq!(result.entities[0].id,EntityId::new(2));
    assert_eq!(snapshot.graph.entities.len(),3);
    assert!(snapshot.graph.edges.is_empty());
    assert!(matches!(result.entities[0].fields["blocking_reason"].presence,Some(FactPresence::Unsupported(_))));
    assert!(snapshot.hash_is_valid());
    Ok(())
}
