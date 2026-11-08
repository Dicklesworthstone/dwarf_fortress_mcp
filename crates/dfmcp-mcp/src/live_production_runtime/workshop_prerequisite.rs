//! A separately pinned native workshop is a host dispatch prerequisite.
//! Its facts never become canonical citizen facts or a production-goal proof.
use super::workshop_config::WorkshopConfig;
use super::{File, MIB, Work, denied, exhausted};
use dfmcp_adapter::build_placement::rpc::BuildCancellation;
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::workshop_construction::{
    acknowledgement::{WorkshopAcknowledgements, open_private_acknowledgements},
    journal::{WorkshopJournal, WorkshopMode, open_private_workshop},
    production_origin::{
        ASSOCIATION_MAX_BYTES, CompletedWorkshopProduction, IMPORT_WORK_BYTES, REFRESH_WORK_BYTES,
        VERIFY_WORK_BYTES, WorkshopProductionOrigin, open_private_association_storage,
    },
    rpc::{WorkshopRpc, WorkshopSource},
};
use dfmcp_core::{
    Capability, Digest32, ErrorCode, GameTick, OperationContext, Result, RiskTier, SessionId,
};
use dfmcp_intent::PreparedPlan;
use serde_json::{Value, json};
use std::io::SeekFrom;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub(super) const OPEN_BYTES: u64 = 2 * IMPORT_WORK_BYTES + 8 * MIB;
pub(super) const CHECK_BYTES: u64 = REFRESH_WORK_BYTES + 16 * MIB;
pub(super) const LOCAL_BYTES: u64 = VERIFY_WORK_BYTES + 8 * MIB;
const STORE_BYTES: u64 = 4 * MIB;
const DOMAIN: &[u8] = b"dfmcp-combined-workshop-prerequisite/1\0";
pub(super) const FILE_NAME: &str = "original.workshop";

type ReadGuard = Arc<dyn Fn() -> Result<()> + Send + Sync>;

fn corrupt() -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(
        ErrorCode::CorruptLedger,
        "original workshop prerequisite custody changed; preserve the complete original recovery",
    )
}
fn put(bytes: &mut Vec<u8>, value: &[u8]) -> Result<()> {
    let length = u32::try_from(value.len()).map_err(|_| corrupt())?;
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(value);
    Ok(())
}
fn association_bytes(
    plan: &PreparedPlan,
    config: &WorkshopConfig,
    origin: &WorkshopProductionOrigin,
) -> Result<Vec<u8>> {
    plan.validate_structure()?;
    if super::workshop_binding::requirement(plan)? != Some(origin.witness())
        || plan.anchor.fortress_id != origin.binding().fortress().fortress_id()
    {
        return Err(denied(
            "original plan does not retain this exact workshop prerequisite",
        ));
    }
    let mut bytes = DOMAIN.to_vec();
    bytes.extend_from_slice(plan.digest.as_bytes());
    let directory = config.directory.to_str().ok_or_else(corrupt)?;
    put(&mut bytes, directory.as_bytes())?;
    put(&mut bytes, origin.canonical_bytes())?;
    if bytes.len() > ASSOCIATION_MAX_BYTES {
        return Err(exhausted(
            "original workshop association exceeds its fixed bound",
        ));
    }
    Ok(bytes)
}

