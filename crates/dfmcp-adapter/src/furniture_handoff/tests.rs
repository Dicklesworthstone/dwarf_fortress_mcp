use super::*;
use crate::build_placement::journal::{BuildInventory, BuildJournal, BuildMode};
use crate::build_placement::BuildPlan;
use crate::control_effect_journal::EffectJournalStorage;
use crate::furniture_allocation::{Request, Slot};
use crate::furniture_batch::{BatchDefinition, MAX_DEFINITION_BYTES, MAX_LEGACY_DEFINITION_BYTES};
use crate::furniture_batch::store::BatchStore;
use crate::live_jobs::LiveJobObservation;
use crate::live_operations::{LiveItem, LiveOperationsObservation};
use dfmcp_core::{CapabilityGrant, CapabilityScope, GameTick, MapCoord, RequestId, SessionId, WorkBudget};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

fn native() -> Vec<u8> {
    let source = include_str!("../../../../bridge/common/tests/fixtures/build_placement_v1_19.json");
    let raw = source.split("\"capture\": \"").nth(1).unwrap().split('"').next().unwrap();
    raw.as_bytes().chunks_exact(2).map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(),16).unwrap()).collect()
}
fn capture() -> BuildCapture { BuildCapture::decode(&native()).unwrap() }
fn endpoint() -> SocketAddr { SocketAddr::from(([127,0,0,1],5000)) }
fn binding() -> BuildBinding { BuildBinding::new(endpoint(), "df", "dfhack", &capture()).unwrap() }
fn slot(name: &str, target: [u32;3]) -> Slot {
    Slot { name:name.into(), kind:Kind::Bed,target,after:Vec::new(),material:None,subtype:None,max_distance:65532 }
}
fn request(count: usize) -> FurnitureRequest {
    FurnitureRequest::new("region1".into(), 2, Request {
        slots: (0..count).map(|i|slot(&format!("s{i:02}"),[15+i as u32,15,2])).collect(), excluded_items:Vec::new()
    }).unwrap()
}
fn observation(count: usize) -> LiveOperationsObservation {
    LiveOperationsObservation {
        jobs: LiveJobObservation { bridge_generation:901, df_version:"df".into(), dfhack_version:"dfhack".into(),
            year:2,year_tick:100,paused:true,site_id:2,world_folder:"region1".into(),next_job_id:90,jobs:Vec::new() },
        next_building_id:70,next_item_id:42+count as u32,buildings:Vec::new(),attachments:Vec::new(),
        items:(0..count).map(|i| LiveItem { native_id:42+i as u32,item_type:101,type_key:"BED".into(),subtype:-1,
            material_type:419,material_index:-1,stack_size:1,raw_position:MapCoord::new(10+i as i32,11,2),flags:64,
            container_native_id:None,holder_building_native_id:None }).collect()
    }
}
fn published(observed: LiveOperationsObservation) -> LiveOperationsState {
    let mut state = LiveOperationsState::with_profile(OperationsProfile::PagedV1_4);
    state.publish(observed).unwrap(); state
}
fn context(state:&LiveOperationsState) -> OperationContext {
    OperationContext { session_id:SessionId::new(41),request_id:RequestId::new(1),anchor:state.snapshot().unwrap().anchor(),
        budget:WorkBudget { max_wall_millis:60_000,max_bytes:64*1024*1024,max_entities:80000,..WorkBudget::CONSERVATIVE_DEFAULT },
        grants:[Capability::Query,Capability::Plan].into_iter().map(|capability| CapabilityGrant { capability,
            scope:CapabilityScope::default(),max_risk:RiskTier::Guarded,expires_at_tick:None,remaining_uses:None }).collect(),cancellation_requested:false }
}
fn allocated(count:usize) -> Handoff {
    let state=published(observation(count));
    Handoff::allocate(&state,&context(&state),endpoint(),&request(count),furniture_supply::MAX_WORK).unwrap().handoff.unwrap()
}
fn constrained() -> Handoff {
    let mut model=request(1).request().clone();
    model.slots[0].max_distance=9;
    model.slots[0].material=Some((419,-1));
    model.slots[0].subtype=Some(-1);
    let request=FurnitureRequest::new("region1".into(),2,model).unwrap();
    let state=published(observation(1));
    Handoff::allocate(&state,&context(&state),endpoint(),&request,furniture_supply::MAX_WORK).unwrap().handoff.unwrap()
}
fn changed_capture(offset:usize, value:u32) -> BuildCapture {
    let mut raw=native();raw[offset..offset+4].copy_from_slice(&value.to_be_bytes());BuildCapture::decode(&raw).unwrap()
}
fn hash(domain:&[u8],bytes:&[u8])->Digest32 {
    let mut raw=domain.to_vec();raw.push(0);raw.extend_from_slice(bytes);Digest32::of_bytes(&raw)
}
fn field(out:&mut Vec<u8>,bytes:&[u8]) {out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());out.extend_from_slice(bytes);}

