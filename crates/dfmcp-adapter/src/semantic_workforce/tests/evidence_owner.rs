// These tests reuse the actual semantic/native coordinator fixture in the
// parent module. The citizen scope is independently injected and intentionally
// grants only pause, positive Unit membership and the exact identity field.

use super::*;
use crate::semantic_workforce::evidence_owner::{
    CitizenEvidenceIo, CitizenEvidenceLimits, CitizenEvidenceSource, CitizenRpcSource,
    CitizenWorkforceEvidenceOwner,
};
use std::cell::Cell;

struct ReadState {
    capsule: LiveObservationCapsule,
    opens: usize,
    active: usize,
    reads: usize,
    authorizations: usize,
    fail_read: Option<usize>,
    deny: bool,
    missing_identity: bool,
    deny_authorization: Option<usize>,
    revoke_on_close: bool,
    delay: Duration,
    contexts: Vec<(u8, u64, u64)>,
}
struct CitizenSource(Rc<RefCell<ReadState>>);
impl Drop for CitizenSource {
    fn drop(&mut self) {
        let mut data = self.0.borrow_mut();
        data.active -= 1;
        if data.revoke_on_close {
            data.deny = true;
        }
    }
}
impl CitizenEvidenceSource for CitizenSource {
    fn bridge_manifest(&self) -> BridgeManifest {
        self.0.borrow().capsule.bridge.clone()
    }
    fn read_page(
        &mut self,
        offset: u32,
        maximum: u32,
        names: bool,
        c: &OperationContext,
    ) -> Result<ObservationPage> {
        let mut data = self.0.borrow_mut();
        data.reads += 1;
        data.contexts
            .push((1, c.budget.max_wall_millis, c.budget.max_bytes));
        if data.fail_read == Some(data.reads) {
            return Err(invalid("injected citizen read interruption"));
        }
        let delay = data.delay;
        let page = page(&data.capsule, offset, maximum, names);
        drop(data);
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        Ok(page)
    }
}
fn page(
    capsule: &LiveObservationCapsule,
    offset: u32,
    maximum: u32,
    names: bool,
) -> ObservationPage {
    let end = (offset as usize + maximum as usize).min(capsule.citizens.len());
    let mut citizens = capsule.citizens[offset as usize..end].to_vec();
    if !names {
        for citizen in &mut citizens {
            citizen.name.clear();
        }
    }
    ObservationPage {
        bridge_generation: capsule.bridge.bridge_generation,
        world_loaded: true,
        fortress_mode: true,
        paused: capsule.paused,
        current_year: capsule.current_year,
        current_year_tick: capsule.current_year_tick,
        world_name: capsule.world_name.clone(),
        world_folder: capsule.world_folder.clone(),
        site_id: capsule.site_id,
        citizen_count_total: capsule.citizens.len() as u32,
        citizen_offset: offset,
        complete: end == capsule.citizens.len(),
        citizens,
    }
}
fn identity_policy(
    capsule: &LiveObservationCapsule,
    projection: &LiveWorldProjection,
) -> EvidencePolicy {
    let mut policy = EvidencePolicy::at(projection.snapshot.anchor());
    policy.paused = true;
    policy
        .entity_kinds
        .insert(EntityKind::Unit, EvidenceCoverage::Observed);
    policy.sources.insert(EvidenceSource::Observed {
        field: "dfmcp_bridge.ReadObservation.citizen.unit_id".to_owned(),
        source_digest: capsule.content_digest,
    });
    policy
}
type Factory = Box<dyn FnMut(&OperationContext) -> Result<CitizenSource>>;
type Authorizer = Box<
    dyn FnMut(
        &LiveObservationCapsule,
        &LiveWorldProjection,
        &OperationContext,
    ) -> Result<EvidencePolicy>,
>;
type EvidenceOwner = CitizenWorkforceEvidenceOwner<Factory, Authorizer>;

