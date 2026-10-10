use super::*;

include!("tests/fixture.rs");

#[path = "tests/evidence_owner.rs"]
mod evidence_owner;

#[test]
fn actual_coordinator_handoff_preserves_original_seal_and_leaves_goals_pending() -> Result<()> {
    let mut f = fixture()?;
    let review = prepared(&mut f)?;
    assert_ne!(review.seal(), review.native_plan().digest());
    assert_eq!(review.native_plan().key(), f.plan.steps[0].idempotency_key);
    assert_eq!(f.observer.refreshes, 3);
    assert!(
        f.owner
            .commit(review.seal(), false, &mut f.observer, &f.context, never)
            .is_err()
    );
    assert!(
        f.owner
            .commit(
                review.native_plan().digest(),
                true,
                &mut f.observer,
                &f.context,
                never
            )
            .is_err()
    );
    let result = f.owner.commit(
        review.seal(),
        true,
        &mut f.observer,
        &f.context,
        |binding, c| f.native.connect(binding, c),
    )?;
    assert!(matches!(
        result.action_result(),
        SingleLaborResult::Verified { .. }
    ));
    assert_eq!(
        result.native_record().effect().map(AssignmentEffect::phase),
        Some(AssignmentPhase::Applied)
    );
    assert_eq!(result.review().original_plan(), &f.plan);
    assert!(!result.original_goal_proven());
    assert_eq!(
        result.pending_obligation(),
        f.plan.steps[0].obligation.as_ref()
    );
    assert_eq!(f.observer.refreshes, 5);
    assert_eq!(f.native.data.borrow().calls[2], 1);
    let again = f
        .owner
        .commit(review.seal(), true, &mut f.observer, &f.context, never)?;
    assert!(matches!(
        again.action_result(),
        SingleLaborResult::Verified { .. }
    ));
    assert_eq!(f.native.data.borrow().calls[2], 1);
    Ok(())
}

#[test]
fn final_guard_runs_after_dispatch_sync_and_unknown_is_never_retry_permission() -> Result<()> {
    let mut f = fixture()?;
    let review = prepared(&mut f)?;
    f.observer.fail_at = Some(5);
    assert!(
        f.owner
            .commit(
                review.seal(),
                true,
                &mut f.observer,
                &f.context,
                |binding, c| f.native.connect(binding, c)
            )
            .is_err()
    );
    assert_eq!(f.native.data.borrow().calls[2], 0);
    let inventory = f.owner.inventory(&f.context)?;
    assert_eq!(
        inventory.records[0].state(),
        AssignmentState::DispatchStarted
    );
    assert!(
        f.owner
            .commit(review.seal(), true, &mut f.observer, &f.context, never)
            .is_err()
    );
    let result = f.owner.reconcile(review.seal(), &f.context, |binding, c| {
        f.native.connect(binding, c)
    })?;
    assert_eq!(result.action_result(), &SingleLaborResult::Unverified);
    assert!(!result.original_goal_proven());
    Ok(())
}

#[test]
fn broad_native_applied_and_permanent_unknown_do_not_complete_original_semantics() -> Result<()> {
    for unknown in [false, true] {
        let mut f = fixture()?;
        let review = prepared(&mut f)?;
        f.native.data.borrow_mut().unknown = unknown;
        f.native.data.borrow_mut().labors = vec![1, 1];
        let result = f.owner.commit(
            review.seal(),
            true,
            &mut f.observer,
            &f.context,
            |binding, c| f.native.connect(binding, c),
        )?;
        if unknown {
            assert_eq!(result.action_result(), &SingleLaborResult::Unverified);
            assert!(
                f.owner
                    .commit(review.seal(), true, &mut f.observer, &f.context, never)
                    .is_err()
            );
            let reread = f.owner.reconcile(review.seal(), &f.context, never)?;
            assert_eq!(reread.action_result(), &SingleLaborResult::Unverified);
        } else {
            assert!(matches!(
                result.action_result(),
                SingleLaborResult::AppliedOutsideSemantics { .. }
            ));
        }
        assert!(!result.original_goal_proven());
        assert_eq!(f.native.data.borrow().calls[2], 1);
    }
    Ok(())
}