#[test]
fn complete_allocation_retains_original_request_source_items_and_exact_plan() {
    let state=published(observation(3));
    let result=Handoff::allocate(&state,&context(&state),endpoint(),&request(3),furniture_supply::MAX_WORK).unwrap();
    let handoff=result.handoff.unwrap();
    assert_eq!(handoff.request(),&request(3));
    assert_eq!(handoff.source().generation,901);
    assert_eq!(handoff.source().anchor,state.snapshot().unwrap().anchor());
    assert_eq!(handoff.source().source_digest,state.source_digest().unwrap());
    assert_eq!(handoff.source().capture_digest,Digest32::of_bytes(&observation(3).encode_profile(OperationsProfile::PagedV1_4).unwrap()));
    assert_eq!(handoff.plan().steps().len(),3);
    for ((step,item),assignment) in handoff.plan().steps().iter().zip(handoff.items()).zip(result.report.allocation.assignments) {
        assert_eq!(step.name,assignment.slot);assert_eq!(step.selection.item_id(),assignment.item_id);
        assert_eq!(item.handle.entity_id,item_entity_id(assignment.item_id));assert_eq!(item.distance,assignment.distance);
    }
    assert!(handoff.validate_binding(&binding()).is_ok());
    assert_ne!(handoff.source().generation,binding().generation());
}

#[test]
fn allocation_uses_global_matching_and_shortage_never_yields_handoff() {
    let mut observed=observation(2);observed.items[1].material_index=1;
    let state=published(observed);
    let mut model=request(2).request().clone();model.slots[1].material=Some((419,-1));
    let requested=FurnitureRequest::new("region1".into(),2,model).unwrap();
    let result=Handoff::allocate(&state,&context(&state),endpoint(),&requested,furniture_supply::MAX_WORK).unwrap();
    assert_eq!(result.handoff.unwrap().items().iter().map(|i|i.candidate.native_id).collect::<Vec<_>>(),[43,42]);
    let state=published(observation(1));
    let shortage=Handoff::allocate(&state,&context(&state),endpoint(),&request(2),furniture_supply::MAX_WORK).unwrap();
    assert!(shortage.handoff.is_none());assert!(shortage.report.allocation.assignments.is_empty());
    assert_eq!(shortage.report.allocation.shortage.unwrap().missing,1);
}

#[test]
fn authority_anchor_profile_and_shared_budgets_precede_publication() {
    let state=published(observation(1));let base=context(&state);
    let mut variants=Vec::new();
    let mut c=base.clone();c.grants.clear();variants.push(c);
    let mut c=base.clone();c.cancellation_requested=true;variants.push(c);
    let mut c=base.clone();c.anchor.tick=GameTick(0);variants.push(c);
    let mut c=base.clone();c.grants[0].expires_at_tick=Some(GameTick(base.anchor.tick.get()-1));variants.push(c);
    let mut c=base.clone();c.budget.max_entities=1;variants.push(c);
    let mut c=base.clone();c.budget.max_bytes=1;variants.push(c);
    let mut c=base.clone();c.budget.max_bytes=ALLOCATION_BYTE_RESERVE-1;variants.push(c);
    for c in variants { assert!(Handoff::allocate(&state,&c,endpoint(),&request(1),furniture_supply::MAX_WORK).is_err()); }
    let mut exact=base.clone();exact.budget.max_bytes=ALLOCATION_BYTE_RESERVE;
    assert!(Handoff::allocate(&state,&exact,endpoint(),&request(1),furniture_supply::MAX_WORK).is_ok());
    for budget in [0,1,10,furniture_supply::MAX_WORK+1] { assert!(Handoff::allocate(&state,&base,endpoint(),&request(1),budget).is_err()); }
    let mut old=LiveOperationsState::default();old.publish(observation(1)).unwrap();
    assert!(Handoff::allocate(&old,&context(&old),endpoint(),&request(1),furniture_supply::MAX_WORK).is_err());
    let foreign=FurnitureRequest::new("region2".into(),2,request(1).request().clone()).unwrap();
    assert!(Handoff::allocate(&state,&base,endpoint(),&foreign,furniture_supply::MAX_WORK).is_err());
    assert!(Handoff::allocate(&state,&base,SocketAddr::from(([192,0,2,1],5000)),&request(1),furniture_supply::MAX_WORK).is_err());
}