fn evidence_owner(
    capsule: LiveObservationCapsule,
    c: &OperationContext,
    limits: CitizenEvidenceLimits,
) -> Result<(EvidenceOwner, Rc<RefCell<ReadState>>)> {
    let state = Rc::new(RefCell::new(ReadState {
        capsule: capsule.clone(),
        opens: 0,
        active: 0,
        reads: 0,
        authorizations: 0,
        fail_read: None,
        deny: false,
        missing_identity: false,
        deny_authorization: None,
        revoke_on_close: false,
        delay: Duration::ZERO,
        contexts: Vec::new(),
    }));
    let acquire = Rc::clone(&state);
    let factory: Factory = Box::new(move |c| {
        let mut data = acquire.borrow_mut();
        data.opens += 1;
        data.active += 1;
        data.contexts
            .push((0, c.budget.max_wall_millis, c.budget.max_bytes));
        Ok(CitizenSource(Rc::clone(&acquire)))
    });
    let grants = Rc::clone(&state);
    let authorizer: Authorizer = Box::new(move |capsule, projection, c| {
        let mut data = grants.borrow_mut();
        assert_eq!(
            data.active, 0,
            "native source must be closed before source authority publication"
        );
        data.authorizations += 1;
        data.contexts
            .push((2, c.budget.max_wall_millis, c.budget.max_bytes));
        if data.deny || data.deny_authorization == Some(data.authorizations) {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "independent source grant revoked",
            ));
        }
        let mut policy = identity_policy(capsule, projection);
        if data.missing_identity {
            policy.sources.clear();
        }
        Ok(policy)
    });
    let owner = CitizenWorkforceEvidenceOwner::new(capsule, limits, factory, authorizer, c)?;
    Ok((owner, state))
}
fn small_limits() -> CitizenEvidenceLimits {
    CitizenEvidenceLimits {
        page_size: 1,
        max_citizens: 4,
    }
}
fn replace_capsule(
    state: &Rc<RefCell<ReadState>>,
    edit: impl FnOnce(&mut BridgeManifest, &mut ObservationPage),
) -> Result<()> {
    let original = state.borrow().capsule.clone();
    let mut manifest = original.bridge.clone();
    let mut one = page(&original, 0, 4096, original.names_included);
    edit(&mut manifest, &mut one);
    let mut assembler = ObservationAssembler::with_names(manifest, original.names_included);
    assembler.push_page(one)?;
    state.borrow_mut().capsule = assembler.finalize()?;
    Ok(())
}

#[test]
fn acquired_citizen_owner_drives_actual_semantic_prepare_and_commit() -> Result<()> {
    let mut f = fixture()?;
    let (mut evidence, state) =
        evidence_owner(f.observer.capsule.clone(), &f.context, small_limits())?;
    let review = f
        .owner
        .observe(f.plan.clone(), &mut evidence, &f.context, |binding, c| {
            f.native.connect(binding, c)
        })?;
    f.owner
        .prepare(review.seal(), &mut evidence, &f.context, |binding, c| {
            f.native.connect(binding, c)
        })?;
    let result = f.owner.commit(
        review.seal(),
        true,
        &mut evidence,
        &f.context,
        |binding, c| f.native.connect(binding, c),
    )?;
    assert!(matches!(
        result.action_result(),
        SingleLaborResult::Verified { .. }
    ));
    assert_eq!(result.review().original_plan(), &f.plan);
    assert!(!result.original_goal_proven());
    assert_eq!(
        result.pending_obligation(),
        f.plan.steps[0].obligation.as_ref()
    );
    let data = state.borrow();
    assert_eq!(data.opens, 6); // initial, observe, twice prepare, twice commit
    assert_eq!(data.reads, 6);
    assert_eq!(data.authorizations, 12);
    assert_eq!(data.active, 0);
    assert_eq!(f.native.data.borrow().calls[2], 1);
    Ok(())
}

