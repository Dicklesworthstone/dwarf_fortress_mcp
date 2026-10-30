#![cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
use super::*;
use dfmcp_adapter::furniture_handoff::AllocationOutcome;
use dfmcp_adapter::live_jobs::LiveJobObservation;
use dfmcp_adapter::live_operations::{
    LiveItem, LiveOperationsObservation, LiveOperationsState, OperationsProfile,
};
use dfmcp_adapter::room_intent::RoomIntent;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

static DIRECTORY: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-room-intake-{}-{}",
            std::process::id(),
            DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).map_err(storage_error)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(storage_error)?;
        Ok(Self(path))
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn storage_error(_: io::Error) -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "room intake fixture storage failed",
    )
}
fn request(count: usize) -> Value {
    json!({"schema":room_intake::SCHEMA,
        "terrain":{"region":{"origin":[10,10,2],"size":[45,8,1]},
            "floors":(0..count).map(|i| [15+i as u32,15,2]).collect::<Vec<_>>(),
            "walls":[[14,14,2],[14,16,2]]},
        "furniture":{"schema":"dfmcp.furniture-request/1","world_folder":"region1","site":2,
            "excluded_items":[999],"slots":(0..count).map(|i| json!({
                "name":format!("s{i:02}"),"kind":"bed","target":[15+i as u32,15,2],
                "material":[419,-1],"subtype":-1,"max_distance":9,
                "after":if i==0 {vec![]} else {vec![format!("s{:02}",i-1)]}
            })).collect::<Vec<_>>()}})
}
fn source(count: usize) -> Result<LiveOperationsState> {
    let observation = LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 901,
            df_version: "fake-df".into(),
            dfhack_version: "fake-dfhack".into(),
            year: 2,
            year_tick: 100,
            paused: true,
            site_id: 2,
            world_folder: "region1".into(),
            next_job_id: 90,
            jobs: Vec::new(),
        },
        next_building_id: 70,
        next_item_id: 42 + count as u32,
        buildings: Vec::new(),
        attachments: Vec::new(),
        items: (0..count)
            .map(|i| LiveItem {
                native_id: 42 + i as u32,
                item_type: 101,
                type_key: "BED".into(),
                subtype: -1,
                material_type: 419,
                material_index: -1,
                stack_size: 1,
                raw_position: MapCoord::new(10 + i as i32, 11, 2),
                flags: 64,
                container_native_id: None,
                holder_building_native_id: None,
            })
            .collect(),
    };
    let mut state = LiveOperationsState::with_profile(OperationsProfile::PagedV1_4);
    state.publish(observation)?;
    Ok(state)
}
fn allocate(room: &RoomIntent, count: usize) -> Result<(AllocationOutcome, OperationContext)> {
    let source = source(count)?;
    let config = config()?;
    let anchor = source.snapshot().ok_or_else(exhausted)?.anchor();
    let mut c = context(
        SessionId::new(731),
        RequestId::new(1),
        anchor.fortress_id,
        anchor.tick.get(),
        budget(),
        BuildMode::Control,
        true,
    );
    c.anchor = anchor;
    let result = Handoff::allocate(
        &source,
        &c,
        config.endpoint,
        room.furniture(),
        dfmcp_adapter::furniture_supply::MAX_WORK,
    )?;
    Ok((result, c))
}

#[test]
fn room_intake_preserves_complete_one_and_32_target_private_batches() -> Result<()> {
    for count in [1, 32] {
        let directory = Directory::new()?;
        let mut config = config()?;
        config.batch_path = Some(directory.0.join("batch"));
        let room = room_intake::parse(&request(count).to_string())?;
        room_intake::validate(&room, &config)?;
        let (outcome, c) = allocate(&room, count)?;
        let handoff = outcome.handoff.ok_or_else(exhausted)?;
        room_intake::reserve_output(handoff.plan(), Some(&room))?;
        let binding = BuildBinding::new(config.endpoint, "fake-df", "fake-dfhack", &capture()?)?;
        room_intake::validate_dimensions(&room, binding.dimensions())?;
        let mut journal = BuildJournal::open(
            Memory::default(),
            &c,
            BuildMode::Control,
            Some(binding.clone()),
            Some([73; 32]),
        )?;
        let view = journal.inventory(&c)?;
        let definition =
            BatchDefinition::from_room_handoff(room.clone(), handoff, binding, view.journal_id)?;
        assert_eq!(
            BatchDefinition::decode(&definition.canonical_bytes())?,
            definition
        );
        let path = config.batch_path.as_ref().ok_or_else(exhausted)?;
        let parent = open_private_batch(path, &c, BuildMode::Control, Some(definition.clone()))?;
        let result = batch::display(&parent, Some(&view), true);
        assert_eq!(result["total"], count);
        assert_eq!(result["room"]["terrain"]["floor_tiles"], count);
        assert_eq!(result["room"]["terrain"]["wall_tiles"], 2);
        assert_eq!(result["room"]["room_completion_proven"], false);
        assert_eq!(result["room"]["terrain_observed"], false);
        let projected =
            FurnitureRequest::decode(result["room"]["furniture_request"].to_string().as_bytes())?;
        assert_eq!(&projected, room.furniture());
        assert!(result.to_string().len() < OUTPUT_BYTES as usize);
        drop(parent);
        let mut reopened = open_private_batch(path, &c, BuildMode::Recover, None)?;
        reopened.verify(&c)?;
        assert_eq!(reopened.definition(), &definition);
        assert_eq!(reopened.definition().room(), Some(&room));
        assert_eq!(reopened.definition().plan().steps().len(), count);
        assert!(
            view.entries().is_empty(),
            "intake cannot register a placement"
        );
    }
    Ok(())
}