#[test]
fn irrelevant_enum_alias_and_attachment_are_not_hidden_by_selection() {
    let mut observed=observation(2);observed.items[1].type_key="CHAIR".into();
    let state=published(observed);
    assert!(Handoff::allocate(&state,&context(&state),endpoint(),&request(1),furniture_supply::MAX_WORK).is_err());
    let mut observed=observation(2);observed.items[1].item_type=102;
    let state=published(observed);
    assert!(Handoff::allocate(&state,&context(&state),endpoint(),&request(1),furniture_supply::MAX_WORK).is_err());
    let mut observed=observation(1);observed.items[0].flags=65;
    let state=published(observed);
    assert!(Handoff::allocate(&state,&context(&state),endpoint(),&request(1),furniture_supply::MAX_WORK).unwrap().handoff.is_none());
}

#[test]
fn fresh_capture_preserves_material_subtype_native_type_and_distance_constraints() {
    let handoff=constrained();let source=binding();
    handoff.validate_capture(&source,&capture()).unwrap();
    // Independent native fixture offsets: visible item begins at215, then xyz,
    // kind, native type, subtype, material type and material index.
    for (offset,value) in [(229,102),(233,0),(237,420),(241,0),(216,9),(224,3)] {
        assert!(handoff.validate_capture(&source,&changed_capture(offset,value)).is_err(),"offset {offset}");
    }
    handoff.validate_capture(&source,&changed_capture(216,11)).unwrap();
    let unconstrained=allocated(1);
    // Unconstrained allocation still names the selected original item's facts.
    assert!(unconstrained.validate_capture(&source,&changed_capture(237,420)).is_err());
    assert!(handoff.validate_capture(&source,&changed_capture(73,43)).is_err());
    let mut raw=native();raw.truncate(215);raw.push(0);
    assert!(handoff.validate_capture(&source,&BuildCapture::decode(&raw).unwrap()).is_err());
}

#[test]
fn allocation_source_clock_horizons_and_software_are_retained_without_generation_alias() {
    let handoff=allocated(1);let source=binding();
    for (offset,value) in [(48,69),(52,89)] {assert!(handoff.validate_capture(&source,&changed_capture(offset,value)).is_err());}
    let mut raw=native();raw[24..32].copy_from_slice(&806499u64.to_be_bytes());
    assert!(handoff.validate_capture(&source,&BuildCapture::decode(&raw).unwrap()).is_err());
    let mut raw=native();raw[8..16].copy_from_slice(&42u64.to_be_bytes());
    assert!(handoff.validate_capture(&source,&BuildCapture::decode(&raw).unwrap()).is_err());
    for (endpoint,df,dfhack) in [(SocketAddr::from(([127,0,0,1],5001)),"df","dfhack"),(endpoint(),"other","dfhack"),(endpoint(),"df","other")] {
        assert!(handoff.validate_binding(&BuildBinding::new(endpoint,df,dfhack,&capture()).unwrap()).is_err());
    }
    // Source generation901 and furniture generation41 are deliberately distinct.
    assert!(handoff.validate_capture(&source,&capture()).is_ok());
}

#[test]
fn retained_codec_refuses_every_truncated_prefix_and_semantically_corrupt_evidence() {
    let handoff=allocated(2);let bytes=handoff.canonical_bytes();
    assert_eq!(Handoff::decode(bytes).unwrap(),handoff);
    for end in 0..bytes.len() {assert!(Handoff::decode(&bytes[..end]).is_err(),"prefix {end}");}
    let mut extra=bytes.to_vec();extra.push(0);assert!(Handoff::decode(&extra).is_err());
    assert!(Handoff::decode(&vec![0;MAX_HANDOFF_BYTES+1]).is_err());
    let mut items=handoff.items.clone();items[1].slot="other".into();
    assert!(Handoff::assemble(handoff.request.clone(),handoff.source.clone(),items).is_err());
    let mut items=handoff.items.clone();items[1].candidate.native_id=items[0].candidate.native_id;
    items[1].handle.entity_id=items[0].handle.entity_id;
    assert!(Handoff::assemble(handoff.request.clone(),handoff.source.clone(),items).is_err());
    for field in 0..4 {
        let mut items=handoff.items.clone();
        match field {0=>items[0].handle.generation=0,1=>items[0].distance+=1,2=>items[0].candidate.position[2]+=1,_=>items[0].candidate.native_id=handoff.source.next_item_id}
        assert!(Handoff::assemble(handoff.request.clone(),handoff.source.clone(),items).is_err());
    }
    let mut source=handoff.source.clone();source.capture_digest=Digest32::ZERO;
    assert!(Handoff::assemble(handoff.request.clone(),source,handoff.items.clone()).is_err());
}