#[test]
fn changed_citizen_evidence_blocks_dispatch_and_clears_routing_scope() -> Result<()> {
    let mut f = fixture()?;
    let (mut evidence, state) =
        evidence_owner(f.observer.capsule.clone(), &f.context, small_limits())?;
    let review = f
        .owner
        .observe(f.plan.clone(), &mut evidence, &f.context, |binding, c| {
            f.native.connect(binding, c)
        })?;
    f.owner
        .prepare(review.seal(), &mut evidence, &f.context, |binding, c| {
            f.native.connect(binding, c)
        })?;
    replace_capsule(&state, |_, page| page.citizens[0].x += 1)?;
    assert!(
        f.owner
            .commit(
                review.seal(),
                true,
                &mut evidence,
                &f.context,
                |binding, c| f.native.connect(binding, c)
            )
            .is_err()
    );
    assert!(evidence.routing_evidence().is_err());
    assert_eq!(f.native.data.borrow().calls[2], 0);
    assert_eq!(state.borrow().active, 0);
    assert_eq!(
        f.owner
            .inspect(review.seal(), &f.context)?
            .native_record()
            .state(),
        AssignmentState::Prepared
    );
    Ok(())
}

#[test]
fn source_revocation_after_dispatch_sync_preserves_uncertainty_without_setter() -> Result<()> {
    let mut f = fixture()?;
    let (mut evidence, state) =
        evidence_owner(f.observer.capsule.clone(), &f.context, small_limits())?;
    let review = f
        .owner
        .observe(f.plan.clone(), &mut evidence, &f.context, |binding, c| {
            f.native.connect(binding, c)
        })?;
    f.owner
        .prepare(review.seal(), &mut evidence, &f.context, |binding, c| {
            f.native.connect(binding, c)
        })?;
    let next = state.borrow().authorizations + 3;
    state.borrow_mut().deny_authorization = Some(next);
    assert!(
        f.owner
            .commit(
                review.seal(),
                true,
                &mut evidence,
                &f.context,
                |binding, c| { f.native.connect(binding, c) }
            )
            .is_err()
    );
    assert!(evidence.routing_evidence().is_err());
    assert_eq!(f.native.data.borrow().calls[2], 0);
    assert_eq!(
        f.owner
            .inspect(review.seal(), &f.context)?
            .native_record()
            .state(),
        AssignmentState::DispatchStarted
    );
    assert!(
        f.owner
            .commit(review.seal(), true, &mut evidence, &f.context, never)
            .is_err()
    );
    assert_eq!(f.native.data.borrow().calls[2], 0);
    Ok(())
}

#[test]
fn complete_repagination_preserves_original_anchor_and_identity_zero() -> Result<()> {
    let (capsule, projection) = observation(&[0, 42, i32::MAX])?;
    let c = context(&projection.snapshot);
    let (mut owner, state) = evidence_owner(capsule, &c, small_limits())?;
    let scope = owner.routing_evidence()?;
    assert_eq!(scope.anchor(), c.anchor);
    let ids = scope.resolve_units(&[
        EntityId::new(1),
        EntityId::new(43),
        EntityId::new(i32::MAX as u64 + 1),
    ])?;
    assert_eq!(
        ids.iter().map(|id| id.native_id).collect::<Vec<_>>(),
        vec![0, 42, i32::MAX as u32]
    );
    owner.refresh(&c)?;
    assert_eq!(owner.routing_evidence()?.anchor(), c.anchor);
    assert_eq!(state.borrow().reads, 6);
    assert_eq!(state.borrow().active, 0);
    Ok(())
}

