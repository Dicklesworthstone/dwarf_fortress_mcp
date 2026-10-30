//! One read-only receipt/map/operations/map/receipt bracket on one connection.
//! Equal paused endpoints are bounded bracket evidence, not an atomic snapshot
//! across native profiles. Only the durable joint monitor can publish progress.
use super::*;
use crate::live_map::{LiveMapObservation, MAX_MAP_BYTES};
use dfmcp_world::map_region::Region;

/// Complete original wire evidence for the joint monitor. Source generations
/// are independently pinned; credentials and transport selectors are absent.
pub struct Capture {
    pub before: Manifest,
    pub before_capture: Vec<u8>,
    pub furnishings: LinkedSample,
    pub after: Manifest,
    pub after_capture: Vec<u8>,
}

fn bind_map(link: &mut Link, existing: &[i16], check: &dyn Fn() -> Result<()>) -> Result<[i16; 2]> {
    let mut methods = [0; 2];
    for (index, name) in ["Handshake", "ReadObservation"].iter().enumerate() {
        let mut request = Vec::new();
        for (tag, value) in [
            (1, *name),
            (2, "dfmcp.map.v1_5.Request"),
            (3, "dfmcp.map.v1_5.Reply"),
            (4, "dfmcp_map_v1_5"),
        ] {
            bytes(&mut request, tag, value.as_bytes());
        }
        let raw = link.frame_bounded(0, &request, 1024, check)?;
        let reply = Message::parse(&raw, 1)?;
        reply.exact(&[1])?;
        let id = reply.number(1)?;
        require(
            (2..=32767).contains(&id)
                && !existing.contains(&(id as i16))
                && !methods[..index].contains(&(id as i16)),
            "room map methods are invalid or aliased",
        )?;
        methods[index] = id as i16;
    }
    Ok(methods)
}

struct MapRead<'a> {
    method: i16,
    token: &'a [u8],
    nonce: &'a [u8; 32],
    region: Region,
    read: bool,
    pinned: Option<&'a Manifest>,
}

fn map_read(
    link: &mut Link,
    request: MapRead<'_>,
    work: &Work<'_>,
) -> Result<(Manifest, Option<Vec<u8>>, Option<LiveMapObservation>)> {
    let MapRead {
        method,
        token,
        nonce,
        region,
        read,
        pinned,
    } = request;
    let mut request = Vec::new();
    bytes(&mut request, 1, token);
    bytes(&mut request, 2, nonce);
    number(&mut request, 3, 1);
    number(&mut request, 4, 5);
    for (index, value) in region.origin.into_iter().chain(region.size).enumerate() {
        number(&mut request, index as u32 + 5, u64::from(value));
    }
    number(&mut request, 11, MAX_MAP_BYTES as u64);
    let check = || work.check();
    let raw = link.frame_bounded(
        method,
        &request,
        if read { MAX_MAP_BYTES + 4096 } else { 4096 },
        &check,
    )?;
    let reply = Message::parse(&raw, 9)?;
    reply.exact(if read {
        &[1, 2, 3, 4, 5, 6, 7, 8, 9]
    } else {
        &[1, 2, 3, 4, 5, 6, 7, 8]
    })?;
    require(
        reply.boolean(1)?
            && reply.number(2)? == 0
            && reply.bytes(3)? == nonce
            && reply.number(4)? == 1
            && reply.number(5)? == 5,
        "map bracket refused or changed nonce/profile",
    )?;
    let manifest = Manifest {
        generation: reply.number(6)?,
        df_version: text(reply.bytes(7)?)?,
        dfhack_version: text(reply.bytes(8)?)?,
    };
    manifest.validate()?;
    require(
        manifest.df_version == work.original.df_version()
            && manifest.dfhack_version == work.original.dfhack_version()
            && pinned.is_none_or(|expected| expected == &manifest),
        "room map source changed within bracket",
    )?;
    if !read {
        work.check()?;
        return Ok((manifest, None, None));
    }
    let payload = reply.bytes(9)?;
    require(
        !payload.is_empty() && payload.len() <= MAX_MAP_BYTES,
        "room map payload exceeds bounds",
    )?;
    let observation = LiveMapObservation::decode_payload(
        payload,
        manifest.generation,
        manifest.df_version.clone(),
        manifest.dfhack_version.clone(),
    )?;
    work.high_tick
        .set(work.high_tick.get().max(observation.tick().get()));
    work.check()?;
    require(
        observation.map.region == region
            && observation.paused
            && observation.map_dimensions == work.original.dimensions()
            && observation.fortress_id()? == work.original.fortress().fortress_id(),
        "room map differs from the original selected paused fortress",
    )?;
    Ok((manifest, Some(payload.to_vec()), Some(observation)))
}