#[test]
fn retained_handles_require_the_original_published_revision() {
    let handoff = allocated(2);
    for revision in [0, 2, u64::MAX] {
        let mut items = handoff.items.clone();
        items[0].handle.revision = revision;
        assert!(Handoff::assemble(handoff.request.clone(), handoff.source.clone(), items).is_err());
    }
    let mut source = handoff.source.clone();
    source.anchor.cursor.sequence = u64::MAX;
    assert!(Handoff::assemble(handoff.request.clone(), source, handoff.items.clone()).is_err());
    let mut state = published(observation(2));
    let mut later = observation(2);
    later.jobs.year_tick += 1;
    state.publish(later).unwrap();
    let value = Handoff::allocate(&state, &context(&state), endpoint(), &request(2), furniture_supply::MAX_WORK).unwrap().handoff.unwrap();
    assert_eq!(value.source.anchor.cursor.sequence, 1);
    assert!(value.items.iter().all(|item| item.handle.revision == 2));
    assert_eq!(Handoff::decode(value.canonical_bytes()).unwrap(), value);
}

fn header(binding:&BuildBinding)->(Vec<u8>,Digest32) {
    let mut raw=b"DFMBJ019".to_vec();field(&mut raw,&binding.encode());raw.extend_from_slice(&[7;32]);
    let id=hash(b"dfmcp-build-journal/1",&raw);raw.extend_from_slice(id.as_bytes());(raw,id)
}
struct Memory(Cursor<Vec<u8>>);
impl Read for Memory {fn read(&mut self,b:&mut[u8])->io::Result<usize>{self.0.read(b)}}
impl Seek for Memory {fn seek(&mut self,p:SeekFrom)->io::Result<u64>{self.0.seek(p)}}
impl Write for Memory {
    fn write(&mut self,b:&[u8])->io::Result<usize>{self.0.write(b)}
    fn flush(&mut self)->io::Result<()>{Ok(())}
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self)->io::Result<()>{Ok(())}
    fn truncate(&mut self,n:u64)->io::Result<()>{self.0.get_mut().truncate(n as usize);Ok(())}
}
fn inventory(definition:&BatchDefinition,capture:Option<BuildCapture>)->BuildInventory {
    let (mut raw,id)=header(definition.binding());assert_eq!(id,definition.journal_id());
    if let Some(capture)=capture {
        let plan=BuildPlan::new(&definition.key(&definition.plan().steps()[0]),capture).unwrap();
        let mut body=vec![0,0,0];field(&mut body,&plan.canonical_bytes());field(&mut body,&[]);
        let mut frame=b"DFMBJF19".to_vec();frame.extend_from_slice(&(body.len()as u32).to_be_bytes());frame.extend_from_slice(&1u64.to_be_bytes());
        frame.extend_from_slice(id.as_bytes());frame.extend_from_slice(&body);let head=hash(b"dfmcp-build-journal-frame/1",&frame);
        frame.extend_from_slice(head.as_bytes());frame.extend_from_slice(b"DFMBJEND");raw.extend_from_slice(&frame);
    }
    let c=context(&published(observation(1)));
    BuildJournal::open(Memory(Cursor::new(raw)),&c,BuildMode::Offline,None,None).unwrap().inventory(&c).unwrap()
}