#[test]
fn partial_page_failure_and_name_projection_never_reuse_prior_scope() -> Result<()> {
    let (named, _) = observation(&[0, 42, 43])?;
    let mut assembler = ObservationAssembler::with_names(named.bridge.clone(), false);
    assembler.push_page(page(&named, 0, 4096, false))?;
    let capsule = assembler.finalize()?;
    let projection = project_live_capsule(
        &capsule,
        crate::workforce_control::fortress_id("region1", 4),
        ObservationCursor::ORIGIN,
    )?;
    let c = context(&projection.snapshot);
    let (mut owner, state) = evidence_owner(capsule, &c, small_limits())?;
    assert_eq!(owner.routing_evidence()?.anchor(), c.anchor);
    let before = state.borrow().reads;
    let authorizations = state.borrow().authorizations;
    state.borrow_mut().fail_read = Some(before + 2);
    assert!(owner.refresh(&c).is_err());
    assert!(owner.routing_evidence().is_err());
    assert_eq!(state.borrow().reads, before + 2);
    assert_eq!(state.borrow().authorizations, authorizations);
    assert_eq!(state.borrow().active, 0);
    Ok(())
}

#[test]
fn denied_or_incomplete_independent_source_grants_never_publish() -> Result<()> {
    for case in 0..4 {
        let f = fixture()?;
        let (mut owner, state) = evidence_owner(f.observer.capsule, &f.context, small_limits())?;
        {
            let mut data = state.borrow_mut();
            match case {
                0 => data.deny = true,
                1 => data.missing_identity = true,
                2 => data.deny_authorization = Some(data.authorizations + 2),
                _ => data.revoke_on_close = true,
            }
        }
        assert!(
            matches!(owner.refresh(&f.context), Err(e) if e.code == ErrorCode::CapabilityDenied)
        );
        assert!(owner.routing_evidence().is_err());
        assert_eq!(state.borrow().active, 0);
    }
    Ok(())
}

#[test]
fn refresh_refusal_invalidates_before_authority_budget_or_anchor_checks() -> Result<()> {
    for case in 0..5 {
        let f = fixture()?;
        let (mut owner, state) = evidence_owner(f.observer.capsule, &f.context, small_limits())?;
        let before = state.borrow().opens;
        let mut c = f.context.clone();
        match case {
            0 => c.grants.clear(),
            1 => c.cancellation_requested = true,
            2 => c.budget.max_bytes = 1,
            3 => c.anchor.tick = GameTick(c.anchor.tick.get() + 1),
            _ => c.budget.max_entities = 1,
        }
        assert!(owner.refresh(&c).is_err());
        assert!(owner.routing_evidence().is_err());
        assert_eq!(state.borrow().opens, before);
    }
    Ok(())
}

#[test]
fn changed_fortress_generation_software_pause_tick_or_roster_is_refused() -> Result<()> {
    for case in 0..7 {
        let f = fixture()?;
        let (mut owner, state) = evidence_owner(f.observer.capsule, &f.context, small_limits())?;
        replace_capsule(&state, |manifest, page| match case {
            0 => page.world_folder = "different-save".to_owned(),
            1 => page.site_id += 1,
            2 => {
                manifest.bridge_generation += 1;
                page.bridge_generation += 1;
            }
            3 => manifest.dfhack_version = "different-build".to_owned(),
            4 => page.paused = false,
            5 => page.current_year_tick += 1,
            _ => page.citizens[0].unit_id += 1,
        })?;
        assert!(owner.refresh(&f.context).is_err());
        assert!(owner.routing_evidence().is_err());
        assert_eq!(state.borrow().active, 0);
    }
    Ok(())
}

#[test]
fn mismatching_native_capture_refuses_before_canonical_source_connection() -> Result<()> {
    for case in 0..6 {
        let f = fixture()?;
        let (mut owner, state) = evidence_owner(f.observer.capsule, &f.context, small_limits())?;
        let before = state.borrow().opens;
        let mut native = WorkforceFixture::default();
        match case {
            0 => native.generation += 1,
            1 => native.tick += 1,
            2 => native.site += 1,
            3 => native.folder = "different-save".to_owned(),
            4 => native.paused = false,
            _ => native.citizens[0].id += 1,
        }
        assert!(
            owner
                .refresh_after_capture(&native.capture()?, &f.context)
                .is_err()
        );
        assert!(owner.routing_evidence().is_err());
        assert_eq!(state.borrow().opens, before);
    }
    Ok(())
}

