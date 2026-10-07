//! State anchor v2: the complete version tuple of `ARCHITECTURE.md`
//! ("One version universe").
//!
//! A v2 anchor names one world version together with every epoch that changes
//! what a handle bound to it means: the fortress lineage, observation epoch
//! and snapshot sequence, the optional game tick, the bridge generation and
//! protocol, and the adapter, schema and policy epochs, plus the semantic
//! world root. It is read as one complete tuple: the canonical encoding is
//! fixed-width and strictly decoded, so a partial tuple cannot be constructed
//! from bytes. Equality, ordering and the digest all derive from the canonical
//! bytes. This type is additive; v1 [`StateAnchor`] remains the wire anchor.

use std::cmp::Ordering;

use crate::{DfmcpError, Digest32, ErrorCode, FortressId, GameTick, Result, StateAnchor};

const DOMAIN: &[u8] = b"dfmcp-state-anchor/2\0";
/// Largest bridge-protocol label an anchor carries.
pub const MAX_PROTOCOL_BYTES: usize = 32;

/// The epochs a v1 anchor does not carry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnchorEpochs {
    /// Identity of the fortress world lineage (a save, not a session).
    pub fortress_lineage: Digest32,
    /// Bridge process generation; a restart advances it.
    pub bridge_generation: u64,
    /// Exact bridge protocol label (e.g. `1.0`, or `lab` for the laboratory).
    pub bridge_protocol: String,
    /// Adapter implementation epoch.
    pub adapter_epoch: u64,
    /// Canonical schema epoch.
    pub schema_epoch: u64,
    /// Authority/policy epoch.
    pub policy_epoch: u64,
}

impl AnchorEpochs {
    /// The deterministic laboratory's epochs for `fortress`.
    #[must_use]
    pub fn laboratory(fortress: FortressId) -> Self {
        let mut lineage = b"dfmcp-lab-lineage/1\0".to_vec();
        lineage.extend_from_slice(&fortress.get().to_be_bytes());
        Self {
            fortress_lineage: Digest32::of_bytes(&lineage),
            bridge_generation: 0,
            bridge_protocol: "lab".to_owned(),
            adapter_epoch: 1,
            schema_epoch: 1,
            policy_epoch: 1,
        }
    }
}

/// The complete v2 anchor tuple.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StateAnchorV2 {
    pub fortress_lineage: Digest32,
    pub observation_epoch: u64,
    pub snapshot_sequence: u64,
    pub game_tick: Option<GameTick>,
    pub bridge_generation: u64,
    pub bridge_protocol: String,
    pub adapter_epoch: u64,
    pub schema_epoch: u64,
    pub policy_epoch: u64,
    pub semantic_world_root: Digest32,
}

/// How a newer anchor relates to an older one a handle was bound to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnchorContinuity {
    /// The very same version.
    Same,
    /// A later snapshot in the same epoch: older handles may be revalidated.
    Advanced,
    /// A new observation epoch (restore, world switch, clock regression,
    /// ambiguous bridge restart): every older handle is stale.
    NewEpoch,
    /// Not comparable: older handles must be discarded, and why.
    Incompatible(&'static str),
}

impl AnchorContinuity {
    /// Whether a handle bound to the older anchor may still be revalidated.
    #[must_use]
    pub const fn admits_older_handles(&self) -> bool {
        matches!(self, Self::Same | Self::Advanced)
    }
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

impl StateAnchorV2 {
    /// Lifts a v1 anchor with the epochs it does not carry.
    pub fn from_v1(anchor: StateAnchor, epochs: AnchorEpochs) -> Result<Self> {
        let lifted = Self {
            fortress_lineage: epochs.fortress_lineage,
            observation_epoch: anchor.cursor.epoch,
            snapshot_sequence: anchor.cursor.sequence,
            game_tick: Some(anchor.tick),
            bridge_generation: epochs.bridge_generation,
            bridge_protocol: epochs.bridge_protocol,
            adapter_epoch: epochs.adapter_epoch,
            schema_epoch: epochs.schema_epoch,
            policy_epoch: epochs.policy_epoch,
            semantic_world_root: anchor.state_hash,
        };
        lifted.validate()?;
        Ok(lifted)
    }