#[test]
fn lost_commit_acknowledgement_reopens_and_reattaches_exact_original_for_query_only_recovery()
-> Result<()> {
    let mut f = fixture()?;
    let review = prepared(&mut f)?;
    f.native.data.borrow_mut().lose_commit = true;
    assert!(
        f.owner
            .commit(
                review.seal(),
                true,
                &mut f.observer,
                &f.context,
                |binding, c| f.native.connect(binding, c)
            )
            .is_err()
    );
    let mut next = f.context.clone();
    next.session_id = SessionId::new(2);
    next.anchor.tick = f.plan.expires_at_tick;
    next.grants
        .retain(|grant| grant.capability == Capability::Query);
    let journal = WorkforceJournal::open(
        f.native.journal.clone(),
        &next,
        WorkforceMode::Recover,
        None,
    )?;
    let (session, view) = WorkforceSession::new(journal, &next)?;
    let store = AssociationStore::open(f.native.associations.clone(), view.id, false, true, &next)?;
    let mut owner = SemanticWorkforceSession::new(session, store, &next)?;
    assert_eq!(owner.native.mode(), WorkforceMode::Recover);
    assert!(
        owner
            .commit(review.seal(), true, &mut f.observer, &next, never)
            .is_err()
    );
    let attached = owner.reattach(f.plan.clone(), &next)?;
    assert_eq!(attached.seal(), review.seal());
    assert!(
        owner
            .commit(attached.seal(), true, &mut f.observer, &next, never)
            .is_err()
    );
    let result = owner.reconcile(attached.seal(), &next, |binding, c| {
        assert!(
            c.grants
                .iter()
                .all(|grant| grant.capability == Capability::Query)
        );
        assert_eq!(c.anchor.tick, next.anchor.tick);
        f.native.connect(binding, c)
    })?;
    assert!(matches!(
        result.action_result(),
        SingleLaborResult::Verified { .. }
    ));
    assert!(!result.original_goal_proven());
    assert_eq!(f.native.data.borrow().calls[2], 1);
    Ok(())
}

#[test]
fn changed_preconditions_or_obligation_under_the_same_native_key_cannot_be_reattached() -> Result<()>
{
    let mut f = fixture()?;
    prepared(&mut f)?;
    let mut changed_predicate = f.plan.clone();
    changed_predicate.steps[0].preconditions = vec![Predicate::EntityExists(EntityId::new(43))];
    reseal(&mut changed_predicate);
    let mut changed_obligation = f.plan.clone();
    if let Some(obligation) = &mut changed_obligation.steps[0].obligation {
        obligation.poll_interval_ticks += 1;
    }
    reseal(&mut changed_obligation);
    for plan in [changed_predicate, changed_obligation] {
        assert_eq!(
            plan.steps[0].idempotency_key,
            f.plan.steps[0].idempotency_key
        );
        assert_ne!(plan.digest, f.plan.digest);
        assert!(f.owner.reattach(plan, &f.context).is_err());
    }
    assert_eq!(f.native.data.borrow().calls[2], 0);
    Ok(())
}

#[test]
fn missing_association_store_cannot_adopt_an_existing_native_key_or_other_journal() -> Result<()> {
    let mut f = fixture()?;
    let review = prepared(&mut f)?;
    let journal = WorkforceJournal::open(
        f.native.journal.clone(),
        &f.context,
        WorkforceMode::Control,
        None,
    )?;
    let (session, view) = WorkforceSession::new(journal, &f.context)?;
    let empty_store = AssociationStore::open(Storage::default(), view.id, true, false, &f.context)?;
    assert!(SemanticWorkforceSession::new(session, empty_store, &f.context).is_err());
    let other = WorkforceJournal::open(
        Storage::default(),
        &f.context,
        WorkforceMode::Control,
        Some((f.native.binding.clone(), [9; 32])),
    )?;
    let (other_session, _) = WorkforceSession::new(other, &f.context)?;
    let store = AssociationStore::open(
        f.native.associations.clone(),
        view.id,
        false,
        false,
        &f.context,
    )?;
    assert!(SemanticWorkforceSession::new(other_session, store, &f.context).is_err());
    assert!(
        f.owner
            .observe(f.plan.clone(), &mut f.observer, &f.context, never)
            .is_err()
    );
    assert!(
        f.owner
            .commit(review.seal(), true, &mut f.observer, &f.context, never)
            .is_err()
    );
    assert_eq!(f.native.data.borrow().calls[2], 0);
    Ok(())
}