/// The bytes are generated from independently verified original custody. A
/// reopened file must equal the complete expected value and external digest;
/// no untrusted field decoder or alternate path is involved.
struct AssociationStore<S> {
    storage: S,
    bytes: Vec<u8>,
    seal: Digest32,
    owner: SessionId,
    fenced: bool,
}
impl<S: EffectJournalStorage> AssociationStore<S> {
    fn access(&self, c: &OperationContext) -> Result<Work> {
        if self.fenced || self.owner != c.session_id || c.session_id == SessionId::NIL {
            return Err(corrupt());
        }
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        let mut work = Work::new(c)?;
        if work.bytes < STORE_BYTES {
            return Err(exhausted("workshop association byte allowance exhausted"));
        }
        work.bytes = STORE_BYTES;
        Ok(work)
    }
    fn verify_inner(&mut self, work: &Work) -> Result<()> {
        self.storage.validate_identity().map_err(|_| corrupt())?;
        if self.storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? != self.bytes.len() as u64 {
            return Err(corrupt());
        }
        self.storage
            .seek(SeekFrom::Start(0))
            .map_err(|_| corrupt())?;
        let mut buffer = [0; 4096];
        for expected in self.bytes.chunks(buffer.len()) {
            work.current()?;
            self.storage
                .read_exact(&mut buffer[..expected.len()])
                .map_err(|_| corrupt())?;
            if &buffer[..expected.len()] != expected {
                return Err(corrupt());
            }
        }
        self.storage.validate_identity().map_err(|_| corrupt())?;
        if self.storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? != self.bytes.len() as u64 {
            return Err(corrupt());
        }
        work.current()?;
        Ok(())
    }
    fn create(storage: S, bytes: Vec<u8>, c: &OperationContext) -> Result<Self> {
        c.authorize(Capability::Plan, RiskTier::ReadOnly, &[], None)?;
        if bytes.is_empty() || bytes.len() > ASSOCIATION_MAX_BYTES {
            return Err(corrupt());
        }
        let mut out = Self {
            storage,
            seal: Digest32::of_bytes(&bytes),
            bytes,
            owner: c.session_id,
            fenced: false,
        };
        let work = out.access(c)?;
        out.storage.validate_identity().map_err(|_| corrupt())?;
        if out.storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? != 0 {
            return Err(corrupt());
        }
        for chunk in out.bytes.chunks(4096) {
            work.current()?
                .authorize(Capability::Plan, RiskTier::ReadOnly, &[], None)?;
            out.storage.write_all(chunk).map_err(|_| corrupt())?;
        }
        out.storage
            .flush()
            .and_then(|_| out.storage.sync())
            .map_err(|_| corrupt())?;
        out.verify_inner(&work)?;
        Ok(out)
    }
    fn open(storage: S, bytes: Vec<u8>, external: Digest32, c: &OperationContext) -> Result<Self> {
        if bytes.is_empty()
            || bytes.len() > ASSOCIATION_MAX_BYTES
            || external == Digest32::ZERO
            || Digest32::of_bytes(&bytes) != external
        {
            return Err(corrupt());
        }
        let mut out = Self {
            storage,
            bytes,
            seal: external,
            owner: c.session_id,
            fenced: false,
        };
        let work = out.access(c)?;
        out.verify_inner(&work)?;
        Ok(out)
    }
    fn verify(&mut self, c: &OperationContext) -> Result<()> {
        let work = self.access(c)?;
        let result = self.verify_inner(&work);
        if result
            .as_ref()
            .is_err_and(|cause| cause.code == ErrorCode::CorruptLedger)
        {
            self.fenced = true;
        }
        result
    }
}

/// Absence is checked even for legacy openings. A configured prerequisite
/// cannot be silently dropped while its original association still exists.
pub(super) fn require_absent(path: &Path, c: &OperationContext) -> Result<()> {
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    c.budget.validate()?;
    let work = Work::new(c)?;
    match std::fs::symlink_metadata(path) {
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            work.current()?;
            Ok(())
        }
        _ => Err(denied(
            "existing original workshop association requires its complete recovery configuration",
        )),
    }
}