    fn validate(&self) -> Result<()> {
        if self.bridge_protocol.is_empty()
            || self.bridge_protocol.len() > MAX_PROTOCOL_BYTES
            || !self
                .bridge_protocol
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
        {
            return Err(invalid(
                "bridge protocol label must be 1..=32 bytes of [A-Za-z0-9._-]",
            ));
        }
        Ok(())
    }

    /// Fixed-layout canonical bytes: domain, lineage, epoch, sequence, tick
    /// tag and value, bridge generation, protocol length and bytes, adapter,
    /// schema and policy epochs, world root.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = DOMAIN.to_vec();
        out.extend_from_slice(self.fortress_lineage.as_bytes());
        out.extend_from_slice(&self.observation_epoch.to_be_bytes());
        out.extend_from_slice(&self.snapshot_sequence.to_be_bytes());
        match self.game_tick {
            Some(tick) => {
                out.push(1);
                out.extend_from_slice(&tick.0.to_be_bytes());
            }
            None => {
                out.push(0);
                out.extend_from_slice(&0_u64.to_be_bytes());
            }
        }
        out.extend_from_slice(&self.bridge_generation.to_be_bytes());
        out.push(self.bridge_protocol.len() as u8);
        out.extend_from_slice(self.bridge_protocol.as_bytes());
        out.extend_from_slice(&self.adapter_epoch.to_be_bytes());
        out.extend_from_slice(&self.schema_epoch.to_be_bytes());
        out.extend_from_slice(&self.policy_epoch.to_be_bytes());
        out.extend_from_slice(self.semantic_world_root.as_bytes());
        out
    }

    /// Strict inverse of [`Self::canonical_bytes`]: a truncated, extended or
    /// non-canonical tuple is refused, never completed with defaults.
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self> {
        let mut rest = bytes
            .strip_prefix(DOMAIN)
            .ok_or_else(|| invalid("not a v2 state anchor"))?;
        let mut take = |n: usize| -> Result<&[u8]> {
            if rest.len() < n {
                return Err(invalid("v2 state anchor is truncated"));
            }
            let (head, tail) = rest.split_at(n);
            rest = tail;
            Ok(head)
        };
        let digest = |raw: &[u8]| -> Result<Digest32> {
            let array: [u8; 32] = raw
                .try_into()
                .map_err(|_| invalid("v2 state anchor digest has the wrong width"))?;
            Ok(Digest32::from_bytes(array))
        };
        let u64_of = |raw: &[u8]| -> Result<u64> {
            let array: [u8; 8] = raw
                .try_into()
                .map_err(|_| invalid("v2 state anchor integer has the wrong width"))?;
            Ok(u64::from_be_bytes(array))
        };
        let fortress_lineage = digest(take(32)?)?;
        let observation_epoch = u64_of(take(8)?)?;
        let snapshot_sequence = u64_of(take(8)?)?;
        let tag = take(1)?[0];
        let tick = u64_of(take(8)?)?;
        let game_tick = match (tag, tick) {
            (1, tick) => Some(GameTick(tick)),
            (0, 0) => None,
            _ => return Err(invalid("v2 state anchor tick tag is not canonical")),
        };
        let bridge_generation = u64_of(take(8)?)?;
        let protocol_len = usize::from(take(1)?[0]);
        let bridge_protocol = std::str::from_utf8(take(protocol_len)?)
            .map_err(|_| invalid("v2 state anchor protocol is not UTF-8"))?
            .to_owned();
        let anchor = Self {
            fortress_lineage,
            observation_epoch,
            snapshot_sequence,
            game_tick,
            bridge_generation,
            bridge_protocol,
            adapter_epoch: u64_of(take(8)?)?,
            schema_epoch: u64_of(take(8)?)?,
            policy_epoch: u64_of(take(8)?)?,
            semantic_world_root: digest(take(32)?)?,
        };
        if !rest.is_empty() {
            return Err(invalid("v2 state anchor has trailing bytes"));
        }
        anchor.validate()?;
        Ok(anchor)
    }