#[test]
fn association_sync_failure_precedes_native_prepare_and_fences_the_owner() -> Result<()> {
    for fail_write in [false, true] {
        let mut f = fixture()?;
        let review = review(&mut f)?;
        if fail_write {
            f.native.associations.0.borrow_mut().fail_write = true;
        } else {
            f.native.associations.0.borrow_mut().fail_sync = true;
        }
        assert!(
            f.owner
                .prepare(review.seal(), &mut f.observer, &f.context, |binding, c| f
                    .native
                    .connect(binding, c))
                .is_err()
        );
        assert_eq!(f.native.data.borrow().calls[1], 0);
        assert!(
            f.owner
                .prepare(review.seal(), &mut f.observer, &f.context, never)
                .is_err()
        );
        f.native.associations.crash();
        let mut owner = reopen(&f.native, &f.context, f.native.associations.clone(), false)?;
        let fresh = owner.observe(f.plan.clone(), &mut f.observer, &f.context, |binding, c| {
            f.native.connect(binding, c)
        })?;
        owner.prepare(fresh.seal(), &mut f.observer, &f.context, |binding, c| {
            f.native.connect(binding, c)
        })?;
        assert_eq!(f.native.data.borrow().calls[1], 1);
    }
    Ok(())
}

#[test]
fn durable_association_before_native_intent_resumes_only_after_original_reobservation() -> Result<()>
{
    let mut f = fixture()?;
    let review = review(&mut f)?;
    f.native.data.borrow_mut().fail_connect = true;
    assert!(
        f.owner
            .prepare(review.seal(), &mut f.observer, &f.context, |binding, c| f
                .native
                .connect(binding, c))
            .is_err()
    );
    assert_eq!(f.native.data.borrow().calls[1], 0);
    f.native.data.borrow_mut().fail_connect = false;
    let mut owner = reopen(&f.native, &f.context, f.native.associations.clone(), false)?;
    assert!(owner.reattach(f.plan.clone(), &f.context).is_err());
    let fresh = owner.observe(f.plan.clone(), &mut f.observer, &f.context, |binding, c| {
        f.native.connect(binding, c)
    })?;
    assert_eq!(fresh.seal(), review.seal());
    owner.prepare(fresh.seal(), &mut f.observer, &f.context, |binding, c| {
        f.native.connect(binding, c)
    })?;
    Ok(())
}

