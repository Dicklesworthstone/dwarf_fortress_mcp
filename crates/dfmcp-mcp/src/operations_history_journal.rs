//! Closed runtime ownership of the two existing operations journal profiles.
//!
//! The bootstrap fixes the codec before opening storage. Neither incoming bytes
//! nor an MCP request can select a profile or reinterpret an existing archive.

use dfmcp_adapter::live_jobs::JobPublication;
use dfmcp_adapter::live_operations::{
    LiveOperationsObservation, LiveOperationsState, OperationsProfile,
};
use dfmcp_adapter::operations_journal::{
    JournalEntry, JournalLimits, Operations14, OperationsJournal, PagedOperationsJournal,
    PrivateJournalFile, TailRecovery, open_private_journal, open_profile_journal,
};
use dfmcp_core::{Digest32, OperationContext, Result};
use dfmcp_world::WorldSnapshot;
use std::path::Path;

pub(crate) enum Journal {
    Single(OperationsJournal<PrivateJournalFile>),
    Paged(PagedOperationsJournal<PrivateJournalFile>),
}

impl Journal {
    pub(crate) fn open(
        profile: OperationsProfile,
        path: &Path,
        context: &OperationContext,
        recovery: TailRecovery,
    ) -> Result<Self> {
        match profile {
            OperationsProfile::V1_3 => {
                open_private_journal(path, context, JournalLimits::default(), recovery)
                    .map(Self::Single)
            }
            OperationsProfile::PagedV1_4 => open_profile_journal::<Operations14>(
                path,
                context,
                JournalLimits::default(),
                recovery,
            )
            .map(Self::Paged),
        }
    }

    pub(crate) fn profile(&self) -> OperationsProfile {
        match self {
            Self::Single(_) => OperationsProfile::V1_3,
            Self::Paged(_) => OperationsProfile::PagedV1_4,
        }
    }

    pub(crate) fn state(&self) -> &LiveOperationsState {
        match self {
            Self::Single(journal) => journal.state(),
            Self::Paged(journal) => journal.state(),
        }
    }

    pub(crate) fn append(
        &mut self,
        observation: LiveOperationsObservation,
        context: &OperationContext,
    ) -> Result<JobPublication> {
        match self {
            Self::Single(journal) => journal.append(observation, context),
            Self::Paged(journal) => journal.append(observation, context),
        }
    }

    pub(crate) fn snapshot_at(
        &mut self,
        number: u64,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<WorldSnapshot> {
        match self {
            Self::Single(journal) => journal.snapshot_at(number, digest, context),
            Self::Paged(journal) => journal.snapshot_at(number, digest, context),
        }
    }

    pub(crate) fn validate_custody(&mut self, context: &OperationContext) -> Result<()> {
        match self {
            Self::Single(journal) => journal.validate_custody(context),
            Self::Paged(journal) => journal.validate_custody(context),
        }
    }

    pub(crate) fn entries(&self) -> &[JournalEntry] {
        match self {
            Self::Single(journal) => journal.entries(),
            Self::Paged(journal) => journal.entries(),
        }
    }

    pub(crate) fn id(&self) -> Digest32 {
        match self {
            Self::Single(journal) => journal.id(),
            Self::Paged(journal) => journal.id(),
        }
    }

    pub(crate) fn head(&self) -> Digest32 {
        match self {
            Self::Single(journal) => journal.head(),
            Self::Paged(journal) => journal.head(),
        }
    }

    pub(crate) fn retained_bytes(&self) -> u64 {
        match self {
            Self::Single(journal) => journal.retained_bytes(),
            Self::Paged(journal) => journal.retained_bytes(),
        }
    }

    pub(crate) fn repaired_tail_bytes(&self) -> u64 {
        match self {
            Self::Single(journal) => journal.repaired_tail_bytes(),
            Self::Paged(journal) => journal.repaired_tail_bytes(),
        }
    }

    pub(crate) fn fenced(&self) -> bool {
        match self {
            Self::Single(journal) => journal.fenced(),
            Self::Paged(journal) => journal.fenced(),
        }
    }
}