struct Owner {
    config: WorkshopConfig,
    guard: ReadGuard,
    journal: WorkshopJournal<File>,
    acknowledgement: WorkshopAcknowledgements<File>,
    origin: WorkshopProductionOrigin,
    association: Option<AssociationStore<File>>,
    last: Option<CompletedWorkshopProduction>,
    nonce_sequence: u64,
}
impl Owner {
    fn open(config: WorkshopConfig, guard: ReadGuard, c: &OperationContext) -> Result<Self> {
        guard()?;
        let mut work = Work::new(c)?;
        let journal_budget = work.take(IMPORT_WORK_BYTES)?;
        let ack_budget = work.take(2 * MIB)?;
        let origin_budget = work.take(IMPORT_WORK_BYTES)?;
        let mut journal = open_private_workshop(
            &config.directory.join("workshop.journal"),
            &work.narrow(&journal_budget)?,
            WorkshopMode::Offline,
            Some(config.binding.clone()),
            [0; 32],
            Some(config.acknowledged),
        )?;
        let mut acknowledgement = open_private_acknowledgements(
            &config.directory.join("workshop.ack"),
            &work.narrow(&ack_budget)?,
            WorkshopMode::Offline,
            false,
            Some(config.acknowledged),
        )?;
        let origin = journal.production_origin(
            &mut acknowledgement,
            &config.original_key,
            config.original_plan_digest,
            &work.narrow(&origin_budget)?,
        )?;
        guard()?;
        work.current()?;
        Ok(Self {
            config,
            guard,
            journal,
            acknowledgement,
            origin,
            association: None,
            last: None,
            nonce_sequence: 0,
        })
    }
    fn context(&self, c: &OperationContext) -> OperationContext {
        let mut current = c.clone();
        current.anchor.tick = current.anchor.tick.max(GameTick(self.origin.high_tick()));
        current
    }
    fn verify(&mut self, c: &OperationContext) -> Result<()> {
        (self.guard)()?;
        let mut work = Work::new(&self.context(c))?;
        let origin = work.take(VERIFY_WORK_BYTES)?;
        let association = work.take(STORE_BYTES)?;
        self.origin.verify(
            &mut self.journal,
            &mut self.acknowledgement,
            &work.narrow(&origin)?,
        )?;
        if let Some(store) = &mut self.association {
            store.verify(&work.narrow(&association)?)?;
        }
        (self.guard)()?;
        work.current()?;
        Ok(())
    }
    fn qualify(&mut self, c: &OperationContext) -> Result<()> {
        self.last = None;
        (self.guard)()?;
        if self.origin.is_fenced() {
            return Err(denied(
                "workshop source continuity is fenced; preserve the original recovery",
            ));
        }
        // Reserve every child before opening a connection or reading a credential.
        let mut work = Work::new(&self.context(c))?;
        let before = work.take(STORE_BYTES)?;
        let connect = work.take(dfmcp_adapter::workshop_construction::rpc::CONNECT_BYTES)?;
        let read = work.take(REFRESH_WORK_BYTES)?;
        let after = work.take(STORE_BYTES)?;
        if let Some(store) = &mut self.association {
            store.verify(&work.narrow(&before)?)?;
        }
        let token = self.config.token()?;
        let pinned_token = token.clone();
        let guard = Arc::clone(&self.guard);
        let config = self.config.clone();
        self.nonce_sequence = self
            .nonce_sequence
            .checked_add(1)
            .ok_or_else(|| exhausted("workshop prerequisite connection identity exhausted"))?;
        let mut identity = b"dfmcp-combined-workshop-read/1\0".to_vec();
        identity.extend_from_slice(&c.session_id.get().to_be_bytes());
        identity.extend_from_slice(&c.request_id.get().to_be_bytes());
        identity.extend_from_slice(&self.nonce_sequence.to_be_bytes());
        let nonce = *Digest32::of_bytes(&identity).as_bytes();
        let source = WorkshopRpc::connect(
            self.config.binding.endpoint(),
            token,
            nonce,
            self.config.binding.fortress().clone(),
            &work.narrow(&connect)?,
            BuildCancellation::with_check(Arc::clone(&self.guard)),
            Box::new(move |write| {
                guard()?;
                if write || config.token()? != pinned_token {
                    return Err(denied(
                        "workshop prerequisite grants only this exact read credential",
                    ));
                }
                Ok(())
            }),
        );
        let mut source = match source {
            Ok(source) => source,
            Err(cause) => {
                self.origin.fence();
                return Err(cause);
            }
        };
        // The handshake has already decoded a source incarnation. Preserve
        // negative continuity before a later deadline or authority check can
        // prevent refresh from consuming that manifest.
        self.origin.retain_source_manifest(source.manifest())?;
        let current = match work.narrow(&read) {
            Ok(current) => current,
            Err(cause) => {
                self.origin.fence();
                return Err(cause);
            }
        };
        let qualified = self.origin.refresh(
            &mut self.journal,
            &mut self.acknowledgement,
            &mut source,
            c.anchor.tick,
            &current,
        )?;
        drop(source);
        if let Some(store) = &mut self.association {
            store.verify(&work.narrow(&after)?)?;
        }
        (self.guard)()?;
        work.current()?;
        self.last = Some(qualified);
        Ok(())
    }
    fn bind(&mut self, path: &Path, plan: &PreparedPlan, c: &OperationContext) -> Result<Digest32> {
        if self.association.is_some()
            || self.last.as_ref().is_none_or(|current| {
                current.anchor() != plan.anchor || current.origin_witness() != self.origin.witness()
            })
        {
            return Err(denied(
                "original production requires a fresh same-anchor completed workshop",
            ));
        }
        let mut work = Work::new(&self.context(c))?;
        self.verify(&work.take(LOCAL_BYTES)?)?;
        let bytes = association_bytes(plan, &self.config, &self.origin)?;
        let (storage, remaining) =
            open_private_association_storage(path, &work.take(2 * STORE_BYTES)?, true)?;
        let store = AssociationStore::create(storage, bytes, &remaining)?;
        let seal = store.seal;
        self.association = Some(store);
        (self.guard)()?;
        work.current()?;
        Ok(seal)
    }
    fn reopen(
        &mut self,
        path: &Path,
        plan: &PreparedPlan,
        seal: Digest32,
        c: &OperationContext,
    ) -> Result<()> {
        if self.association.is_some() {
            return Err(corrupt());
        }
        let mut work = Work::new(&self.context(c))?;
        self.verify(&work.take(LOCAL_BYTES)?)?;
        let bytes = association_bytes(plan, &self.config, &self.origin)?;
        let (storage, remaining) =
            open_private_association_storage(path, &work.take(2 * STORE_BYTES)?, false)?;
        self.association = Some(AssociationStore::open(storage, bytes, seal, &remaining)?);
        self.last = None;
        (self.guard)()?;
        work.current()?;
        Ok(())
    }
    fn status(&self) -> Value {
        let record = self.origin.historical_record();
        json!({"schema":"dfmcp.completed-workshop-prerequisite/1", "source_protocol":"workshop/1.20",
            "scope":"original_workshop_dispatch_prerequisite", "origin_witness":self.origin.witness().to_hex(),
            "workshop_key":self.origin.plan().key(), "workshop_plan_digest":self.origin.plan().digest().to_hex(),
            "workshop_receipt":record.receipt().to_hex(),
            "building_id":record.insertion().map(|insert|insert.building_id()),
            "source_generation":self.origin.binding().generation(),
            "association_digest":self.association.as_ref().map(|s|s.seal.to_hex()),
            "consumed_tick_floor":self.origin.high_tick(), "source_fenced":self.origin.is_fenced(),
            "checkpoint":{"origin_witness":self.origin.witness().to_hex(),
                "tick":self.origin.high_tick(),"generation":self.origin.high_generation(),
                "next_building_id":self.origin.high_next_building_id(),"fenced":self.origin.is_fenced()},
            "last_qualified":self.last.as_ref().map(|p|json!({"tick":p.progress().tick(),
                "witness":p.witness().to_hex(),"canonical_parent":super::anchor_json(p.anchor()),
                "status":"complete","fresh_for_current_call":false})),
            "current_state_proven":false,"canonical_workshop_evidence":false,
            "production_feasibility_proven":false,"mutation_authority":false})
    }
}