#[test]
fn unsupported_controls_and_canonical_unit_scope_refuse_before_native_calls() -> Result<()> {
    let mut f = fixture()?;
    let mut checkpoint = f.plan.clone();
    checkpoint.requires_checkpoint = true;
    checkpoint
        .required_capabilities
        .insert(Capability::Checkpoint);
    reseal(&mut checkpoint);
    let mut compensate = f.plan.clone();
    compensate.steps[0].compensation = Some(labor(vec![EntityId::new(43)], false));
    reseal(&mut compensate);
    let mut outside = f.plan.clone();
    outside.steps[0].preconditions = vec![Predicate::FieldCompare {
        entity_id: EntityId::new(43),
        field: "inventory.stock".to_owned(),
        op: CompareOp::Eq,
        value: Value::U64(5),
    }];
    reseal(&mut outside);
    for plan in [checkpoint, compensate, outside] {
        assert!(
            f.owner
                .observe(plan, &mut f.observer, &f.context, never)
                .is_err()
        );
    }
    let mut narrowed = f.context.clone();
    for grant in &mut narrowed.grants {
        if grant.capability == Capability::ConfigureLabor {
            grant.scope.entity_ids = BTreeSet::from([EntityId::new(42)]);
        }
    }
    assert!(
        f.owner
            .observe(f.plan.clone(), &mut f.observer, &narrowed, never)
            .is_err()
    );
    let mut selected_only = f.context.clone();
    for grant in &mut selected_only.grants {
        if grant.capability == Capability::ConfigureLabor {
            grant.scope.entity_ids = BTreeSet::from([EntityId::new(43)]);
        }
    }
    // Exact selected scope can review the read, but the existing native/store
    // control APIs require whole-fortress authority and are never widened here.
    let selected_review = f.owner.observe(
        f.plan.clone(),
        &mut f.observer,
        &selected_only,
        |binding, c| f.native.connect(binding, c),
    )?;
    assert!(
        f.owner
            .prepare(
                selected_review.seal(),
                &mut f.observer,
                &selected_only,
                never,
            )
            .is_err()
    );
    assert_eq!(f.native.data.borrow().calls[1], 0);
    let review = prepared(&mut f)?;
    for denied in [&narrowed, &selected_only] {
        assert!(
            f.owner
                .prepare(review.seal(), &mut f.observer, denied, never)
                .is_err()
        );
        assert!(
            f.owner
                .commit(review.seal(), true, &mut f.observer, denied, never)
                .is_err()
        );
    }
    assert!(
        f.owner
            .prepare(review.seal(), &mut f.observer, &narrowed, never)
            .is_err()
    );
    assert!(
        f.owner
            .commit(review.seal(), true, &mut f.observer, &narrowed, never)
            .is_err()
    );
    assert_eq!(f.native.data.borrow().calls[1], 1);
    assert_eq!(f.native.data.borrow().calls[2], 0);
    Ok(())
}

#[test]
fn lost_store_custody_at_final_guard_blocks_native_effect_and_cannot_be_cached_away() -> Result<()>
{
    let mut f = fixture()?;
    let review = prepared(&mut f)?;
    f.observer.corrupt_at = Some((5, f.native.associations.clone()));
    assert!(
        f.owner
            .commit(
                review.seal(),
                true,
                &mut f.observer,
                &f.context,
                |binding, c| f.native.connect(binding, c)
            )
            .is_err()
    );
    assert_eq!(f.native.data.borrow().calls[2], 0);
    assert!(f.owner.inspect(review.seal(), &f.context).is_err());
    Ok(())
}

#[test]
fn aggregate_byte_budget_and_expired_original_plan_refuse_before_dispatch() -> Result<()> {
    let mut f = fixture()?;
    let mut tiny = f.context.clone();
    tiny.budget.max_bytes = 4 * 1024 * 1024;
    assert!(
        f.owner
            .observe(f.plan.clone(), &mut f.observer, &tiny, never)
            .is_err()
    );
    assert_eq!(f.native.data.borrow().calls[0], 0);
    let review = prepared(&mut f)?;
    let mut expired = f.context.clone();
    expired.anchor.tick = f.plan.expires_at_tick;
    assert!(
        f.owner
            .commit(review.seal(), true, &mut f.observer, &expired, never)
            .is_err()
    );
    assert_eq!(f.native.data.borrow().calls[2], 0);
    Ok(())
}

#[test]
fn final_evidence_refresh_consumes_the_native_deadline_without_replenishing_bytes() -> Result<()> {
    let mut f = fixture()?;
    let review = prepared(&mut f)?;
    let start = f.native.data.borrow().edges.len();
    f.observer.delay_at = Some(5);
    f.owner.commit(
        review.seal(),
        true,
        &mut f.observer,
        &f.context,
        |binding, c| f.native.connect(binding, c),
    )?;
    let data = f.native.data.borrow();
    let edges = &data.edges[start..];
    let observation = edges.iter().find(|edge| edge.0 == 0).ok_or_else(custody)?;
    let commit = edges.iter().find(|edge| edge.0 == 2).ok_or_else(custody)?;
    assert!(commit.1 + 20 <= observation.1);
    assert_eq!(commit.2, RPC_BYTES);
    assert_eq!(commit.3, f.context.anchor.tick);
    assert!(
        edges
            .iter()
            .any(|edge| edge.0 == 5 && edge.2 == CONNECT_BYTES)
    );
    assert_eq!(data.calls[2], 1);
    Ok(())
}