#[test]
fn room_shortage_retains_every_original_requirement_without_a_partial_batch() -> Result<()> {
    for count in [1, 32] {
        let directory = Directory::new()?;
        let room = room_intake::parse(&request(count).to_string())?;
        let (outcome, _) = allocate(&room, 0)?;
        assert!(outcome.handoff.is_none());
        assert!(outcome.report.allocation.assignments.is_empty());
        let result = allocation::shortage_result(&outcome, room.furniture(), Some(&room))?;
        assert_eq!(result["session_opened"], false);
        assert_eq!(result["batch_created"], false);
        assert_eq!(result["game_mutation_dispatched"], false);
        assert_eq!(
            result["allocation"]["request"]["slots"]
                .as_array()
                .map(Vec::len),
            Some(count)
        );
        assert_eq!(
            result["room"]["furniture_request"]["slots"]
                .as_array()
                .map(Vec::len),
            Some(count)
        );
        assert_eq!(result["room"]["terrain"]["floor_tiles"], count);
        assert!(!directory.0.join("batch").exists());
        assert!(result.to_string().len() < OUTPUT_BYTES as usize);
    }
    Ok(())
}

#[test]
fn closed_room_intake_preserves_nested_strict_decoding_and_intent_exclusivity() -> Result<()> {
    let raw = request(1).to_string();
    for bad in [
        raw.replace("\"kind\":\"bed\"", "\"kind\":\"bed\",\"kind\":\"chair\""),
        raw.replace("\"site\":2", "\"site\":2.0"),
        raw.replace("\"site\":2", "\"site\":2,\"unexpected\":1"),
        raw.replace(
            "\"schema\":\"dfmcp.furniture-request/1\"",
            "\"schema\":\"dfmcp.furniture-request/1\",\"schema\":\"dfmcp.furniture-request/1\"",
        ),
        raw.replace("\"walls\":", "\"unknown\":0,\"walls\":"),
        format!(
            "{{\"schema\":\"{}\",{}}}",
            room_intake::SCHEMA,
            &raw[1..raw.len() - 1]
        ),
        " ".repeat(128 * 1024 + 1),
    ] {
        assert!(room_intake::parse(&bad).is_err());
    }
    for bits in 0u8..16 {
        assert_eq!(
            room_intake::exclusive(std::array::from_fn(|i| bits & (1 << i) != 0)).is_ok(),
            bits.count_ones() <= 1
        );
    }
    // Escaped field spellings and input order must not bypass the strict nested
    // decoder or alter the original canonical room identity.
    let escaped = raw.replace("\"furniture\":", "\"furn\\u0069ture\":");
    assert_eq!(room_intake::parse(&escaped)?, room_intake::parse(&raw)?);
    Ok(())
}

#[test]
fn original_room_geometry_is_normalized_bound_and_immutable() -> Result<()> {
    let original = request(2);
    let room = room_intake::parse(&original.to_string())?;
    let mut reordered = original.clone();
    reordered["terrain"]["floors"]
        .as_array_mut()
        .ok_or_else(exhausted)?
        .reverse();
    assert_eq!(room, room_intake::parse(&reordered.to_string())?);
    assert_eq!(RoomIntent::decode(room.canonical_bytes())?, room);
    let mut different = original.clone();
    different["terrain"]["walls"] = json!([[13, 14, 2], [14, 16, 2]]);
    assert_ne!(
        room.digest(),
        room_intake::parse(&different.to_string())?.digest()
    );
    for (field, value) in [
        ("floors", json!([[15, 15, 2], [15, 15, 2]])),
        ("floors", json!([[15, 15, 2]])),
        ("walls", json!([[15, 15, 2]])),
        ("walls", json!([[100, 15, 2]])),
        ("walls", json!([[14, 14, 3]])),
    ] {
        let mut bad = original.clone();
        bad["terrain"][field] = value;
        assert!(room_intake::parse(&bad.to_string()).is_err());
    }
    let mut config = config()?;
    config.scope.max.x = 40;
    assert!(
        room_intake::validate(&room, &config).is_err(),
        "all terrain, not only targets, must fit scope"
    );
    assert!(room_intake::validate_dimensions(&room, [40, 64, 8]).is_err());
    Ok(())
}

#[test]
fn room_origin_cannot_be_retargeted_or_dropped_from_an_allocated_batch() -> Result<()> {
    let room = room_intake::parse(&request(1).to_string())?;
    let (outcome, c) = allocate(&room, 1)?;
    let handoff = outcome.handoff.ok_or_else(exhausted)?;
    let config = config()?;
    let binding = BuildBinding::new(config.endpoint, "fake-df", "fake-dfhack", &capture()?)?;
    let mut journal = BuildJournal::open(
        Memory::default(),
        &c,
        BuildMode::Control,
        Some(binding.clone()),
        Some([74; 32]),
    )?;
    let id = journal.inventory(&c)?.journal_id;
    let legacy = BatchDefinition::from_handoff(handoff.clone(), binding.clone(), id)?;
    let retained =
        BatchDefinition::from_room_handoff(room.clone(), handoff.clone(), binding.clone(), id)?;
    assert_ne!(retained.id(), legacy.id());
    assert_ne!(
        retained.key(&retained.plan().steps()[0]),
        legacy.key(&legacy.plan().steps()[0])
    );
    let mut changed = request(1);
    changed["furniture"]["excluded_items"] = json!([998]);
    let changed = room_intake::parse(&changed.to_string())?;
    assert!(BatchDefinition::from_room_handoff(changed, handoff, binding, id).is_err());
    assert!(batch::reserve_output_with_extra(retained.plan(), OUTPUT_BYTES as usize).is_err());
    Ok(())
}