#[test]
fn legacy_batch_bytes_and_id_stay_exact_while_allocation_binds_more_intent() {
    let handoff=allocated(1);let binding=binding();let(_,journal)=header(&binding);
    let legacy=BatchDefinition::new(handoff.plan().clone(),binding.clone(),journal).unwrap();
    let mut independent=b"DFMFBD01".to_vec();field(&mut independent,handoff.plan().canonical_bytes());field(&mut independent,&binding.encode());independent.extend_from_slice(journal.as_bytes());
    assert_eq!(legacy.canonical_bytes(),independent);
    assert_eq!(legacy.id(),hash(b"dfmcp-furniture-batch-rust/1",&independent));
    assert!(legacy.handoff().is_none());assert!(independent.len()<=MAX_LEGACY_DEFINITION_BYTES);
    assert_eq!(BatchDefinition::decode(&independent).unwrap(),legacy);
    let definition=BatchDefinition::from_handoff(handoff,binding.clone(),journal).unwrap();
    assert!(definition.canonical_bytes().starts_with(b"DFMFBD02"));assert_ne!(definition.id(),legacy.id());
    assert_eq!(BatchDefinition::decode(&definition.canonical_bytes()).unwrap(),definition);
    // Same chosen item/target but narrower original request has a distinct key.
    let constrained=BatchDefinition::from_handoff(constrained(),binding,journal).unwrap();
    assert_eq!(definition.plan(),constrained.plan());assert_ne!(definition.id(),constrained.id());
    let mut corrupted=definition.canonical_bytes();corrupted[7]=b'1';assert!(BatchDefinition::decode(&corrupted).is_err());
    let mut corrupted=definition.canonical_bytes();
    let needle=b"\"item\":42";let at=corrupted.windows(needle.len()).position(|w|w==needle).unwrap();corrupted[at+needle.len()-1]=b'3';
    assert!(BatchDefinition::decode(&corrupted).is_err());
}

#[test]
fn audit_and_next_step_guard_enforce_constraints_after_reopening() {
    let binding=binding();let(_,journal)=header(&binding);
    let original=BatchDefinition::from_handoff(constrained(),binding,journal).unwrap();
    let definition=BatchDefinition::decode(&original.canonical_bytes()).unwrap();
    let view=inventory(&definition,None);
    let step=&definition.plan().steps()[0];let key=definition.key(step);
    definition.validate_next(&view,false,&key,step.selection,Some(&capture())).unwrap();
    for altered in [changed_capture(237,420),changed_capture(216,9),changed_capture(52,89)] {
        assert!(definition.validate_next(&view,false,&key,step.selection,Some(&altered)).is_err());
        let foreign=inventory(&definition,Some(altered));
        assert!(definition.audit(&foreign,false).is_err());
    }
    let retained=inventory(&definition,Some(capture()));
    assert_eq!(definition.audit(&retained,false).unwrap().status,"pending_recovery");
    assert!(definition.validate_next(&view,true,&key,step.selection,Some(&capture())).is_err());
}

#[test]
fn maximum_complete_request_and_handoff_fit_unchanged_parent_and_completion_bounds() {
    let mut model=request(32).request().clone();
    for i in 0..32 {
        model.slots[i].name=format!("{i:02}{}","x".repeat(46));
        model.slots[i].after=(0..i.min(3)).map(|j|format!("{j:02}{}","x".repeat(46))).collect();
    }
    // Fill close to the canonical request bound without truncating original
    // exclusions or shrinking the 32-target result.
    let mut best=FurnitureRequest::new("region1".into(),2,model.clone()).unwrap();
    for n in 10000..14096 {
        model.excluded_items.push(n);
        match FurnitureRequest::new("region1".into(),2,model.clone()) {Ok(request)=>best=request,Err(_)=>break}
    }
    assert!(best.canonical_bytes().len()>MAX_REQUEST_BYTES-10);
    let state=published(observation(32));
    let handoff=Handoff::allocate(&state,&context(&state),endpoint(),&best,furniture_supply::MAX_WORK).unwrap().handoff.unwrap();
    assert_eq!(handoff.items().len(),32);assert!(handoff.canonical_bytes().len()<=MAX_HANDOFF_BYTES);
    let binding=binding();let(_,journal)=header(&binding);
    let definition=BatchDefinition::from_handoff(handoff,binding,journal).unwrap();
    assert!(definition.canonical_bytes().len()<=MAX_DEFINITION_BYTES);
    assert!(MAX_DEFINITION_BYTES+256<crate::furniture_batch::store::MAX_STORE_BYTES);
    assert!(MAX_DEFINITION_BYTES+32*crate::build_placement::MAX_RECORD_BYTES+20_000<crate::construction_plan::origin::MAX_ORIGIN_BYTES);
    let c=context(&state);
    let mut store=BatchStore::open(Memory(Cursor::new(Vec::new())),&c,BuildMode::Control,Some(definition.clone()),true).unwrap();
    store.verify(&c).unwrap();assert_eq!(store.definition(),&definition);store.stop(&c).unwrap();assert!(store.stopped());
}
