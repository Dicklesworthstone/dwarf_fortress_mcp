#[allow(dead_code, unused_imports)]
mod actual {
    // Reuse the checked-in strict workforce codec, source and durable storage
    // fixture. Both coordinators below are the ordinary production owners.
    include!("../semantic_workforce/tests/fixture.rs");
    use crate::build_placement::journal::BuildMode;
    use crate::live_dispatch::durable::{Coordinator, DurableFamilyExecutor, RecoveredFamily};
    use crate::live_dispatch::{FamilyPreparation, LiveFamilyExecutor, ReconcileOutcome};
    use crate::live_production::{Executor, StepEvidence, WorkforceFactory, validate_plan};
    use crate::live_routing::{LiveFamily, LiveRequest, RoutedStep, route_step};
    use crate::semantic_work_orders::{SemanticWorkOrderSession, WorkOrderEvidenceOwner};
    use crate::work_order_control::session::WorkOrderSession;
    use crate::work_order_control::{
        CreationState, JournalMode, WorkOrderJournal, WorkOrderQuerySource, WorkOrderSource,
    };
    use crate::work_orders::rpc::WorkOrderManifest;
    use crate::work_orders::{
        WorkOrderEffect, WorkOrderObservation, WorkOrderPlan, WorkOrderState,
    };
    use std::collections::BTreeMap;