    /// SHA-256 of the canonical bytes.
    #[must_use]
    pub fn digest(&self) -> Digest32 {
        Digest32::of_bytes(&self.canonical_bytes())
    }

    /// How this anchor relates to `older`, the anchor a handle was bound to.
    #[must_use]
    pub fn continuity_from(&self, older: &Self) -> AnchorContinuity {
        if self.fortress_lineage != older.fortress_lineage {
            return AnchorContinuity::Incompatible("a different fortress lineage");
        }
        if self.bridge_protocol != older.bridge_protocol {
            return AnchorContinuity::Incompatible("a different bridge protocol");
        }
        if (self.adapter_epoch, self.schema_epoch, self.policy_epoch)
            != (older.adapter_epoch, older.schema_epoch, older.policy_epoch)
        {
            return AnchorContinuity::Incompatible("an adapter, schema or policy epoch changed");
        }
        match self.observation_epoch.cmp(&older.observation_epoch) {
            Ordering::Less => AnchorContinuity::Incompatible("the observation epoch regressed"),
            Ordering::Greater => AnchorContinuity::NewEpoch,
            Ordering::Equal if self.bridge_generation != older.bridge_generation => {
                AnchorContinuity::Incompatible("the bridge restarted within one observation epoch")
            }
            Ordering::Equal => match self.snapshot_sequence.cmp(&older.snapshot_sequence) {
                Ordering::Less => AnchorContinuity::Incompatible("the snapshot sequence regressed"),
                Ordering::Greater => AnchorContinuity::Advanced,
                Ordering::Equal if self == older => AnchorContinuity::Same,
                Ordering::Equal => {
                    AnchorContinuity::Incompatible("one sequence names two different versions")
                }
            },
        }
    }
}

