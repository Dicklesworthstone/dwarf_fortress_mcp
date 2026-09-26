//! Bounded multi-record projection without retaining a collection of worlds.
//! Framing, decoding and generation publication reuse the journal's fixed codec.
use super::super::*;

impl<S: JournalStorage, P: JournalProfile> ObservationJournal<S, P> {
    /// Reconstruct the prefix once and project at most 33 exact, increasing
    /// records. The extra record permits a 32-row page to compare its first row
    /// with the preceding sample. The current world and journal bytes never change.
    ///
    /// `project` must be a bounded, side-effect-free projection: no result is
    /// trustworthy until this method returns Ok, including its final custody and
    /// deadline checks. Only projected values are retained, not historical worlds.
    /// Authority is always checked at the caller's CURRENT anchor. A historical
    /// tick never revives an expired grant. No write or sync occurs on this path.
    pub fn project_records<T, F>(
        &mut self,
        records: &[(u64, Digest32)],
        context: &OperationContext,
        mut project: F,
    ) -> Result<Vec<(JournalEntry, T)>>
    where
        F: FnMut(&JournalEntry, &P::State) -> Result<T>,
    {
        let result = self.project_records_inner(records, context, &mut project);
        if matches!(&result, Err(e) if e.code == ErrorCode::CorruptLedger) {
            self.fenced = true;
        }
        result
    }