#[test]
fn interrupted_or_over_deadline_reads_close_without_retries_or_old_scope() -> Result<()> {
    for delayed in [false, true] {
        let f = fixture()?;
        let (mut owner, state) = evidence_owner(f.observer.capsule, &f.context, small_limits())?;
        let mut c = f.context.clone();
        {
            let mut data = state.borrow_mut();
            if delayed {
                data.delay = Duration::from_millis(30);
                c.budget.max_wall_millis = 10;
            } else {
                data.fail_read = Some(data.reads + 1);
            }
        }
        let before = state.borrow().reads;
        assert!(owner.refresh(&c).is_err());
        assert!(owner.routing_evidence().is_err());
        assert_eq!(state.borrow().reads, before + 1);
        assert_eq!(state.borrow().active, 0);
    }
    Ok(())
}

#[test]
fn all_pages_share_shrinking_work_and_context_free_views_are_bounded() -> Result<()> {
    let (capsule, projection) = observation(&[1, 2, 3])?;
    let c = context(&projection.snapshot);
    let (owner, state) = evidence_owner(capsule, &c, small_limits())?;
    let data = state.borrow();
    assert_eq!(data.contexts[0].0, 0);
    assert!(data.contexts.windows(2).all(|pair| pair[1].1 <= pair[0].1));
    assert!(data.contexts[1..4].iter().all(|edge| edge.0 == 1));
    assert!(
        data.contexts[4..]
            .iter()
            .all(|edge| edge.0 == 2 && edge.2 <= 256 * 1024)
    );
    assert!(data.contexts[0].2 < c.budget.max_bytes);
    drop(data);
    owner.routing_evidence()?;
    owner.routing_evidence()?;
    assert!(matches!(owner.routing_evidence(), Err(e) if e.code == ErrorCode::BudgetExceeded));
    Ok(())
}

#[test]
fn oversized_basis_and_total_page_allowance_are_refused_without_scope() -> Result<()> {
    let (capsule, projection) = observation(&[1, 2, 3])?;
    let mut c = context(&projection.snapshot);
    assert!(
        evidence_owner(
            capsule.clone(),
            &c,
            CitizenEvidenceLimits {
                page_size: 1,
                max_citizens: 2
            }
        )
        .is_err()
    );
    c.budget.max_bytes = 1;
    assert!(evidence_owner(capsule, &c, small_limits()).is_err());
    for limits in [
        CitizenEvidenceLimits {
            page_size: 0,
            max_citizens: 1,
        },
        CitizenEvidenceLimits {
            page_size: 4097,
            max_citizens: 1,
        },
        CitizenEvidenceLimits {
            page_size: 1,
            max_citizens: 4097,
        },
    ] {
        assert!(limits.validate().is_err());
    }
    Ok(())
}

