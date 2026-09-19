//! Bounded recovery discovery over the exact retained journal generation.
//! Query-only recovery may append reconciliation evidence but cannot initialize
//! a journal, manufacture grants, or bypass per-call production authorization.
use std::ops::Bound::{Excluded, Unbounded};

use super::{authorize, corrupt, exhausted, io_error, Budget, DurableJobRecord,
    DurableJobState, EffectJournalStorage, JobControlJournal, MAX_BODY};
use dfmcp_core::{Digest32, FortressId, OperationContext, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobJournalSummary {
    pub fortress_id: FortressId,
    pub journal_id: Digest32,
    pub head: Digest32,
    pub retained_bytes: u64,
    pub transitions: u64,
    pub records: usize,
    pub prepared: usize,
    pub unresolved: usize,
    pub terminal: usize,
    pub read_only: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobRecordPage {
    pub head: Digest32,
    pub records: Vec<DurableJobRecord>,
    /// Last returned key, only when another complete record remains.
    pub next_after: Option<String>,
}

impl<S: EffectJournalStorage> JobControlJournal<S> {
    /// Open existing bytes with Query authority and a writable injected store.
    /// This never initializes or repairs bytes. Mutations still require an
    /// independently granted ConfigureProduction capability on each call.
    pub fn open_for_reconciliation(storage: S, context: &OperationContext) -> Result<Self> {
        let mut journal = Self::open_read_only(storage, context)?;
        journal.read_only = false;
        Ok(journal)
    }

    /// Recheck custody even when the answer can be satisfied from cached state.
    pub fn validate_access(&self, context: &OperationContext) -> Result<()> {
        authorize(context, self.fortress, false)?;
        if self.fenced {
            return Err(corrupt("job journal fenced; reopen for verified recovery"));
        }
        self.storage.validate_identity().map_err(io_error)
    }

    pub fn summary(&self, context: &OperationContext) -> Result<JobJournalSummary> {
        self.validate_access(context)?;
        let budget = Budget::new(context)?;
        let mut prepared = 0;
        let mut unresolved = 0;
        let mut terminal = 0;
        for record in self.records.values() {
            budget.remaining()?;
            if record.state.terminal() { terminal += 1; }
            else if record.state == DurableJobState::Prepared { prepared += 1; }
            else { unresolved += 1; }
        }
        self.validate_access(context)?;
        Ok(JobJournalSummary {
            fortress_id: self.fortress, journal_id: self.id, head: self.head,
            retained_bytes: self.length, transitions: self.transitions,
            records: self.records.len(), prepared, unresolved, terminal,
            read_only: self.read_only,
        })
    }

    /// Deterministic whole-record keyset page, including terminal records.
    /// A continuation must carry this head, not an index into changing state.
    pub fn records_page(&self, expected_head: Digest32, after: Option<&str>,
        limit: usize, context: &OperationContext) -> Result<JobRecordPage>
    {
        self.validate_access(context)?;
        if expected_head != self.head {
            return Err(super::fail(dfmcp_core::ErrorCode::StaleAnchor,
                "job journal changed; restart effect discovery"));
        }
        if limit == 0 || limit > 64 || limit > context.budget.max_entities as usize {
            return Err(exhausted());
        }
        let start = match after {
            Some(key) => {
                super::validate_key(key)?;
                if !self.records.contains_key(key) {
                    return Err(super::conflict("job discovery continuation names no retained key"));
                }
                Excluded(key)
            }
            None => Unbounded,
        };
        let mut budget = Budget::new(context)?;
        let mut values = self.records.range::<str, _>((start, Unbounded));
        let mut records = Vec::new();
        for (_, record) in values.by_ref().take(limit) {
            budget.charge(MAX_BODY as u64)?;
            records.push(record.clone());
        }
        let more = values.next().is_some();
        let next_after = if more { records.last().map(|r| r.plan.key().to_owned()) } else { None };
        self.validate_access(context)?;
        budget.remaining()?;
        Ok(JobRecordPage { head: self.head, records, next_after })
    }
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
