//! Independent, descriptor-pinned acknowledgements for the workshop consumer.
//! This file is separate from workshop.journal. It must be synchronized BEFORE
//! a native writer, not exported after the MCP call. It is local crash/rollback
//! custody, not distributed consensus or protection against rolling back both
//! stores and every externally held acknowledgement together.
use super::journal::{MAX_FRAMES, WorkshopMode, WorkshopPin};
use crate::build_placement::journal::{
    BuildMode,
    private_file::{PrivateBuildFile, open_private_storage_scoped},
};
use crate::control_effect_journal::EffectJournalStorage;
use dfmcp_core::{Capability, Digest32, ErrorCode, MapCuboid, OperationContext, Result, RiskTier};
use std::io::SeekFrom;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
enum Protocol {
    Workshop,
    Tree,
    Manager,
    ManagerV1_23,
}
impl Protocol {
    fn magic(self) -> &'static [u8; 8] {
        match self {
            Self::Workshop => b"DFMWA020",
            Self::Tree => b"DFMTA021",
            Self::Manager => b"DFMMA022",
            Self::ManagerV1_23 => b"DFMMA023",
        }
    }
    fn domain(self) -> &'static [u8] {
        match self {
            Self::Workshop => b"dfmcp-workshop-acknowledgement/1\0",
            Self::Tree => b"dfmcp-tree-harvest-acknowledgement/1\0",
            Self::Manager | Self::ManagerV1_23 => b"dfmcp-manager-acknowledgement/1\0",
        }
    }
}
const RECORD: usize = 140;
const MAX_BYTES: usize = (MAX_FRAMES as usize + 1) * RECORD;
pub const WORK_BYTES: u64 = 1024 * 1024;

fn corrupt() -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(
        ErrorCode::CorruptLedger,
        "independent workshop acknowledgement missing, regressed or corrupt; preserve both stores",
    )
}
fn check(value: bool) -> Result<()> {
    if value { Ok(()) } else { Err(corrupt()) }
}
fn digest(protocol: Protocol, bytes: &[u8]) -> Digest32 {
    let mut data = protocol.domain().to_vec();
    data.extend_from_slice(bytes);
    Digest32::of_bytes(&data)
}
fn read<S: EffectJournalStorage>(
    storage: &mut S,
    context: &OperationContext,
    scope: Option<MapCuboid>,
) -> Result<Vec<u8>> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], scope)?;
    context.budget.validate()?;
    let start = Instant::now();
    storage.validate_identity().map_err(|_| corrupt())?;
    let size = storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())?;
    check(size <= MAX_BYTES as u64 && size <= context.budget.max_bytes)?;
    storage.seek(SeekFrom::Start(0)).map_err(|_| corrupt())?;
    let mut raw = vec![0; size as usize];
    storage.read_exact(&mut raw).map_err(|_| corrupt())?;
    check(storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? == size)?;
    storage.validate_identity().map_err(|_| corrupt())?;
    check(start.elapsed() < Duration::from_millis(context.budget.max_wall_millis))?;
    Ok(raw)
}