impl PartialOrd for StateAnchorV2 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StateAnchorV2 {
    /// Ordered by canonical bytes.
    fn cmp(&self, other: &Self) -> Ordering {
        self.canonical_bytes().cmp(&other.canonical_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ObservationCursor;

    fn v1(epoch: u64, sequence: u64) -> StateAnchor {
        StateAnchor {
            fortress_id: FortressId::new(7),
            cursor: ObservationCursor { epoch, sequence },
            tick: GameTick(100 + sequence),
            state_hash: Digest32::of_bytes(&sequence.to_be_bytes()),
        }
    }

    fn lab(epoch: u64, sequence: u64) -> Result<StateAnchorV2> {
        StateAnchorV2::from_v1(
            v1(epoch, sequence),
            AnchorEpochs::laboratory(FortressId::new(7)),
        )
    }

    #[test]
    fn canonical_bytes_round_trip_and_partial_tuples_are_refused() -> Result<()> {
        let anchor = lab(2, 9)?;
        let bytes = anchor.canonical_bytes();
        assert_eq!(StateAnchorV2::from_canonical_bytes(&bytes)?, anchor);
        // Golden layout: domain + 32 + 8 + 8 + 9 + 8 + 1 + "lab" + 8 + 8 + 8 + 32.
        assert_eq!(
            bytes.len(),
            DOMAIN.len() + 32 + 8 + 8 + 9 + 8 + 1 + 3 + 24 + 32
        );
        for cut in 0..bytes.len() {
            assert!(
                StateAnchorV2::from_canonical_bytes(&bytes[..cut]).is_err(),
                "cut {cut}"
            );
        }
        let mut extended = bytes.clone();
        extended.push(0);
        assert!(StateAnchorV2::from_canonical_bytes(&extended).is_err());
        let mut no_tick = anchor.clone();
        no_tick.game_tick = None;
        assert_eq!(
            StateAnchorV2::from_canonical_bytes(&no_tick.canonical_bytes())?,
            no_tick
        );
        // A tickless tag with a nonzero tick is not canonical.
        let mut forged = no_tick.canonical_bytes();
        let tick_at = DOMAIN.len() + 32 + 16 + 1;
        forged[tick_at + 7] = 1;
        assert!(StateAnchorV2::from_canonical_bytes(&forged).is_err());
        Ok(())
    }

    #[test]
    fn every_component_participates_in_the_digest() -> Result<()> {
        let base = lab(1, 1)?;
        let mutations: Vec<fn(&mut StateAnchorV2)> = vec![
            |a| a.fortress_lineage = Digest32::of_bytes(b"other"),
            |a| a.observation_epoch += 1,
            |a| a.snapshot_sequence += 1,
            |a| a.game_tick = None,
            |a| a.bridge_generation += 1,
            |a| a.bridge_protocol = "1.0".to_owned(),
            |a| a.adapter_epoch += 1,
            |a| a.schema_epoch += 1,
            |a| a.policy_epoch += 1,
            |a| a.semantic_world_root = Digest32::ZERO,
        ];
        let mut seen = std::collections::BTreeSet::from([base.digest()]);
        for mutate in mutations {
            let mut changed = base.clone();
            mutate(&mut changed);
            assert!(seen.insert(changed.digest()), "{changed:?}");
        }
        Ok(())
    }

    #[test]
    fn continuity_is_total_and_new_epochs_invalidate_older_handles() -> Result<()> {
        let older = lab(1, 4)?;
        assert_eq!(lab(1, 4)?.continuity_from(&older), AnchorContinuity::Same);
        assert_eq!(
            lab(1, 5)?.continuity_from(&older),
            AnchorContinuity::Advanced
        );
        let restored = lab(2, 0)?;
        assert_eq!(restored.continuity_from(&older), AnchorContinuity::NewEpoch);
        assert!(!restored.continuity_from(&older).admits_older_handles());
        assert!(matches!(
            lab(1, 3)?.continuity_from(&older),
            AnchorContinuity::Incompatible(_)
        ));
        assert!(matches!(
            older.continuity_from(&restored),
            AnchorContinuity::Incompatible(_)
        ));
        let mut restarted = lab(1, 5)?;
        restarted.bridge_generation = 1;
        assert!(matches!(
            restarted.continuity_from(&older),
            AnchorContinuity::Incompatible(_)
        ));
        let mut forked = lab(1, 4)?;
        forked.semantic_world_root = Digest32::ZERO;
        assert!(matches!(
            forked.continuity_from(&older),
            AnchorContinuity::Incompatible(_)
        ));
        let mut other_policy = lab(1, 5)?;
        other_policy.policy_epoch = 2;
        assert!(matches!(
            other_policy.continuity_from(&older),
            AnchorContinuity::Incompatible(_)
        ));
        // Ordering follows canonical bytes.
        assert!(lab(1, 4)? < lab(1, 5)? && lab(1, 9)? < lab(2, 0)?);
        Ok(())
    }

    #[test]
    fn protocol_labels_are_bounded_and_closed() {
        let mut epochs = AnchorEpochs::laboratory(FortressId::new(1));
        epochs.bridge_protocol = "a b".to_owned();
        assert!(StateAnchorV2::from_v1(v1(0, 0), epochs.clone()).is_err());
        epochs.bridge_protocol = "x".repeat(33);
        assert!(StateAnchorV2::from_v1(v1(0, 0), epochs.clone()).is_err());
        epochs.bridge_protocol = String::new();
        assert!(StateAnchorV2::from_v1(v1(0, 0), epochs).is_err());
    }
}