    fn order_queue() -> Result<WorkOrderObservation> {
        let mut bytes = b"DFMWO010".to_vec();
        for value in [53u64, 11, 120] {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        bytes.extend_from_slice(&100u32.to_be_bytes());
        bytes.extend_from_slice(&4u32.to_be_bytes());
        bytes.push(1);
        text(&mut bytes, "region1");
        bytes.extend_from_slice(&0u32.to_be_bytes());
        WorkOrderObservation::decode(&bytes)
    }
    fn order_after(p: &WorkOrderPlan) -> Result<WorkOrderObservation> {
        let o = p.observation();
        let mut bytes = o.canonical_bytes().to_vec();
        bytes[16..24].copy_from_slice(&(o.sequence() + 1).to_be_bytes());
        bytes[32..36].copy_from_slice(&(o.next_order_id() + 1).to_be_bytes());
        let at = 43 + o.world_folder().len();
        bytes[at..at + 4].copy_from_slice(&((o.order_ids().len() + 1) as u32).to_be_bytes());
        bytes.extend_from_slice(&o.next_order_id().to_be_bytes());
        WorkOrderObservation::decode(&bytes)
    }
    fn order_effect(p: &WorkOrderPlan, state: WorkOrderState) -> Result<WorkOrderEffect> {
        let o = p.observation();
        let mut bytes = b"DFMWOE10".to_vec();
        for n in [o.generation(), o.sequence(), o.tick()] {
            bytes.extend_from_slice(&n.to_be_bytes());
        }
        bytes.extend_from_slice(&o.next_order_id().to_be_bytes());
        bytes.push(p.spec().recipe() as u8);
        bytes.extend_from_slice(&p.spec().amount().to_be_bytes());
        bytes.extend_from_slice(o.witness().as_bytes());
        bytes.extend_from_slice(p.digest().as_bytes());
        bytes.extend_from_slice(p.prepare_token());
        let created = state == WorkOrderState::Created;
        bytes.extend_from_slice(&[state as u8, u8::from(created)]);
        bytes.extend_from_slice(&(if created { o.tick() } else { 0 }).to_be_bytes());
        let (after, config) = if created {
            let mut config = b"DFMWOC10".to_vec();
            config.extend_from_slice(&o.next_order_id().to_be_bytes());
            config.push(p.spec().recipe() as u8);
            config.extend_from_slice(&p.spec().amount().to_be_bytes());
            (order_after(p)?.witness(), Digest32::of_bytes(&config))
        } else {
            (Digest32::ZERO, Digest32::ZERO)
        };
        bytes.extend_from_slice(after.as_bytes());
        bytes.extend_from_slice(config.as_bytes());
        let receipt = if state.terminal() {
            let mut proof = b"dfmcp-work-order-receipt/1\0".to_vec();
            proof.extend_from_slice(&o.generation().to_be_bytes());
            text(&mut proof, p.key());
            proof.extend_from_slice(&bytes[73..195]);
            Digest32::of_bytes(&proof)
        } else {
            Digest32::ZERO
        };
        bytes.extend_from_slice(receipt.as_bytes());
        text(&mut bytes, p.key());
        WorkOrderEffect::decode(&bytes, p)
    }
    struct Orders {
        queue: WorkOrderObservation,
        retained: BTreeMap<String, WorkOrderEffect>,
        reads: usize,
        prepares: usize,
        commits: usize,
        queries: usize,
        lose_reply: bool,
        lose_prepare_reply: bool,
        refuse: bool,
    }
    #[derive(Clone)]
    struct OrderNative {
        state: Rc<RefCell<Orders>>,
        journal: Storage,
        associations: Storage,
        context: OperationContext,
        manifest: WorkOrderManifest,
        fenced: bool,
    }
    impl OrderNative {
        fn new(
            state: Rc<RefCell<Orders>>,
            journal: Storage,
            associations: Storage,
            c: &OperationContext,
        ) -> Self {
            Self {
                manifest: WorkOrderManifest {
                    generation: state.borrow().queue.generation(),
                    df_version: "0.51.11".into(),
                    dfhack_version: "0.51.11-r1".into(),
                },
                state: Rc::clone(&state),
                journal,
                associations,
                context: c.clone(),
                fenced: false,
            }
        }
        fn check(&self) -> Result<()> {
            if self.fenced {
                Err(invalid("fenced order source"))
            } else {
                Ok(())
            }
        }
        fn durable_dispatch(&self, plan: &WorkOrderPlan) -> Result<()> {
            let journal = WorkOrderJournal::open(
                Storage::from_bytes(self.journal.0.borrow().synced.clone()),
                &self.context,
                JournalMode::Offline,
                false,
            )?;
            let summary = journal.summary(&self.context)?;
            let page = journal.records_page(summary.head, None, 64, &self.context)?;
            let record = page
                .records
                .iter()
                .find(|r| r.plan().key() == plan.key())
                .ok_or_else(|| invalid("missing synced order dispatch"))?;
            assert_eq!(record.state(), CreationState::DispatchStarted);
            let associations = crate::semantic_work_orders::AssociationStore::open(
                Storage::from_bytes(self.associations.0.borrow().synced.clone()),
                summary.journal_id,
                false,
                true,
                &self.context,
            )?;
            let owner = WorkOrderSession::new(journal, None::<OrderNative>, &self.context)?;
            SemanticWorkOrderSession::new(owner, associations, &self.context)?;
            Ok(())
        }
    }
    impl WorkOrderQuerySource for OrderNative {
        fn manifest(&self) -> &WorkOrderManifest {
            &self.manifest
        }
        fn fence(&mut self) {
            self.fenced = true;
        }
        fn read_orders(&mut self, timeout: Duration) -> Result<WorkOrderObservation> {
            self.check()?;
            assert!(timeout <= Duration::from_secs(10));
            let mut s = self.state.borrow_mut();
            s.reads += 1;
            Ok(s.queue.clone())
        }
        fn query(&mut self, p: &WorkOrderPlan, _: Duration) -> Result<Option<WorkOrderEffect>> {
            self.check()?;
            let mut s = self.state.borrow_mut();
            s.queries += 1;
            Ok(s.retained.get(p.key()).cloned())
        }
    }
    impl WorkOrderSource for OrderNative {
        fn prepare(&mut self, p: &WorkOrderPlan, _: Duration) -> Result<WorkOrderEffect> {
            self.check()?;
            let mut s = self.state.borrow_mut();
            s.prepares += 1;
            assert_eq!(&s.queue, p.observation());
            let value = order_effect(p, WorkOrderState::Prepared)?;
            s.retained.insert(p.key().into(), value.clone());
            if s.lose_prepare_reply {
                Err(invalid("lost prepare reservation reply"))
            } else {
                Ok(value)
            }
        }
        fn commit(
            &mut self,
            p: &WorkOrderPlan,
            prepared: &WorkOrderEffect,
            _: Duration,
        ) -> Result<WorkOrderEffect> {
            self.check()?;
            self.durable_dispatch(p)?;
            assert_eq!(prepared.state(), WorkOrderState::Prepared);
            let mut s = self.state.borrow_mut();
            s.commits += 1;
            assert_eq!(&s.queue, p.observation());
            let value = order_effect(
                p,
                if s.refuse {
                    WorkOrderState::Refused
                } else {
                    WorkOrderState::Created
                },
            )?;
            if !s.refuse {
                s.queue = order_after(p)?;
            }
            s.retained.insert(p.key().into(), value.clone());
            if s.lose_reply {
                Err(invalid("lost order commit reply"))
            } else {
                Ok(value)
            }
        }
    }
    #[derive(Default)]
    struct EvidenceHooks {
        orders: usize,
        deny_at: Option<usize>,
        corrupt_at: Option<(usize, Storage)>,
    }
    struct CombinedEvidence {
        inner: Observer,
        hooks: Rc<RefCell<EvidenceHooks>>,
    }
    impl WorkforceEvidenceOwner for CombinedEvidence {
        fn refresh_after_capture(
            &mut self,
            capture: &WorkforceCapture,
            c: &OperationContext,
        ) -> Result<()> {
            self.inner.refresh_after_capture(capture, c)
        }
        fn routing_evidence(&self) -> Result<LiveRoutingEvidence<'_>> {
            self.inner.routing_evidence()
        }
    }
    impl WorkOrderEvidenceOwner for CombinedEvidence {
        fn refresh_after_observation(
            &mut self,
            observation: &WorkOrderObservation,
            c: &OperationContext,
        ) -> Result<()> {
            c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
            assert_eq!(
                observation.tick(),
                self.inner.projection.snapshot.tick.get()
            );
            assert_eq!(c.anchor, self.inner.projection.snapshot.anchor());
            let mut hooks = self.hooks.borrow_mut();
            hooks.orders += 1;
            if hooks.deny_at == Some(hooks.orders) {
                return Err(invalid("denied order evidence"));
            }
            if let Some((at, storage)) = &hooks.corrupt_at
                && *at == hooks.orders
            {
                if let Some(byte) = storage.0.borrow_mut().file.get_mut().last_mut() {
                    *byte ^= 1;
                }
            }
            Ok(())
        }
        fn routing_evidence(&self) -> Result<LiveRoutingEvidence<'_>> {
            self.inner.routing_evidence()
        }
    }
    struct Factory(Native);
    impl WorkforceFactory for Factory {
        type Source = Native;
        fn connect(&mut self, binding: &WorkforceBinding, c: &OperationContext) -> Result<Native> {
            self.0.connect(binding, c)
        }
    }
    type Combined =
        Executor<Storage, Storage, Storage, Storage, OrderNative, CombinedEvidence, Factory>;
    fn original_plan(snapshot: &WorldSnapshot, orders: usize) -> Result<PreparedPlan> {
        let intent = IntentId::new(501);
        let mut steps = Vec::new();
        let labor = labor(vec![EntityId::new(43)], true);
        let post = Predicate::FieldCompare {
            entity_id: EntityId::new(43),
            field: "labor.MINE".into(),
            op: CompareOp::Eq,
            value: Value::Bool(true),
        };
        steps.push(PlanStep {
            id: StepId::ZERO,
            idempotency_key: derive_step_idempotency_key(
                intent,
                snapshot.anchor(),
                StepId::ZERO,
                &labor,
            ),
            required_capability: labor.capability(),
            risk: labor.risk(),
            action: labor,
            preconditions: vec![Predicate::Paused(true)],
            postconditions: vec![post],
            compensation: None,
            obligation: None,
            depends_on: Vec::new(),
        });
        for index in 1..=orders {
            let action = Action::CreateWorkOrder {
                name: format!("original quota {index}"),
                job_token: "CONSTRUCT_BED".into(),
                amount: 2,
                conditions: Vec::new(),
            }
            .normalized();
            let id = StepId::new(index as u32);
            let goal = Predicate::EntityExists(EntityId::new(9999 + index as u64));
            steps.push(PlanStep {
                id,
                idempotency_key: derive_step_idempotency_key(
                    intent,
                    snapshot.anchor(),
                    id,
                    &action,
                ),
                required_capability: action.capability(),
                risk: action.risk(),
                action,
                preconditions: vec![Predicate::Paused(true)],
                postconditions: vec![goal.clone()],
                compensation: None,
                obligation: Some(ObligationSpec {
                    terminal: goal,
                    failure: None,
                    deadline_tick: GameTick(220),
                    poll_interval_ticks: 5,
                    stable_for_observations: 2,
                }),
                depends_on: vec![StepId::ZERO],
            });
        }
        let risk = steps
            .iter()
            .map(|step| step.risk)
            .max()
            .ok_or_else(|| invalid("fixture original plan has no steps"))?;
        let caps = steps.iter().map(|step| step.required_capability).collect();
        Ok(PreparedPlan::builder(
            intent,
            snapshot.anchor(),
            "Assign original miners then queue exact furniture orders",
            Predicate::EntityExists(EntityId::new(55555)),
        )
        .max_risk(risk)
        .required_capabilities(caps)
        .expires_at_tick(GameTick(200))
        .steps(steps)
        .build())
    }
    struct ProductionFixture {
        executor: Combined,
        plan: PreparedPlan,
        c: OperationContext,
        workforce: Native,
        orders: Rc<RefCell<Orders>>,
        order_journal: Storage,
        order_associations: Storage,
        hooks: Rc<RefCell<EvidenceHooks>>,
    }
    impl ProductionFixture {
        fn new(count: usize) -> Result<Self> {
            let Fixture {
                owner,
                mut native,
                observer,
                mut context,
                ..
            } = fixture()?;
            context.budget.max_bytes = 2 * 1024 * 1024 * 1024;
            context.grants.push(CapabilityGrant {
                capability: Capability::ConfigureProduction,
                scope: CapabilityScope {
                    fortress_id: Some(context.anchor.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::Guarded,
                expires_at_tick: None,
                remaining_uses: None,
            });
            native.context = context.clone();
            let plan = original_plan(&observer.projection.snapshot, count)?;
            plan.validate_structure()?;
            let orders = Rc::new(RefCell::new(Orders {
                queue: order_queue()?,
                retained: BTreeMap::new(),
                reads: 0,
                prepares: 0,
                commits: 0,
                queries: 0,
                lose_reply: false,
                lose_prepare_reply: false,
                refuse: false,
            }));
            let order_journal = Storage::default();
            let order_associations = Storage::default();
            let journal = WorkOrderJournal::open(
                order_journal.clone(),
                &context,
                JournalMode::Control,
                true,
            )?;
            let id = journal.summary(&context)?.journal_id;
            let associations = crate::semantic_work_orders::AssociationStore::open(
                order_associations.clone(),
                id,
                true,
                false,
                &context,
            )?;
            let order_native = OrderNative::new(
                Rc::clone(&orders),
                order_journal.clone(),
                order_associations.clone(),
                &context,
            );
            let session = WorkOrderSession::new(journal, Some(order_native), &context)?;
            let order_owner = SemanticWorkOrderSession::new(session, associations, &context)?;
            let hooks = Rc::new(RefCell::new(EvidenceHooks::default()));
            let evidence = CombinedEvidence {
                inner: observer,
                hooks: Rc::clone(&hooks),
            };
            let executor = Executor::new(
                plan.clone(),
                owner,
                order_owner,
                Some(evidence),
                Factory(native.clone()),
                &context,
            )?;
            Ok(Self {
                executor,
                plan,
                c: context,
                workforce: native,
                orders,
                order_journal,
                order_associations,
                hooks,
            })
        }
        fn route(&self, index: usize) -> Result<RoutedStep> {
            let step = self
                .plan
                .steps
                .get(index)
                .ok_or_else(|| invalid("fixture step is absent"))?;
            route_step(step)
                .outcome
                .map_err(|cause| invalid(cause.reason))
        }
        fn prepare(&mut self, index: usize) -> Result<FamilyPreparation> {
            let route = self.route(index)?;
            self.executor
                .prepare(&self.plan, &self.plan.steps[index], &route, &self.c)
        }
        fn commit(
            &mut self,
            index: usize,
            prepared: FamilyPreparation,
        ) -> Result<crate::live_dispatch::FamilyReceipt> {
            let route = self.route(index)?;
            self.executor.commit(
                &self.plan,
                &self.plan.steps[index],
                &route,
                prepared,
                &self.c,
            )
        }
        fn applied(&mut self, index: usize) -> Result<()> {
            let prepared = self.prepare(index)?;
            self.commit(index, prepared)?;
            Ok(())
        }
        fn reopen_query(self) -> Result<Self> {
            self.reopen(true)
        }
        fn reopen(self, query_only: bool) -> Result<Self> {
            let Self {
                executor,
                plan,
                mut c,
                mut workforce,
                orders,
                order_journal,
                order_associations,
                hooks,
            } = self;
            let Executor {
                evidence,
                workforce: old_workforce,
                orders: old_orders,
                ..
            } = executor;
            drop((old_workforce, old_orders));
            c.session_id = SessionId::new(2);
            if query_only {
                c.grants.retain(|g| g.capability == Capability::Query);
            }
            workforce.context = c.clone();
            let journal = WorkforceJournal::open(
                workforce.journal.clone(),
                &c,
                if query_only {
                    WorkforceMode::Recover
                } else {
                    WorkforceMode::Control
                },
                None,
            )?;
            let (native_owner, view) = WorkforceSession::new(journal, &c)?;
            let associations = AssociationStore::open(
                workforce.associations.clone(),
                view.id,
                false,
                query_only,
                &c,
            )?;
            let labor_owner = SemanticWorkforceSession::new(native_owner, associations, &c)?;
            let journal = WorkOrderJournal::open(
                order_journal.clone(),
                &c,
                if query_only {
                    JournalMode::Reconcile
                } else {
                    JournalMode::Control
                },
                false,
            )?;
            let id = journal.summary(&c)?.journal_id;
            let associations = crate::semantic_work_orders::AssociationStore::open(
                order_associations.clone(),
                id,
                false,
                query_only,
                &c,
            )?;
            let source = OrderNative::new(
                Rc::clone(&orders),
                order_journal.clone(),
                order_associations.clone(),
                &c,
            );
            let native_owner = WorkOrderSession::new(journal, Some(source), &c)?;
            let order_owner = SemanticWorkOrderSession::new(native_owner, associations, &c)?;
            let executor = Executor::new(
                plan.clone(),
                labor_owner,
                order_owner,
                if query_only { None } else { evidence },
                Factory(workforce.clone()),
                &c,
            )?;
            Ok(Self {
                executor,
                plan,
                c,
                workforce,
                orders,
                order_journal,
                order_associations,
                hooks,
            })
        }
    }

    #[test]
    fn actual_labor_then_order_retains_whole_parent_and_never_promotes_creation_to_goal()
    -> Result<()> {
        let mut f = ProductionFixture::new(1)?;
        let archive = f.executor.archive_seal();
        f.applied(0)?;
        f.applied(1)?;
        assert_eq!(f.workforce.data.borrow().calls[2], 1);
        assert_eq!(f.orders.borrow().commits, 1);
        let Some(StepEvidence::WorkOrder(result)) = f.executor.latest_step_evidence() else {
            return Err(invalid("expected original selected work-order result"));
        };
        assert_eq!(result.review().original_plan(), &f.plan);
        assert_eq!(result.review().step_id(), StepId::new(1));
        assert_eq!(result.review().selected_step(), &f.plan.steps[1]);
        assert_eq!(
            result.pending_obligation(),
            f.plan.steps[1].obligation.as_ref()
        );
        assert!(!result.original_goal_proven());
        assert!(!f.executor.original_goal_proven());
        assert_eq!(f.executor.archive_seal(), archive);
        let review = result.review().clone();
        assert!(
            f.executor
                .orders
                .begin_goal_monitor(review.seal(), &f.c)
                .is_err()
        );
        let native_journal = WorkOrderJournal::open(
            Storage::from_bytes(f.order_journal.0.borrow().synced.clone()),
            &f.c,
            JournalMode::Offline,
            false,
        )?
        .summary(&f.c)?
        .journal_id;
        assert!(
            crate::semantic_work_orders::GoalHistoryStore::open(
                Storage::default(),
                native_journal,
                &review,
                true,
                false,
                &f.c
            )
            .is_err()
        );
        Ok(())
    }
    #[test]
    fn full_sixteen_step_original_executes_both_families_once_per_original_key() -> Result<()> {
        let mut f = ProductionFixture::new(15)?;
        let (bytes, seal) =
            dfmcp_intent::plan_archive::encode(&f.plan, crate::live_production::archive_limits())?;
        assert_eq!(
            dfmcp_intent::plan_archive::decode(
                &bytes,
                seal,
                crate::live_production::archive_limits()
            )?,
            f.plan
        );
        for index in 0..16 {
            f.applied(index)?;
        }
        assert_eq!(f.orders.borrow().commits, 15);
        assert_eq!(f.orders.borrow().retained.len(), 15);
        assert_eq!(f.workforce.data.borrow().calls[2], 1);
        assert!(!f.executor.original_goal_proven());
        Ok(())
    }
    #[test]
    fn direct_trait_calls_cannot_skip_dependencies_or_substitute_any_typed_route_field()
    -> Result<()> {
        let mut f = ProductionFixture::new(1)?;
        assert!(f.prepare(1).is_err());
        assert_eq!(f.orders.borrow().reads, 0);
        let mut route = f.route(0)?;
        if let LiveRequest::WorkDetail { assigned, .. } = &mut route.request {
            *assigned = !*assigned;
        }
        assert!(
            f.executor
                .prepare(&f.plan, &f.plan.steps[0], &route, &f.c)
                .is_err()
        );
        let mut route = f.route(1)?;
        route.live_preconditions.push("forged native condition");
        assert!(
            f.executor
                .prepare(&f.plan, &f.plan.steps[1], &route, &f.c)
                .is_err()
        );
        let forged =
            FamilyPreparation::new(LiveFamily::WorkOrdersV1_10, Digest32::of_bytes(b"forged"))?;
        assert!(f.commit(1, forged).is_err());
        assert_eq!(f.orders.borrow().commits, 0);
        assert_eq!(f.workforce.data.borrow().calls, [0; 5]);
        Ok(())
    }
    #[test]
    fn unsupported_later_actions_and_unproved_predecessor_predicates_refuse_the_complete_plan()
    -> Result<()> {
        let f = ProductionFixture::new(1)?;
        let invalid_posts = [
            Predicate::Paused(true),
            Predicate::EntityExists(EntityId::new(43)),
            Predicate::FieldCompare {
                entity_id: EntityId::new(44),
                field: "labor.MINE".into(),
                op: CompareOp::Eq,
                value: Value::Bool(true),
            },
            Predicate::FieldCompare {
                entity_id: EntityId::new(43),
                field: "labor.CARPENTRY".into(),
                op: CompareOp::Eq,
                value: Value::Bool(true),
            },
            Predicate::FieldCompare {
                entity_id: EntityId::new(43),
                field: "labor.MINE".into(),
                op: CompareOp::Eq,
                value: Value::Bool(false),
            },
        ];
        for predicate in invalid_posts {
            let mut plan = f.plan.clone();
            plan.steps[0].postconditions = vec![predicate];
            reseal(&mut plan);
            assert!(validate_plan(&plan).is_err());
        }
        let mut plan = f.plan.clone();
        plan.steps[0].obligation = f.plan.steps[1].obligation.clone();
        reseal(&mut plan);
        assert!(validate_plan(&plan).is_err());
        let mut plan = f.plan.clone();
        plan.steps[1].action = Action::Pause { paused: true };
        plan.steps[1].risk = plan.steps[1].action.risk();
        plan.steps[1].required_capability = plan.steps[1].action.capability();
        plan.steps[1].idempotency_key = derive_step_idempotency_key(
            plan.intent_id,
            plan.anchor,
            plan.steps[1].id,
            &plan.steps[1].action,
        );
        plan.required_capabilities = plan.steps.iter().map(|s| s.required_capability).collect();
        plan.max_risk = plan
            .steps
            .iter()
            .map(|s| s.risk)
            .max()
            .ok_or_else(|| invalid("fixture plan is empty"))?;
        reseal(&mut plan);
        assert!(validate_plan(&plan).is_err());
        assert_eq!(f.orders.borrow().reads, 0);
        Ok(())
    }
    #[test]
    fn applied_outside_requested_labor_semantics_never_releases_order() -> Result<()> {
        let mut f = ProductionFixture::new(1)?;
        f.workforce.data.borrow_mut().labors = vec![1, 1];
        let prepared = f.prepare(0)?;
        assert!(f.commit(0, prepared).is_err());
        assert!(f.prepare(1).is_err());
        assert_eq!(f.orders.borrow().reads, 0);
        let route = f.route(0)?;
        assert!(matches!(
            f.executor
                .recover_step(&f.plan, &f.plan.steps[0], &route, &f.c)?,
            RecoveredFamily::Failed(Some(_))
        ));
        Ok(())
    }
    #[test]
    fn predecessor_custody_is_rechecked_after_order_dispatch_marker_before_native_insertion()
    -> Result<()> {
        let mut f = ProductionFixture::new(1)?;
        f.applied(0)?;
        let prepared = f.prepare(1)?;
        let next = f.hooks.borrow().orders + 1;
        f.hooks.borrow_mut().corrupt_at = Some((next, f.workforce.journal.clone()));
        assert!(f.commit(1, prepared).is_err());
        assert_eq!(f.orders.borrow().commits, 0);
        assert!(f.executor.latest_step_evidence().is_none());
        let journal = WorkOrderJournal::open(
            Storage::from_bytes(f.order_journal.0.borrow().synced.clone()),
            &f.c,
            JournalMode::Offline,
            false,
        )?;
        let summary = journal.summary(&f.c)?;
        assert_eq!(
            journal.records_page(summary.head, None, 64, &f.c)?.records[0].state(),
            CreationState::DispatchStarted
        );
        Ok(())
    }
    #[test]
    fn lost_labor_reply_queries_original_key_then_releases_order_without_reassignment() -> Result<()>
    {
        let mut f = ProductionFixture::new(1)?;
        f.workforce.data.borrow_mut().lose_commit = true;
        let prepared = f.prepare(0)?;
        assert!(f.commit(0, prepared).is_err());
        assert!(f.prepare(1).is_err());
        let route = f.route(0)?;
        assert!(matches!(
            f.executor
                .recover_step(&f.plan, &f.plan.steps[0], &route, &f.c)?,
            RecoveredFamily::Applied(_)
        ));
        assert_eq!(f.workforce.data.borrow().calls[2], 1);
        assert_eq!(f.workforce.data.borrow().calls[3], 1);
        f.applied(1)?;
        assert_eq!(f.orders.borrow().commits, 1);
        Ok(())
    }
    #[test]
    fn lost_order_reply_reopens_query_only_and_resolves_without_another_insertion() -> Result<()> {
        let mut f = ProductionFixture::new(1)?;
        f.applied(0)?;
        let prepared = f.prepare(1)?;
        f.orders.borrow_mut().lose_reply = true;
        assert!(f.commit(1, prepared).is_err());
        let mut f = f.reopen_query()?;
        f.c.anchor.tick = GameTick(1000);
        let route = f.route(1)?;
        assert!(matches!(
            f.executor
                .recover_step(&f.plan, &f.plan.steps[1], &route, &f.c)?,
            RecoveredFamily::Applied(_)
        ));
        assert_eq!(f.orders.borrow().commits, 1);
        assert_eq!(f.orders.borrow().queries, 1);
        assert!(f.prepare(1).is_err());
        assert!(f.commit(1, prepared).is_err());
        Ok(())
    }
    #[test]
    fn recovered_prepared_identity_never_restores_a_local_commit_permission() -> Result<()> {
        let mut f = ProductionFixture::new(1)?;
        f.applied(0)?;
        let prepared = f.prepare(1)?;
        let mut f = f.reopen_query()?;
        let route = f.route(1)?;
        assert_eq!(
            f.executor
                .recover_step(&f.plan, &f.plan.steps[1], &route, &f.c)?,
            RecoveredFamily::Prepared(prepared)
        );
        assert!(f.commit(1, prepared).is_err());
        assert_eq!(f.orders.borrow().commits, 0);
        Ok(())
    }
    #[test]
    fn precontact_budget_and_current_or_retained_authority_fail_without_native_mutation()
    -> Result<()> {
        let mut f = ProductionFixture::new(1)?;
        f.c.budget.max_bytes = 10 * 1024 * 1024;
        assert!(f.prepare(0).is_err());
        assert_eq!(f.workforce.data.borrow().calls, [0; 5]);
        f.c.budget.max_bytes = 2 * 1024 * 1024 * 1024;
        f.c.grants
            .retain(|g| g.capability != Capability::ConfigureLabor);
        assert!(f.prepare(0).is_err());
        assert_eq!(f.workforce.data.borrow().calls, [0; 5]);
        Ok(())
    }
    #[test]
    fn durable_frontier_recovers_real_lost_insertion_with_query_only_after_original_expiry()
    -> Result<()> {
        let mut f = ProductionFixture::new(1)?;
        let storage = Storage::default();
        let pin = crate::live_dispatch::durable::initial_recovery(&f.plan, &f.c)?;
        let mut coordinator = Coordinator::open(
            storage.clone(),
            &f.c,
            BuildMode::Control,
            Some(f.plan.clone()),
            pin,
            true,
        )?;
        let labor = coordinator.prepare_next(&mut f.executor, &f.c)?;
        assert_eq!(labor.step, StepId::ZERO);
        let view = coordinator.commit(labor.seal, true, &mut f.executor, &f.c)?;
        assert_eq!(view.next_ready, Some(StepId::new(1)));
        let order = coordinator.prepare_next(&mut f.executor, &f.c)?;
        f.orders.borrow_mut().lose_reply = true;
        assert!(
            coordinator
                .commit(order.seal, true, &mut f.executor, &f.c)
                .is_err()
        );
        let recovery = coordinator.recovery();
        drop(coordinator);
        let mut f = f.reopen_query()?;
        f.c.anchor.tick = GameTick(1000);
        let mut coordinator = Coordinator::open(
            storage,
            &f.c,
            BuildMode::Recover,
            Some(f.plan.clone()),
            recovery,
            false,
        )?;
        let view = coordinator.reconcile(StepId::new(1), &mut f.executor, &f.c)?;
        assert!(view.all_applied());
        assert!(!view.original_goal_proven());
        assert!(view.review.is_none());
        assert_eq!(f.orders.borrow().commits, 1);
        assert_eq!(f.orders.borrow().queries, 1);
        assert_eq!(f.workforce.data.borrow().calls[2], 1);
        assert!(
            coordinator
                .commit(order.seal, true, &mut f.executor, &f.c)
                .is_err()
        );
        Ok(())
    }
    #[test]
    fn native_prepare_reply_loss_is_not_misreported_as_no_contact_or_permission_to_change_source()
    -> Result<()> {
        let mut f = ProductionFixture::new(1)?;
        f.applied(0)?;
        f.orders.borrow_mut().lose_prepare_reply = true;
        assert!(f.prepare(1).is_err());
        assert_eq!(f.orders.borrow().prepares, 1);
        assert_eq!(f.orders.borrow().commits, 0);
        let route = f.route(1)?;
        assert_eq!(
            f.executor
                .recover_step(&f.plan, &f.plan.steps[1], &route, &f.c)?,
            RecoveredFamily::AbsentBeforeDispatch
        );
        let original_effect = f
            .orders
            .borrow()
            .retained
            .values()
            .next()
            .ok_or_else(|| invalid("native reservation is absent"))?
            .canonical_bytes()
            .to_vec();
        f.orders.borrow_mut().lose_prepare_reply = false;
        // A lost native reply fences that connection. Reopen the same private
        // journals with a fresh source before an explicit same-key preparation.
        let mut f = f.reopen(false)?;
        let prepared = f.prepare(1)?;
        assert_eq!(
            f.orders
                .borrow()
                .retained
                .values()
                .next()
                .ok_or_else(|| invalid("original native reservation is absent"))?
                .canonical_bytes(),
            original_effect
        );
        assert_eq!(f.orders.borrow().prepares, 2);
        assert_eq!(f.orders.borrow().commits, 0);
        f.commit(1, prepared)?;
        assert_eq!(f.orders.borrow().commits, 1);
        Ok(())
    }
    #[test]
    fn executor_opening_risk_and_expiry_remain_effective_when_current_grants_later_broaden()
    -> Result<()> {
        let mut f = ProductionFixture::new(1)?;
        let Executor {
            original,
            workforce,
            orders,
            evidence,
            factory,
            ..
        } = f.executor;
        let mut opening = f.c.clone();
        for grant in &mut opening.grants {
            grant.max_risk = RiskTier::Reversible;
        }
        f.executor = Executor::new(original, workforce, orders, evidence, factory, &opening)?;
        assert!(f.prepare(0).is_err());
        assert_eq!(f.workforce.data.borrow().calls, [0; 5]);

        let mut f = ProductionFixture::new(1)?;
        let Executor {
            original,
            workforce,
            orders,
            evidence,
            factory,
            ..
        } = f.executor;
        let mut opening = f.c.clone();
        for grant in &mut opening.grants {
            if matches!(
                grant.capability,
                Capability::Plan | Capability::ConfigureLabor
            ) {
                grant.expires_at_tick = Some(GameTick(125));
            }
        }
        f.executor = Executor::new(original, workforce, orders, evidence, factory, &opening)?;
        let prepared = f.prepare(0)?;
        f.c.anchor.tick = GameTick(130);
        let route = f.route(0)?;
        assert!(
            f.executor
                .cancel_prepared(&f.plan, &f.plan.steps[0], &route, prepared, &f.c)
                .is_err()
        );
        assert_eq!(f.workforce.data.borrow().calls[4], 0);
        assert_eq!(f.workforce.data.borrow().calls[2], 0);
        Ok(())
    }
    #[test]
    fn decoded_future_queue_expiring_query_is_retained_by_executor_and_durable_handoff()
    -> Result<()> {
        let mut f = ProductionFixture::new(1)?;
        let Executor {
            original,
            workforce,
            orders,
            evidence,
            factory,
            ..
        } = f.executor;
        let mut opening = f.c.clone();
        for grant in &mut opening.grants {
            if grant.capability == Capability::Query {
                grant.expires_at_tick = Some(GameTick(125));
            }
        }
        f.executor = Executor::new(original, workforce, orders, evidence, factory, &opening)?;
        let pin = crate::live_dispatch::durable::initial_recovery(&f.plan, &f.c)?;
        let mut coordinator = Coordinator::open(
            Storage::default(),
            &f.c,
            BuildMode::Control,
            Some(f.plan.clone()),
            pin,
            true,
        )?;
        let labor = coordinator.prepare_next(&mut f.executor, &f.c)?;
        coordinator.commit(labor.seal, true, &mut f.executor, &f.c)?;
        let mut bytes = f.orders.borrow().queue.canonical_bytes().to_vec();
        bytes[24..32].copy_from_slice(&140u64.to_be_bytes());
        f.orders.borrow_mut().queue = WorkOrderObservation::decode(&bytes)?;
        assert!(coordinator.prepare_next(&mut f.executor, &f.c).is_err());
        assert_eq!(f.orders.borrow().reads, 1);
        assert_eq!(f.orders.borrow().prepares, 0);
        assert!(f.executor.high_tick() >= 140);
        assert!(coordinator.recovery().authority_tick_floor >= 140);
        assert!(f.executor.latest_step_evidence().is_none());
        let before = f.orders.borrow().reads;
        assert!(
            f.prepare(1).is_err(),
            "an older context cannot revive the shorter opening Query grant"
        );
        assert_eq!(f.orders.borrow().reads, before);
        Ok(())
    }
}