#[test]
fn reserved_edges_keep_native_bytes_and_current_clock_and_reauthorize() -> Result<()> {
    let f = fixture()?;
    let mut call = Call::new(&f.context)?;
    call.deadline = Instant::now()
        .checked_add(Duration::from_millis(500))
        .ok_or_else(exhausted)?;
    let mut edge = f.context.clone();
    edge.budget.max_bytes = RPC_BYTES;
    edge.anchor.tick = GameTick(180);
    let bytes = call.remaining;
    let current = call.edge(&edge)?;
    assert_eq!(current.budget.max_bytes, RPC_BYTES);
    assert!(current.budget.max_wall_millis <= 500);
    assert_eq!(current.anchor.tick, GameTick(180));
    assert_eq!(call.remaining, bytes);
    edge.anchor.tick = GameTick(120);
    assert_eq!(call.edge(&edge)?.anchor.tick, GameTick(180));
    for grant in &mut edge.grants {
        if grant.capability == Capability::Query {
            grant.expires_at_tick = Some(GameTick(179));
        }
    }
    assert!(call.edge(&edge).is_err());
    Ok(())
}

#[test]
fn cancellation_keeps_unit_scope_at_the_native_boundary_and_can_clean_up_after_expiry() -> Result<()>
{
    let mut f = fixture()?;
    let review = prepared(&mut f)?;
    // Directly exercise the last scope boundary, beyond the native family's
    // own outer authorization, without invoking its cancellation setter.
    let mut denied = f.context.clone();
    for grant in &mut denied.grants {
        if grant.capability == Capability::ConfigureLabor {
            grant.scope.entity_ids = BTreeSet::from([EntityId::new(42)]);
        }
    }
    let mut call = Call::new(&f.context)?;
    {
        let mut scoped = Scoped {
            native: f.native.clone(),
            original: &review.original,
            association: Some(&review.association),
            associations: &mut f.owner.associations,
            call: &mut call,
        };
        assert!(scoped.cancel(&review.native, &denied).is_err());
    }
    assert_eq!(f.native.data.borrow().calls[4], 0);
    f.observer.fail_at = Some(5);
    assert!(
        f.owner
            .commit(
                review.seal(),
                true,
                &mut f.observer,
                &f.context,
                |binding, c| f.native.connect(binding, c)
            )
            .is_err()
    );
    assert_eq!(
        f.owner.inventory(&f.context)?.records[0].state(),
        AssignmentState::DispatchStarted
    );
    let mut expired = f.context.clone();
    expired.anchor.tick = f.plan.expires_at_tick;
    denied.anchor.tick = expired.anchor.tick;
    assert!(f.owner.cancel(review.seal(), &denied, never).is_err());
    let refreshes = f.observer.refreshes;
    let result = f.owner.cancel(review.seal(), &expired, |binding, c| {
        f.native.connect(binding, c)
    })?;
    assert_eq!(
        result.native_record().effect().map(AssignmentEffect::phase),
        Some(AssignmentPhase::Cancelled)
    );
    assert_eq!(result.action_result(), &SingleLaborResult::Unverified);
    assert!(!result.original_goal_proven());
    assert_eq!(f.observer.refreshes, refreshes);
    assert_eq!(f.native.data.borrow().calls[2], 0);
    assert_eq!(f.native.data.borrow().calls[4], 1);
    Ok(())
}