    fn project_records_inner<T, F>(
        &mut self,
        records: &[(u64, Digest32)],
        context: &OperationContext,
        project: &mut F,
    ) -> Result<Vec<(JournalEntry, T)>>
    where
        F: FnMut(&JournalEntry, &P::State) -> Result<T>,
    {
        let started = Instant::now();
        let check_now = || -> Result<()> {
            check(context, started)?;
            if started.elapsed().as_millis() >= u128::from(context.budget.max_wall_millis) {
                return Err(budget(
                    "historical projections exhausted their shared replay deadline",
                ));
            }
            Ok(())
        };
        check_now()?;
        self.ensure_healthy(context)?;
        if records.is_empty() || records.len() > 33 {
            return Err(budget(
                "project one to 33 exact historical records per pass",
            ));
        }
        let mut previous_number = 0;
        // Validate the WHOLE selection before executing any projection callback.
        for &(number, digest) in records {
            if number <= previous_number {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "historical projection records must be strictly increasing",
                ));
            }
            let index = usize::try_from(number - 1).map_err(|_| {
                DfmcpError::new(ErrorCode::CursorGap, "historical record index overflow")
            })?;
            let entry = self.entries.get(index).ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::CursorGap,
                    "historical projection record is not retained",
                )
            })?;
            if entry.record_digest != digest {
                return Err(DfmcpError::new(
                    ErrorCode::StaleAnchor,
                    "historical projection digest differs",
                ));
            }
            previous_number = number;
        }
        self.storage
            .seek(SeekFrom::Start(0))
            .map_err(storage_error)?;
        let mut header = [0; HEADER_BYTES];
        self.storage
            .read_exact(&mut header)
            .map_err(storage_error)?;
        if decode_header::<P>(&header)? != (self.fortress, self.id, self.header_digest) {
            return Err(corrupt(
                "historical projection header changed after opening",
            ));
        }
        let mut state = P::empty();
        let mut predecessor = self.header_digest;
        let mut payload_base = None;
        let mut projected = Vec::with_capacity(records.len());
        let last = usize::try_from(previous_number).map_err(|_| {
            DfmcpError::new(ErrorCode::CursorGap, "historical projection bound overflow")
        })?;
        for index in 0..last {
            check_now()?;
            let known = &self.entries[index];
            self.storage
                .seek(SeekFrom::Start(known.offset))
                .map_err(storage_error)?;
            let mut frame = vec![0; known.encoded_bytes as usize];
            self.storage.read_exact(&mut frame).map_err(storage_error)?;
            // Carry every verified predecessor, including records omitted from
            // the selection. Delta payloads always refer to the preceding record,
            // not the preceding projected row. Use the same production decoder
            // as open/state_at and apply the expanded budget before allocation.
            let decoded = compression::decode::<P>(
                &frame,
                self.id,
                known.offset,
                payload_base.as_ref(),
                expanded_allowance::<P>(context),
            )?;
            let entry = decoded.entry;
            let observation = decoded.observation;
            if &entry != known
                || entry.previous_digest != predecessor
                || entry.anchor.fortress_id != self.fortress
                || P::source_digest(&observation)? != entry.source_digest
            {
                return Err(corrupt(
                    "historical projection record or predecessor changed",
                ));
            }
            check_observation::<P>(&observation, context)?;
            if P::publish(&mut state, observation)? == JobPublication::Heartbeat
                || P::snapshot(&state).map(WorldSnapshot::anchor) != Some(entry.anchor)
            {
                return Err(corrupt(
                    "historical projection does not reproduce its exact anchor",
                ));
            }
            predecessor = entry.record_digest;
            payload_base = decoded.payload_base;
            if records
                .get(projected.len())
                .is_some_and(|(number, _)| *number == entry.number)
            {
                check_now()?;
                let value = project(&entry, &state)?;
                check_now()?;
                projected.push((entry, value));
            }
        }
        if projected.len() != records.len() {
            return Err(corrupt(
                "historical projection did not visit its entire selection",
            ));
        }
        // Detect a changed length/custody even if a projection or cooperating
        // writer changed storage after the last read. Nothing has been published.
        self.ensure_healthy(context)?;
        check_now()?;
        Ok(projected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live_operations::OperationsProfile;
    use crate::live_spatial::LiveSpatialObservation;
    use dfmcp_core::{CapabilityGrant, CapabilityScope, RequestId, SessionId, WorkBudget};
    use std::io::Cursor;

    #[derive(Default)]
    struct Memory {
        bytes: Cursor<Vec<u8>>,
        read_bytes: usize,
        writes: usize,
        syncs: usize,
    }
    impl Read for Memory {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            let n = self.bytes.read(out)?;
            self.read_bytes += n;
            Ok(n)
        }
    }
    impl Seek for Memory {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.bytes.seek(position)
        }
    }
    impl Write for Memory {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            self.bytes.write(data)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl JournalStorage for Memory {
        fn sync(&mut self) -> io::Result<()> {
            self.syncs += 1;
            Ok(())
        }
        fn truncate(&mut self, n: u64) -> io::Result<()> {
            self.bytes.get_mut().truncate(n as usize);
            Ok(())
        }
    }
    pub(super) fn observation(tick: u32) -> Result<LiveSpatialObservation> {
        let hex = include_str!("../tests/fixtures/spatial_v1_6.hex").trim();
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| corrupt("fixture hex")))
            .collect::<Result<Vec<_>>>()?;
        let source =
            LiveSpatialObservation::decode_payload(&bytes, 7, "df".into(), "dfhack".into())?;
        let mut operations = source.operations().clone();
        let mut terrain = source.terrain().clone();
        operations.jobs.year_tick = tick;
        terrain.year_tick = tick;
        let mut bytes = b"DFMS1600".to_vec();
        for part in [
            operations.encode_profile(OperationsProfile::PagedV1_4)?,
            terrain.encode_payload()?,
        ] {
            bytes.extend_from_slice(&(part.len() as u32).to_be_bytes());
            bytes.extend_from_slice(&part);
        }
        LiveSpatialObservation::decode_payload(&bytes, 7, "df".into(), "dfhack".into())
    }
    fn fixture() -> Result<(ObservationJournal<Memory, Spatial16>, OperationContext)> {
        let mut state = crate::live_spatial::LiveSpatialState::default();
        state.publish(observation(3)?)?;
        let anchor = state
            .snapshot()
            .ok_or_else(|| corrupt("fixture state"))?
            .anchor();
        let mut context = OperationContext {
            session_id: SessionId::new(98_301),
            request_id: RequestId::new(1),
            anchor,
            budget: WorkBudget {
                max_entities: 100_000,
                max_bytes: 16 * 1024 * 1024,
                max_wall_millis: 60_000,
                ..WorkBudget::default()
            },
            cancellation_requested: false,
            grants: [Capability::Query, Capability::Observe]
                .into_iter()
                .map(|capability| CapabilityGrant {
                    capability,
                    scope: CapabilityScope::default(),
                    max_risk: RiskTier::ReadOnly,
                    expires_at_tick: None,
                    remaining_uses: None,
                })
                .collect(),
        };
        let mut journal = ObservationJournal::<Memory, Spatial16>::open(
            Memory::default(),
            &context,
            JournalLimits::default(),
            true,
            TailRecovery::Refuse,
        )?;
        for tick in 3..7 {
            journal.append(observation(tick)?, &context)?;
        }
        context.anchor = journal
            .state()
            .snapshot()
            .ok_or_else(|| corrupt("fixture latest"))?
            .anchor();
        context.grants.retain(|g| g.capability == Capability::Query);
        journal.storage.read_bytes = 0;
        Ok((journal, context))
    }
    #[test]
    fn projects_one_prefix_without_writes_or_replacing_current_state() -> Result<()> {
        let (mut journal, context) = fixture()?;
        let wanted: Vec<_> = journal
            .entries()
            .iter()
            .take(3)
            .map(|e| (e.number, e.record_digest))
            .collect();
        let prior = journal.state().clone();
        let bytes = journal.storage.bytes.get_ref().clone();
        let mutations = (journal.storage.writes, journal.storage.syncs);
        let expected_read =
            journal.entries()[2].offset as usize + journal.entries()[2].encoded_bytes as usize;
        let result = journal.project_records(&wanted, &context, |_, s| {
            Ok(s.snapshot().ok_or_else(|| corrupt("snapshot"))?.anchor())
        })?;
        assert_eq!(result.len(), 3);
        for (entry, anchor) in result {
            assert_eq!(entry.anchor, anchor);
        }
        assert_eq!(journal.storage.read_bytes, expected_read);
        assert_eq!(journal.state().snapshot(), prior.snapshot());
        assert_eq!(journal.storage.bytes.get_ref(), &bytes);
        assert_eq!((journal.storage.writes, journal.storage.syncs), mutations);
        Ok(())
    }
    #[test]
    fn validates_all_record_identities_before_running_a_projection() -> Result<()> {
        let (mut journal, context) = fixture()?;
        let first = journal.entries()[0].clone();
        for records in [
            vec![],
            vec![(0, first.record_digest)],
            vec![(1, first.record_digest); 34],
            vec![(1, first.record_digest), (1, first.record_digest)],
            vec![(1, first.record_digest), (4, Digest32::ZERO)],
            vec![(1, first.record_digest), (99, first.record_digest)],
        ] {
            let mut calls = 0;
            assert!(
                journal
                    .project_records(&records, &context, |_, _| {
                        calls += 1;
                        Ok(())
                    })
                    .is_err()
            );
            assert_eq!(calls, 0);
            assert!(!journal.fenced());
        }
        Ok(())
    }
    #[test]
    fn late_projection_refusal_returns_no_partial_result_or_new_root() -> Result<()> {
        let (mut journal, context) = fixture()?;
        let wanted: Vec<_> = journal
            .entries()
            .iter()
            .map(|e| (e.number, e.record_digest))
            .collect();
        let result = journal.project_records(&wanted, &context, |e, _| {
            if e.number == 3 {
                Err(budget("injected late refusal"))
            } else {
                Ok(e.number)
            }
        });
        assert!(matches!(result,Err(e) if e.code == ErrorCode::BudgetExceeded));
        assert_eq!(
            journal.state().snapshot().map(WorldSnapshot::anchor),
            Some(context.anchor)
        );
        assert!(!journal.fenced());
        Ok(())
    }
    #[test]
    fn corruption_in_unselected_prefix_is_not_skipped() -> Result<()> {
        let (mut journal, context) = fixture()?;
        let last = journal.entries()[3].clone();
        let offset = journal.entries()[1].offset as usize + FRAME_HEADER_BYTES + 10;
        journal.storage.bytes.get_mut()[offset] ^= 1;
        let mut calls = 0;
        assert!(
            matches!(journal.project_records(&[(last.number,last.record_digest)],&context,|_,_| {
            calls += 1; Ok(())
        }),Err(e) if e.code == ErrorCode::CorruptLedger)
        );
        assert_eq!(calls, 0);
        assert!(journal.fenced());
        Ok(())
    }
    #[test]
    fn historical_records_do_not_revive_current_authority() -> Result<()> {
        let (mut journal, context) = fixture()?;
        let first = journal.entries()[0].clone();
        for case in 0..4 {
            let mut denied = context.clone();
            match case {
                0 => denied.grants.clear(),
                1 => denied.cancellation_requested = true,
                2 => denied.grants[0].expires_at_tick = Some(first.anchor.tick),
                _ => denied.budget.max_entities = 1,
            }
            assert!(
                journal
                    .project_records(&[(first.number, first.record_digest)], &denied, |_, _| Ok(
                        ()
                    ))
                    .is_err()
            );
            assert!(!journal.fenced());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "observation_projection_compressed_tests.rs"]
mod compressed_tests;