pub struct WorkshopAcknowledgements<S> {
    protocol: Protocol,
    scope: Option<MapCuboid>,
    storage: S,
    raw: Vec<u8>,
    latest: Option<WorkshopPin>,
    chain: Digest32,
    offline: bool,
    fenced: bool,
}
pub fn open_private_acknowledgements(
    path: &Path,
    context: &OperationContext,
    mode: WorkshopMode,
    initialize: bool,
    external: Option<WorkshopPin>,
) -> Result<WorkshopAcknowledgements<PrivateBuildFile>> {
    check(!initialize || (mode == WorkshopMode::Control && external.is_none()))?;
    check(initialize || external.is_some())?;
    open_private_for(
        path,
        context,
        mode,
        initialize,
        external,
        Protocol::Workshop,
        None,
    )
}
pub fn open_private_tree_acknowledgements(
    path: &Path,
    context: &OperationContext,
    mode: WorkshopMode,
    initialize: bool,
    external: Option<WorkshopPin>,
    scope: MapCuboid,
) -> Result<WorkshopAcknowledgements<PrivateBuildFile>> {
    check(!initialize || (mode == WorkshopMode::Control && external.is_none()))?;
    check(initialize || external.is_some())?;
    open_private_for(
        path,
        context,
        mode,
        initialize,
        external,
        Protocol::Tree,
        Some(scope),
    )
}
pub fn open_private_manager_acknowledgements(
    path: &Path,
    context: &OperationContext,
    mode: WorkshopMode,
    initialize: bool,
    external: Option<WorkshopPin>,
    scope: MapCuboid,
) -> Result<WorkshopAcknowledgements<PrivateBuildFile>> {
    check(!initialize || (mode == WorkshopMode::Control && external.is_none()))?;
    check(initialize || external.is_some())?;
    open_private_for(
        path,
        context,
        mode,
        initialize,
        external,
        Protocol::Manager,
        Some(scope),
    )
}
/// A host-pinned Manager generation selects distinct custody bytes. No tool
/// argument or journal contents may select a generation on the caller's behalf.
pub fn open_private_manager_acknowledgements_for_protocol(
    path: &Path,
    context: &OperationContext,
    mode: WorkshopMode,
    initialize: bool,
    external: Option<WorkshopPin>,
    scope: MapCuboid,
    protocol: crate::manager_appointment::ManagerProtocol,
) -> Result<WorkshopAcknowledgements<PrivateBuildFile>> {
    check(!initialize || (mode == WorkshopMode::Control && external.is_none()))?;
    check(initialize || external.is_some())?;
    let protocol = match protocol {
        crate::manager_appointment::ManagerProtocol::V1_22 => Protocol::Manager,
        crate::manager_appointment::ManagerProtocol::V1_23 => Protocol::ManagerV1_23,
    };
    open_private_for(
        path,
        context,
        mode,
        initialize,
        external,
        protocol,
        Some(scope),
    )
}
fn open_private_for(
    path: &Path,
    context: &OperationContext,
    mode: WorkshopMode,
    initialize: bool,
    external: Option<WorkshopPin>,
    protocol: Protocol,
    scope: Option<MapCuboid>,
) -> Result<WorkshopAcknowledgements<PrivateBuildFile>> {
    let storage_mode = match mode {
        WorkshopMode::Control => BuildMode::Control,
        WorkshopMode::Recover => BuildMode::Recover,
        WorkshopMode::Offline => BuildMode::Offline,
    };
    let (storage, created, current) =
        open_private_storage_scoped(path, context, storage_mode, initialize, MAX_BYTES, scope)?;
    check(created == initialize)?;
    WorkshopAcknowledgements::open_for(
        storage, &current, mode, initialize, external, protocol, scope,
    )
}
impl<S: EffectJournalStorage> WorkshopAcknowledgements<S> {
    pub fn open(
        storage: S,
        context: &OperationContext,
        mode: WorkshopMode,
        initialize: bool,
        external: Option<WorkshopPin>,
    ) -> Result<Self> {
        Self::open_for(
            storage,
            context,
            mode,
            initialize,
            external,
            Protocol::Workshop,
            None,
        )
    }
    fn open_for(
        mut storage: S,
        context: &OperationContext,
        mode: WorkshopMode,
        initialize: bool,
        external: Option<WorkshopPin>,
        protocol: Protocol,
        scope: Option<MapCuboid>,
    ) -> Result<Self> {
        let raw = read(&mut storage, context, scope)?;
        check(
            (initialize && raw.is_empty() && external.is_none() && mode == WorkshopMode::Control)
                || (!initialize && !raw.is_empty() && external.is_some()),
        )?;
        check(raw.len().is_multiple_of(RECORD))?;
        let mut latest = None;
        let mut chain = Digest32::ZERO;
        let mut acknowledged = false;
        for record in raw.as_chunks::<RECORD>().0 {
            check(&record[..8] == protocol.magic() && &record[76..108] == chain.as_bytes())?;
            let pin = WorkshopPin {
                journal_id: Digest32::from_bytes(record[8..40].try_into().map_err(|_| corrupt())?),
                head: Digest32::from_bytes(record[40..72].try_into().map_err(|_| corrupt())?),
                frames: u32::from_be_bytes(record[72..76].try_into().map_err(|_| corrupt())?),
            };
            check(
                pin.journal_id != Digest32::ZERO
                    && pin.head != Digest32::ZERO
                    && pin.frames <= MAX_FRAMES,
            )?;
            if let Some(old) = latest {
                let old: WorkshopPin = old;
                check(
                    pin.journal_id == old.journal_id
                        && pin.frames > old.frames
                        && pin.head != old.head,
                )?;
            } else {
                check(pin.frames == 0 && pin.head == pin.journal_id)?;
            }
            chain = digest(protocol, &record[..108]);
            check(&record[108..] == chain.as_bytes())?;
            acknowledged |= external == Some(pin);
            latest = Some(pin);
        }
        check(initialize || acknowledged)?;
        Ok(Self {
            protocol,
            scope,
            storage,
            raw,
            latest,
            chain,
            offline: mode == WorkshopMode::Offline,
            fenced: false,
        })
    }
    pub fn latest(&self) -> Option<WorkshopPin> {
        self.latest
    }
    pub fn verify(&mut self, context: &OperationContext) -> Result<()> {
        if self.fenced {
            return Err(corrupt());
        }
        if read(&mut self.storage, context, self.scope)? != self.raw {
            self.fenced = true;
            return Err(corrupt());
        }
        Ok(())
    }
    /// A Tree material import must verify its own independently retained ACK;
    /// an equal-looking pin under another native profile cannot substitute.
    pub(crate) fn verify_tree(
        &mut self,
        context: &OperationContext,
        scope: MapCuboid,
    ) -> Result<()> {
        check(matches!(self.protocol, Protocol::Tree) && self.scope == Some(scope))?;
        self.verify(context)
    }
    /// A production importer holds the independent Workshop acknowledgement
    /// read-only. An equal-looking Tree/Manager pin cannot stand in for it.
    pub(crate) fn verify_workshop_offline(&mut self, context: &OperationContext) -> Result<()> {
        check(matches!(self.protocol, Protocol::Workshop) && self.scope.is_none() && self.offline)?;
        self.verify(context)
    }
    pub fn acknowledge(&mut self, pin: WorkshopPin, context: &OperationContext) -> Result<()> {
        self.verify(context)?;
        if self.latest == Some(pin) {
            return Ok(());
        }
        check(!self.offline && self.raw.len() + RECORD <= MAX_BYTES)?;
        if let Some(old) = self.latest {
            check(
                pin.journal_id == old.journal_id && pin.frames > old.frames && pin.head != old.head,
            )?;
        } else {
            check(pin.frames == 0 && pin.head == pin.journal_id)?;
        }
        check(
            pin.frames <= MAX_FRAMES
                && pin.journal_id != Digest32::ZERO
                && pin.head != Digest32::ZERO,
        )?;
        let mut record = self.protocol.magic().to_vec();
        record.extend_from_slice(pin.journal_id.as_bytes());
        record.extend_from_slice(pin.head.as_bytes());
        record.extend_from_slice(&pin.frames.to_be_bytes());
        record.extend_from_slice(self.chain.as_bytes());
        let chain = digest(self.protocol, &record);
        record.extend_from_slice(chain.as_bytes());
        self.fenced = true;
        self.storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())?;
        self.storage
            .write_all(&record)
            .and_then(|_| self.storage.flush())
            .and_then(|_| self.storage.sync())
            .map_err(|_| corrupt())?;
        let mut expected = self.raw.clone();
        expected.extend_from_slice(&record);
        check(read(&mut self.storage, context, self.scope)? == expected)?;
        self.raw = expected;
        self.latest = Some(pin);
        self.chain = chain;
        self.fenced = false;
        Ok(())
    }
}