#[test]
fn readonly_semantic_custody_never_controls_even_with_a_native_control_session() -> Result<()> {
    let mut f = fixture()?;
    let review = prepared(&mut f)?;
    let mut owner = reopen(&f.native, &f.context, f.native.associations.clone(), true)?;
    assert_eq!(owner.native.mode(), WorkforceMode::Control);
    assert!(owner.associations.is_read_only());
    let attached = owner.reattach(f.plan.clone(), &f.context)?;
    assert_eq!(attached.seal(), review.seal());
    owner.inspect(attached.seal(), &f.context)?;
    assert!(
        owner
            .prepare(attached.seal(), &mut f.observer, &f.context, never)
            .is_err()
    );
    assert!(
        owner
            .commit(attached.seal(), true, &mut f.observer, &f.context, never)
            .is_err()
    );
    assert!(owner.cancel(attached.seal(), &f.context, never).is_err());
    owner.reconcile(attached.seal(), &f.context, never)?;
    assert_eq!(f.native.data.borrow().calls, [2, 1, 0, 0, 0]);
    Ok(())
}

#[test]
fn immutable_associations_bound_capacity_and_read_only_control_are_enforced() -> Result<()> {
    let mut f = fixture()?;
    let review = review(&mut f)?;
    let id = f.owner.associations.native_journal();
    let storage = Storage::default();
    let mut store = AssociationStore::open(storage.clone(), id, true, false, &f.context)?;
    store.retain(review.association.clone(), &f.context)?;
    let length = store.byte_len();
    store.retain(review.association.clone(), &f.context)?;
    assert_eq!(store.byte_len(), length);
    let mut changed = review.association.clone();
    changed.semantic = Digest32::of_bytes(b"different original obligation");
    assert!(store.retain(changed, &f.context).is_err());
    for index in 1..64 {
        let mut record = review.association.clone();
        record.key = format!("bounded_{index}");
        store.retain(record, &f.context)?;
    }
    let mut extra = review.association.clone();
    extra.key = "bounded_overflow".to_owned();
    assert!(store.retain(extra, &f.context).is_err());
    let mut query = f.context.clone();
    query
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    let mut readonly = AssociationStore::open(storage, id, false, true, &query)?;
    readonly.verify(&query)?;
    assert!(readonly.retain(review.association, &f.context).is_err());
    Ok(())
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn real_private_association_file_uses_existing_custody_and_query_only_recovery() -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
    let mut f = fixture()?;
    let review = review(&mut f)?;
    let id = f.owner.associations.native_journal();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| invalid("fixture clock"))?
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "dfmcp-semantic-workforce-{}-{nonce}",
        std::process::id()
    ));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .map_err(|_| invalid("fixture directory"))?;
    let path = directory.join("association.bin");
    let mut store = open_private_association_store(&path, id, true, false, &f.context)?;
    assert!(!store.is_read_only());
    let header_len = store.byte_len();
    store.retain(review.association.clone(), &f.context)?;
    store.verify(&f.context)?;
    assert!(store.byte_len() > header_len);
    let durable_bytes = std::fs::read(&path).map_err(|_| invalid("fixture durable read"))?;
    assert_eq!(durable_bytes.len(), store.byte_len());
    assert!(open_private_association_store(&path, id, false, true, &f.context).is_err());
    drop(store);
    let mut query = f.context.clone();
    query.session_id = SessionId::new(2);
    query
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    let mut readonly = open_private_association_store(&path, id, false, true, &query)?;
    assert!(readonly.is_read_only());
    readonly.verify(&query)?;
    assert_eq!(
        readonly.get(&review.association.key),
        Some(&review.association)
    );
    assert_eq!(readonly.byte_len(), durable_bytes.len());
    assert_eq!(
        readonly
            .get(&review.association.key)
            .map(|value| value.seal(id)),
        Some(review.seal())
    );
    drop(readonly);
    assert_eq!(
        std::fs::read(&path).map_err(|_| invalid("fixture recovery read"))?,
        durable_bytes
    );
    assert!(
        open_private_association_store(
            &path,
            Digest32::of_bytes(b"other native journal"),
            false,
            true,
            &query
        )
        .is_err()
    );
    let missing = directory.join("missing.bin");
    assert!(open_private_association_store(&missing, id, false, true, &query).is_err());
    assert!(!missing.exists());
    assert!(open_private_association_store(&path, id, true, false, &f.context).is_err());
    assert_eq!(
        std::fs::read(&path).map_err(|_| invalid("fixture immutable read"))?,
        durable_bytes
    );
    let link = directory.join("link.bin");
    symlink(&path, &link).map_err(|_| invalid("fixture symlink"))?;
    assert!(open_private_association_store(&link, id, false, true, &query).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .map_err(|_| invalid("fixture mode"))?;
    assert!(open_private_association_store(&path, id, false, true, &query).is_err());
    std::fs::remove_dir_all(&directory).map_err(|_| invalid("fixture cleanup"))?;
    Ok(())
}