#[derive(Clone)]
pub(super) struct SharedWorkshop(Arc<Mutex<Owner>>);
impl SharedWorkshop {
    pub(super) fn open(
        config: WorkshopConfig,
        guard: ReadGuard,
        c: &OperationContext,
    ) -> Result<Self> {
        Ok(Self(Arc::new(Mutex::new(Owner::open(config, guard, c)?))))
    }
    pub(super) fn verify(&self, c: &OperationContext) -> Result<()> {
        self.0
            .try_lock()
            .map_err(|_| denied("workshop prerequisite owner is unavailable"))?
            .verify(c)
    }
    pub(super) fn qualify(&self, c: &OperationContext) -> Result<()> {
        self.0
            .try_lock()
            .map_err(|_| denied("workshop prerequisite owner is unavailable"))?
            .qualify(c)
    }
    pub(super) fn bind(
        &self,
        path: &Path,
        plan: &PreparedPlan,
        c: &OperationContext,
    ) -> Result<Digest32> {
        self.0
            .try_lock()
            .map_err(|_| denied("workshop prerequisite owner is unavailable"))?
            .bind(path, plan, c)
    }
    pub(super) fn reopen(
        &self,
        path: &Path,
        plan: &PreparedPlan,
        seal: Digest32,
        c: &OperationContext,
    ) -> Result<()> {
        self.0
            .try_lock()
            .map_err(|_| denied("workshop prerequisite owner is unavailable"))?
            .reopen(path, plan, seal, c)
    }
    pub(super) fn witness(&self) -> Result<Digest32> {
        Ok(self
            .0
            .try_lock()
            .map_err(|_| denied("workshop prerequisite owner is unavailable"))?
            .origin
            .witness())
    }
    pub(super) fn high_tick(&self) -> u64 {
        self.0
            .try_lock()
            .map_or(u64::MAX, |owner| owner.origin.high_tick())
    }
    pub(super) fn withdraw(&self) -> Result<()> {
        self.0
            .try_lock()
            .map_err(|_| denied("workshop prerequisite owner is unavailable"))?
            .last = None;
        Ok(())
    }
    pub(super) fn status(&self) -> Value {
        self.0.try_lock().map_or_else(
            |_| json!({"status":"unavailable","current_state_proven":false}),
            |owner| owner.status(),
        )
    }
    pub(super) fn seal(&self) -> Option<Digest32> {
        self.0
            .try_lock()
            .ok()
            .and_then(|owner| owner.association.as_ref().map(|store| store.seal))
    }
    pub(super) fn recovery_floor(&self) -> Value {
        self.0.try_lock().map_or(Value::Null, |owner| json!({"tick":owner.origin.high_tick(),
            "generation":owner.origin.high_generation(), "next_building_id":owner.origin.high_next_building_id(),
            "fenced":owner.origin.is_fenced()}))
    }
    pub(super) fn restore_floor(
        &self,
        tick: u64,
        generation: u64,
        next_building_id: u32,
        fenced: bool,
        c: &OperationContext,
    ) -> Result<()> {
        self.0
            .try_lock()
            .map_err(|_| denied("workshop prerequisite owner is unavailable"))?
            .origin
            .retain_recovery_floor(tick, generation, next_building_id, fenced, c)
    }
}

#[cfg(test)]
#[path = "workshop_association_tests.rs"]
mod association_tests;