// Encode test replies independently, then run the actual strict V1 client.
fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 128 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}
fn number(out: &mut Vec<u8>, tag: u32, value: u64) {
    varint(out, u64::from(tag) << 3);
    varint(out, value);
}
fn data(out: &mut Vec<u8>, tag: u32, value: &[u8]) {
    varint(out, (u64::from(tag) << 3) | 2);
    varint(out, value.len() as u64);
    out.extend_from_slice(value);
}
fn signed(out: &mut Vec<u8>, tag: u32, value: i32) {
    number(out, tag, ((value << 1) ^ (value >> 31)) as u32 as u64);
}
fn reply(out: &mut Vec<u8>, payload: &[u8]) {
    out.extend_from_slice(&(-1i16).to_ne_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&(payload.len() as i32).to_ne_bytes());
    out.extend_from_slice(payload);
}
fn wire(capsule: &LiveObservationCapsule, nonce: &[u8]) -> Vec<u8> {
    let mut out = b"DFHack!\n".to_vec();
    out.extend_from_slice(&1i32.to_ne_bytes());
    for id in [2, 3] {
        let mut p = Vec::new();
        number(&mut p, 1, id);
        reply(&mut out, &p);
    }
    let mut p = Vec::new();
    number(&mut p, 1, 1);
    data(&mut p, 2, b"");
    data(&mut p, 3, b"");
    number(&mut p, 4, 1);
    number(&mut p, 5, 0);
    data(&mut p, 6, capsule.bridge.bridge_version.as_bytes());
    data(&mut p, 7, capsule.bridge.dfhack_version.as_bytes());
    data(&mut p, 8, capsule.bridge.df_version.as_bytes());
    number(&mut p, 9, 1);
    number(&mut p, 10, 1);
    data(&mut p, 11, nonce);
    number(&mut p, 12, capsule.bridge.bridge_generation);
    data(&mut p, 13, b"Handshake");
    data(&mut p, 13, b"ReadObservation");
    reply(&mut out, &p);
    for offset in 0..capsule.citizens.len() {
        let page = page(capsule, offset as u32, 1, capsule.names_included);
        let mut p = Vec::new();
        number(&mut p, 1, 1);
        data(&mut p, 2, b"");
        data(&mut p, 3, b"");
        number(&mut p, 4, 1);
        number(&mut p, 5, 0);
        data(&mut p, 6, nonce);
        number(&mut p, 7, page.bridge_generation);
        number(&mut p, 8, 1);
        number(&mut p, 9, 1);
        number(&mut p, 10, u64::from(page.paused));
        number(&mut p, 11, u64::from(page.current_year));
        number(&mut p, 12, u64::from(page.current_year_tick));
        data(&mut p, 13, page.world_name.as_bytes());
        data(&mut p, 14, page.world_folder.as_bytes());
        signed(&mut p, 15, page.site_id);
        number(&mut p, 16, u64::from(page.citizen_count_total));
        number(&mut p, 17, u64::from(page.citizen_offset));
        number(&mut p, 18, u64::from(page.complete));
        for u in &page.citizens {
            let mut item = Vec::new();
            signed(&mut item, 1, u.unit_id);
            data(&mut item, 2, u.name.as_bytes());
            data(&mut item, 3, u.race.as_bytes());
            signed(&mut item, 4, u.profession);
            signed(&mut item, 5, u.x);
            signed(&mut item, 6, u.y);
            signed(&mut item, 7, u.z);
            for (i, value) in [
                u.alive, u.sane, u.active, u.visible, u.citizen, u.resident, u.baby, u.child,
                u.adult,
            ]
            .iter()
            .enumerate()
            {
                number(&mut item, i as u32 + 8, u64::from(*value));
            }
            data(&mut p, 19, &item);
        }
        reply(&mut out, &p);
    }
    out
}
struct WireIo {
    input: Cursor<Vec<u8>>,
    output: Rc<RefCell<Vec<u8>>>,
    timeouts: Rc<RefCell<Vec<Duration>>>,
    closed: Rc<Cell<bool>>,
}
fn wire_config() -> crate::LiveConnectionConfig {
    crate::LiveConnectionConfig {
        endpoint: SocketAddr::from(([127, 0, 0, 1], 5000)),
        connect_timeout: Duration::from_secs(10),
        read_timeout: Duration::from_secs(10),
        write_timeout: Duration::from_secs(10),
        client_name: "semantic-citizen-test".to_owned(),
        client_version: "1".to_owned(),
    }
}
impl Read for WireIo {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.input.read(out)
    }
}
impl Write for WireIo {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.output.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl CitizenEvidenceIo for WireIo {
    fn narrow_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        self.timeouts.borrow_mut().push(timeout);
        Ok(())
    }
}
impl Drop for WireIo {
    fn drop(&mut self) {
        self.closed.set(true);
    }
}