#[test]
fn association_replay_refuses_every_partial_frame_and_corruption_without_repair() -> Result<()> {
    fn refuse(bytes: Vec<u8>, id: Digest32, context: &OperationContext) -> Result<()> {
        let storage = Storage::from_bytes(bytes.clone());
        let error = match AssociationStore::open(storage.clone(), id, false, false, context) {
            Err(error) => error,
            Ok(_) => return Err(invalid("association replay accepted corrupt history")),
        };
        assert_eq!(error.code, ErrorCode::CorruptLedger);
        let memory = storage.0.borrow();
        assert_eq!(memory.file.get_ref(), &bytes);
        assert_eq!(memory.synced, bytes);
        Ok(())
    }

    let mut f = fixture()?;
    let review = review(&mut f)?;
    let id = f.owner.associations.native_journal();
    let storage = Storage::default();
    let mut store = AssociationStore::open(storage.clone(), id, true, false, &f.context)?;
    let header_len = store.byte_len();
    store.retain(review.association.clone(), &f.context)?;
    let valid = storage.0.borrow().synced.clone();
    drop(store);

    // Every interrupted publication except the already complete empty header
    // is refused. No call may silently truncate an incomplete association.
    for end in 0..valid.len() {
        if end != header_len {
            refuse(valid[..end].to_vec(), id, &f.context)?;
        }
    }
    for offset in 0..valid.len() {
        let mut changed = valid.clone();
        changed[offset] ^= 1;
        refuse(changed, id, &f.context)?;
    }

    // A second fully checksummed frame must still not replay the same key.
    // Correct framing and a valid chain cannot authorize a mutable association.
    let mut duplicate = valid[header_len..].to_vec();
    let digest_start = duplicate.len() - 40;
    let first_digest = duplicate[digest_start..digest_start + 32].to_vec();
    duplicate[12..20].copy_from_slice(&2u64.to_be_bytes());
    duplicate[20..52].copy_from_slice(&first_digest);
    let digest = crate::bounded_run::hash(
        b"dfmcp-semantic-workforce-frame/1",
        &duplicate[..digest_start],
    );
    duplicate[digest_start..digest_start + 32].copy_from_slice(digest.as_bytes());
    let mut rebound = valid;
    rebound.extend_from_slice(&duplicate);
    refuse(rebound, id, &f.context)
}