/// The host first synchronizes the original room monitor's read intent. This
/// client cannot initialize journals, acquire mutation authority, reconnect or
/// select arbitrary native methods. All six bindings are fixed here.
#[allow(clippy::too_many_arguments)]
pub fn acquire_trusted(
    original: &BuildBinding,
    goal: &Goal,
    region: Region,
    operations_token: Vec<u8>,
    build_token: Vec<u8>,
    map_token: Vec<u8>,
    nonce: [u8; 32],
    context: &OperationContext,
    cancellation: BuildCancellation,
    permission: ReadPermission,
) -> Result<Capture> {
    let mut observed_tick_floor = context.anchor.tick.get();
    acquire_trusted_with_floor(
        original,
        goal,
        region,
        operations_token,
        build_token,
        map_token,
        nonce,
        context,
        cancellation,
        permission,
        &mut observed_tick_floor,
    )
}

/// The caller retains this conservative clock floor on both success and error.
/// A decoded map or operations clock must survive loss of the rest of its
/// bracket; the floor grants no condition evidence or positive stability credit.
#[allow(clippy::too_many_arguments)]
pub fn acquire_trusted_with_floor(
    original: &BuildBinding,
    goal: &Goal,
    region: Region,
    operations_token: Vec<u8>,
    build_token: Vec<u8>,
    map_token: Vec<u8>,
    nonce: [u8; 32],
    context: &OperationContext,
    cancellation: BuildCancellation,
    permission: ReadPermission,
    observed_tick_floor: &mut u64,
) -> Result<Capture> {
    context.budget.validate()?;
    let cells = region.volume().map_err(crate::live_map::map_error)?;
    require(
        region.size[2] == 1
            && cells <= 4096
            && cells.saturating_mul(2).saturating_add(goal.records().len())
                <= context.budget.max_entities as usize
            && context.budget.max_bytes >= 22 * 1024 * 1024
            && (1..=60_000).contains(&context.budget.max_wall_millis)
            && original.endpoint().is_ipv4()
            && original.endpoint().ip().is_loopback()
            && original.endpoint().port() != 0
            && nonce != [0; 32]
            && [&operations_token, &build_token, &map_token]
                .iter()
                .all(|v| (32..=256).contains(&v.len()) && !v.contains(&0)),
        "invalid complete room acquisition bounds, endpoint or credentials",
    )?;
    let work = Work {
        context,
        original,
        cancellation: &cancellation,
        permission: &permission,
        deadline: Instant::now()
            .checked_add(Duration::from_millis(context.budget.max_wall_millis))
            .ok_or_else(exhausted)?,
        high_tick: Cell::new(context.anchor.tick.get().max(*observed_tick_floor)),
    };
    let result = (|| {
        for record in goal.records() {
            work.check()?;
            require(
                original.capture_matches(record.plan().before()),
                "room receipt belongs to another original source",
            )?;
            require(
                record
                    .plan()
                    .before()
                    .dimensions()
                    .iter()
                    .zip(region.origin.into_iter().zip(region.size))
                    .all(|(bound, (origin, size))| {
                        origin.checked_add(size).is_some_and(|end| end <= *bound)
                    }),
                "room map selection exceeds original dimensions",
            )?;
            work.high_tick.set(
                work.high_tick
                    .get()
                    .max(record.plan().before().tick())
                    .max(record.after().map_or(0, |capture| capture.tick())),
            );
        }
        work.check()?;
        let check = || work.check();
        let mut link = Link::connect_room_construction(
            original.endpoint(),
            work.remaining()?,
            context.budget.max_bytes,
            cancellation.clone(),
            &check,
        )?;
        link.greeting(&check)?;
        let methods = bind(&mut link, &check)?;
        let maps = bind_map(&mut link, &methods, &check)?;
        let original_manifest = original_manifest(original);
        build_read(
            &mut link,
            methods[0],
            &build_token,
            &nonce,
            None,
            &original_manifest,
            &check,
        )?;
        let limits = PagedOperationsLimits::default();
        let source = handshake_bound(&operations_token, &nonce, limits, |request, maximum| {
            link.frame_bounded(methods[2], request, maximum, &check)
        })?;
        require(
            source.df == original.df_version() && source.dfhack == original.dfhack_version(),
            "room native software families disagree",
        )?;
        let (map, _, _) = map_read(
            &mut link,
            MapRead {
                method: maps[0],
                token: &map_token,
                nonce: &nonce,
                region,
                read: false,
                pinned: None,
            },
            &work,
        )?;
        let before_records = receipts(
            &mut link,
            methods[1],
            &build_token,
            &nonce,
            goal.records(),
            &original_manifest,
            &check,
        )?;
        let (before, before_raw, before_map) = map_read(
            &mut link,
            MapRead {
                method: maps[1],
                token: &map_token,
                nonce: &nonce,
                region,
                read: true,
                pinned: Some(&map),
            },
            &work,
        )?;
        let before_capture = before_raw.ok_or_else(|| invalid("missing room map"))?;
        let before_map = before_map.ok_or_else(|| invalid("missing decoded room map"))?;
        let capture = acquire_bound(
            &operations_token,
            &nonce,
            limits,
            &source,
            true,
            |request, maximum| link.frame_bounded(methods[3], request, maximum, &check),
            |observation| {
                work.high_tick
                    .set(work.high_tick.get().max(observation.jobs.tick().get()));
                work.check()?;
                let count = 1usize
                    .saturating_add(cells.saturating_mul(2))
                    .saturating_add(observation.jobs.jobs.len())
                    .saturating_add(observation.buildings.len())
                    .saturating_add(observation.items.len());
                require(
                    count <= context.budget.max_entities as usize,
                    "complete room capture exceeds entity allowance",
                )?;
                require(
                    observation.jobs.tick() == before_map.tick()
                        && observation.jobs.paused
                        && observation.jobs.fortress_id()? == original.fortress().fortress_id(),
                    "room operations capture differs from paused map endpoint",
                )
            },
        )?;
        drop(capture.observation);
        let (after, after_raw, _) = map_read(
            &mut link,
            MapRead {
                method: maps[1],
                token: &map_token,
                nonce: &nonce,
                region,
                read: true,
                pinned: Some(&map),
            },
            &work,
        )?;
        let after_capture = after_raw.ok_or_else(|| invalid("missing trailing room map"))?;
        require(
            before_capture == after_capture,
            "room map changed across the complete operations capture",
        )?;
        let after_records = receipts(
            &mut link,
            methods[1],
            &build_token,
            &nonce,
            goal.records(),
            &original_manifest,
            &check,
        )?;
        work.check()?;
        Ok(Capture {
            before,
            before_capture,
            furnishings: LinkedSample {
                before: original_manifest.clone(),
                before_records,
                operations: operations_manifest(capture.manifest),
                capture: capture.payload,
                after: original_manifest,
                after_records,
            },
            after,
            after_capture,
        })
    })();
    *observed_tick_floor = (*observed_tick_floor).max(work.high_tick.get());
    result
}
