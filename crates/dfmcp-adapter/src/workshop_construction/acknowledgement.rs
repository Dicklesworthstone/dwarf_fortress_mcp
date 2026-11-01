//! Independent, descriptor-pinned acknowledgements for the workshop consumer.
//! This file is separate from workshop.journal. It must be synchronized BEFORE
//! a native writer, not exported after the MCP call. It is local crash/rollback
//! custody, not distributed consensus or protection against rolling back both
//! stores and every externally held acknowledgement together.
use super::journal::{MAX_FRAMES, WorkshopMode, WorkshopPin};
use crate::build_placement::journal::{
    BuildMode,
    private_file::{PrivateBuildFile, open_private_storage},
};
use crate::control_effect_journal::EffectJournalStorage;
use dfmcp_core::{Capability, Digest32, ErrorCode, OperationContext, Result, RiskTier};
use std::io::SeekFrom;
use std::path::Path;
use std::time::{Duration, Instant};

const MAGIC: &[u8; 8] = b"DFMWA020";
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
fn digest(bytes: &[u8]) -> Digest32 {
    let mut data = b"dfmcp-workshop-acknowledgement/1\0".to_vec();
    data.extend_from_slice(bytes);
    Digest32::of_bytes(&data)
}
fn read<S: EffectJournalStorage>(storage: &mut S, context: &OperationContext) -> Result<Vec<u8>> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
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
    let storage_mode = match mode {
        WorkshopMode::Control => BuildMode::Control,
        WorkshopMode::Recover => BuildMode::Recover,
        WorkshopMode::Offline => BuildMode::Offline,
    };
    let (storage, created, current) =
        open_private_storage(path, context, storage_mode, initialize, MAX_BYTES)?;
    check(created == initialize)?;
    WorkshopAcknowledgements::open(storage, &current, mode, initialize, external)
}
impl<S: EffectJournalStorage> WorkshopAcknowledgements<S> {
    pub fn open(
        mut storage: S,
        context: &OperationContext,
        mode: WorkshopMode,
        initialize: bool,
        external: Option<WorkshopPin>,
    ) -> Result<Self> {
        let raw = read(&mut storage, context)?;
        check(
            (initialize && raw.is_empty() && external.is_none() && mode == WorkshopMode::Control)
                || (!initialize && !raw.is_empty() && external.is_some()),
        )?;
        check(raw.len().is_multiple_of(RECORD))?;
        let mut latest = None;
        let mut chain = Digest32::ZERO;
        let mut acknowledged = false;
        for record in raw.as_chunks::<RECORD>().0 {
            check(&record[..8] == MAGIC && &record[76..108] == chain.as_bytes())?;
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
            chain = digest(&record[..108]);
            check(&record[108..] == chain.as_bytes())?;
            acknowledged |= external == Some(pin);
            latest = Some(pin);
        }
        check(initialize || acknowledged)?;
        Ok(Self {
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
        if read(&mut self.storage, context)? != self.raw {
            self.fenced = true;
            return Err(corrupt());
        }
        Ok(())
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
        let mut record = MAGIC.to_vec();
        record.extend_from_slice(pin.journal_id.as_bytes());
        record.extend_from_slice(pin.head.as_bytes());
        record.extend_from_slice(&pin.frames.to_be_bytes());
        record.extend_from_slice(self.chain.as_bytes());
        let chain = digest(&record);
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
        check(read(&mut self.storage, context)? == expected)?;
        self.raw = expected;
        self.latest = Some(pin);
        self.chain = chain;
        self.fenced = false;
        Ok(())
    }
}