#[test]
fn complete_association_after_failed_sync_requires_online_durability_promotion() -> Result<()> {
    let mut f = fixture()?;
    let review = review(&mut f)?;
    let association = review.association.clone();
    let id = f.owner.associations.native_journal();
    let storage = Storage::default();
    let mut store = AssociationStore::open(storage.clone(), id, true, false, &f.context)?;
    let durable_header = storage.0.borrow().synced.clone();
    storage.0.borrow_mut().fail_sync = true;
    let error = store
        .retain(association.clone(), &f.context)
        .err()
        .ok_or_else(|| invalid("association sync failure was acknowledged"))?;
    assert_eq!(error.code, ErrorCode::CorruptLedger);
    assert!(store.is_fenced());
    assert_eq!(store.byte_len(), durable_header.len());
    assert!(store.get(&association.key).is_none());
    let surviving = storage.0.borrow().file.get_ref().clone();
    assert!(surviving.len() > durable_header.len());
    assert_eq!(storage.0.borrow().synced, durable_header);
    drop(store);

    // Do not simulate a power loss here: complete but unacknowledged bytes may
    // survive in the file. Read-only replay must not call the failing sync.
    let mut next = f.context.clone();
    next.session_id = SessionId::new(2);
    let mut query = next.clone();
    query
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    let mut readonly = AssociationStore::open(storage.clone(), id, false, true, &query)?;
    assert!(readonly.is_read_only());
    readonly.verify(&query)?;
    assert_eq!(readonly.get(&association.key), Some(&association));
    assert!(readonly.retain(association.clone(), &next).is_err());
    assert_eq!(storage.0.borrow().synced, durable_header);
    drop(readonly);

    // Online reopening cannot acknowledge durability while sync still fails.
    assert!(AssociationStore::open(storage.clone(), id, false, false, &query).is_err());
    assert_eq!(storage.0.borrow().synced, durable_header);
    storage.0.borrow_mut().fail_sync = false;
    let mut promoted = AssociationStore::open(storage.clone(), id, false, false, &query)?;
    assert!(!promoted.is_read_only());
    promoted.verify(&query)?;
    assert_eq!(storage.0.borrow().synced, surviving);
    assert_eq!(promoted.get(&association.key), Some(&association));
    // Reopening restores neither Plan nor ConfigureLabor authority. A current
    // grant is independently necessary even for an exact retention retry.
    assert!(promoted.retain(association.clone(), &query).is_err());
    promoted.retain(association, &next)?;
    assert_eq!(storage.0.borrow().file.get_ref(), &surviving);
    assert_eq!(storage.0.borrow().synced, surviving);
    Ok(())
}

#[test]
fn association_post_sync_byte_mismatch_fences_before_publishing_the_new_root() -> Result<()> {
    use std::cell::Cell;

    struct CorruptAfterSync(Storage, Rc<Cell<bool>>);
    impl Read for CorruptAfterSync {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            self.0.read(out)
        }
    }
    impl Write for CorruptAfterSync {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.write(bytes)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
    impl Seek for CorruptAfterSync {
        fn seek(&mut self, at: SeekFrom) -> io::Result<u64> {
            self.0.seek(at)
        }
    }
    impl EffectJournalStorage for CorruptAfterSync {
        fn sync(&mut self) -> io::Result<()> {
            self.0.sync()?;
            if self.1.get() {
                let mut memory = self.0.0.borrow_mut();
                let last = memory
                    .file
                    .get_mut()
                    .last_mut()
                    .ok_or_else(|| io::Error::other("missing synchronized association"))?;
                *last ^= 1;
            }
            Ok(())
        }
        fn truncate(&mut self, length: u64) -> io::Result<()> {
            self.0.truncate(length)
        }
        fn validate_identity(&self) -> io::Result<()> {
            self.0.validate_identity()
        }
    }

    let mut f = fixture()?;
    let review = review(&mut f)?;
    let association = review.association;
    let id = f.owner.associations.native_journal();
    let storage = Storage::default();
    let armed = Rc::new(Cell::new(false));
    let mut store = AssociationStore::open(
        CorruptAfterSync(storage.clone(), armed.clone()),
        id,
        true,
        false,
        &f.context,
    )?;
    let old_len = store.byte_len();
    armed.set(true);
    let error = store
        .retain(association.clone(), &f.context)
        .err()
        .ok_or_else(|| invalid("post-sync corruption was acknowledged"))?;
    assert_eq!(error.code, ErrorCode::CorruptLedger);
    assert!(store.is_fenced());
    assert_eq!(store.byte_len(), old_len);
    assert!(store.get(&association.key).is_none());
    assert!(store.verify(&f.context).is_err());
    drop(store);
    // Durable evidence survives the failed publication; it can only become a
    // usable root through a new complete verified replay.
    storage.crash();
    let recovered = AssociationStore::open(storage, id, false, false, &f.context)?;
    assert_eq!(recovered.get(&association.key), Some(&association));
    Ok(())
}