#[test]
fn actual_v1_wire_codec_assembles_pages_and_closes_before_authorization() -> Result<()> {
    let (capsule, projection) = observation(&[0, 42])?;
    let c = context(&projection.snapshot);
    let output = Rc::new(RefCell::new(Vec::new()));
    let timeouts = Rc::new(RefCell::new(Vec::new()));
    let closed = Rc::new(Cell::new(false));
    let bytes = wire(&capsule, &[2; 16]);
    let wire_output = Rc::clone(&output);
    let wire_timeouts = Rc::clone(&timeouts);
    let wire_closed = Rc::clone(&closed);
    let factory = move |c: &OperationContext| {
        CitizenRpcSource::negotiate(
            WireIo {
                input: Cursor::new(bytes.clone()),
                output: Rc::clone(&wire_output),
                timeouts: Rc::clone(&wire_timeouts),
                closed: Rc::clone(&wire_closed),
            },
            crate::BridgeCredentials::new(vec![1; 32], vec![2; 16])?,
            &wire_config(),
            c,
        )
    };
    let proof_closed = Rc::clone(&closed);
    let authorizer = move |capsule: &LiveObservationCapsule,
                           projection: &LiveWorldProjection,
                           _: &OperationContext| {
        assert!(proof_closed.get());
        Ok(identity_policy(capsule, projection))
    };
    let owner =
        CitizenWorkforceEvidenceOwner::new(capsule, small_limits(), factory, authorizer, &c)?;
    assert_eq!(owner.routing_evidence()?.anchor(), c.anchor);
    assert!(output.borrow().starts_with(b"DFHack?\n"));
    assert!(timeouts.borrow().len() >= 10);
    assert!(timeouts.borrow().windows(2).all(|pair| pair[1] <= pair[0]));
    assert!(closed.get());
    Ok(())
}

#[test]
fn actual_wire_source_fences_after_revoked_authority_without_more_io() -> Result<()> {
    let (capsule, projection) = observation(&[42])?;
    let mut c = context(&projection.snapshot);
    let output = Rc::new(RefCell::new(Vec::new()));
    let io = WireIo {
        input: Cursor::new(wire(&capsule, &[2; 16])),
        output: Rc::clone(&output),
        timeouts: Rc::new(RefCell::new(Vec::new())),
        closed: Rc::new(Cell::new(false)),
    };
    let mut source = CitizenRpcSource::negotiate(
        io,
        crate::BridgeCredentials::new(vec![1; 32], vec![2; 16])?,
        &wire_config(),
        &c,
    )?;
    let before = output.borrow().len();
    c.grants.clear();
    assert!(source.read_page(0, 1, true, &c).is_err());
    assert!(
        source
            .read_page(0, 1, true, &context(&projection.snapshot))
            .is_err()
    );
    assert_eq!(output.borrow().len(), before);
    Ok(())
}

#[test]
fn actual_wire_io_retains_distinct_configured_read_and_write_caps() -> Result<()> {
    let (capsule, projection) = observation(&[42])?;
    let c = context(&projection.snapshot);
    let timeouts = Rc::new(RefCell::new(Vec::new()));
    let stream = WireIo {
        input: Cursor::new(wire(&capsule, &[2; 16])),
        output: Rc::new(RefCell::new(Vec::new())),
        timeouts: Rc::clone(&timeouts),
        closed: Rc::new(Cell::new(false)),
    };
    let mut config = wire_config();
    config.read_timeout = Duration::from_millis(37);
    config.write_timeout = Duration::from_millis(5);
    let mut source = CitizenRpcSource::negotiate(
        stream,
        crate::BridgeCredentials::new(vec![1; 32], vec![2; 16])?,
        &config,
        &c,
    )?;
    let observed = source.read_page(0, 1, true, &c)?;
    assert_eq!(observed.citizens[0].unit_id, 42);
    let values = timeouts.borrow();
    assert!(values.contains(&config.read_timeout));
    assert!(values.contains(&config.write_timeout));
    assert!(values.iter().all(|value| *value <= config.read_timeout));
    Ok(())
}
