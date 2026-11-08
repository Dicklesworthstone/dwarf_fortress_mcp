//! One foreground tree/1.21 connection. No caller chooses methods or profiles.
use super::*;
use crate::build_placement::rpc::{
    BuildCancellation,
    codec::{Message, bytes, number},
    link::Link,
};
use dfmcp_core::{Capability, MapCoord, MapCuboid, OperationContext, RiskTier};
use std::net::SocketAddr;
use std::time::Duration;
pub const METHODS: [&str; 7] = [
    "Handshake",
    "ReadTreeHarvest",
    "PrepareTreeHarvest",
    "CommitTreeHarvest",
    "QueryTreeHarvest",
    "CancelTreeHarvest",
    "ReadTreeHarvestProgress",
];
#[cfg(all(test, target_os = "linux"))]
#[path = "rpc_tests.rs"]
mod rpc_tests;
pub const CONNECT_BYTES: u64 = 8 * 1024 * 1024;
pub type Permission = Box<dyn Fn(bool) -> Result<()> + Send>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub endpoint: SocketAddr,
    pub source: FortressIdentity,
    pub generation: u64,
    pub dimensions: [u32; 3],
    pub scope: MapCuboid,
    pub df_version: String,
    pub dfhack_version: String,
}
impl Binding {
    pub fn effect_scope(&self) -> Result<MapCuboid> {
        self.validate()?;
        Ok(self.scope)
    }
    pub fn read_scope(&self) -> Result<MapCuboid> {
        self.validate()?;
        MapCuboid::new(
            MapCoord::new(0, 0, 0),
            MapCoord::new(
                (self.dimensions[0] - 1) as i32,
                (self.dimensions[1] - 1) as i32,
                (self.dimensions[2] - 1) as i32,
            ),
        )
    }
    pub fn validate(&self) -> Result<()> {
        MapCuboid::new(self.scope.min, self.scope.max)?;
        require(
            self.endpoint.is_ipv4()
                && self.endpoint.ip().is_loopback()
                && self.endpoint.port() != 0
                && self.generation > 0
                && self.generation < u64::MAX
                && self.dimensions.iter().all(|v| (1..=32768).contains(v))
                && self.scope.min.x >= 0
                && self.scope.min.y >= 0
                && self.scope.min.z >= 0
                && self.scope.max.x < self.dimensions[0] as i32
                && self.scope.max.y < self.dimensions[1] as i32
                && self.scope.max.z < self.dimensions[2] as i32
                && [&self.df_version, &self.dfhack_version]
                    .iter()
                    .all(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control)),
            "invalid tree source binding",
        )
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut out = b"DFMTB021".to_vec();
        field(&mut out, self.endpoint.to_string().as_bytes());
        out.extend_from_slice(&self.generation.to_be_bytes());
        out.extend_from_slice(&self.source.site().to_be_bytes());
        field(&mut out, self.source.folder().as_bytes());
        put_coord(&mut out, self.dimensions);
        field(&mut out, self.df_version.as_bytes());
        field(&mut out, self.dfhack_version.as_bytes());
        for n in [
            self.scope.min.x,
            self.scope.min.y,
            self.scope.min.z,
            self.scope.max.x,
            self.scope.max.y,
            self.scope.max.z,
        ] {
            out.extend_from_slice(&n.to_be_bytes());
        }
        out
    }
    pub fn capture(&self, c: &Capture) -> Result<()> {
        self.validate()?;
        require(
            c.source == self.source
                && c.dimensions == self.dimensions
                && c.generation == self.generation,
            "tree capture source differs",
        )
    }
}
fn authorize(c: &OperationContext, b: &Binding, write: Option<&Plan>) -> Result<()> {
    c.budget.validate()?;
    require(
        c.budget.max_wall_millis <= 60000
            && c.budget.max_wall_millis > 0
            && c.budget.max_bytes >= CONNECT_BYTES,
        "tree connection budget",
    )?;
    require(
        c.anchor.fortress_id == b.source.fortress_id(),
        "tree authority source differs",
    )?;
    let area = Some(b.read_scope()?);
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], area)?;
    c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], area)?;
    if let Some(plan) = write {
        let effect = plan.before.effect_scope()?;
        require(
            b.scope.contains_cuboid(effect),
            "tree effect exceeds host scope",
        )?;
        c.authorize(Capability::Plan, RiskTier::Irreversible, &[], Some(effect))?;
        c.authorize(
            Capability::Designate,
            RiskTier::Irreversible,
            &[],
            Some(effect),
        )?;
    }
    Ok(())
}
struct Reply {
    observation: Option<Vec<u8>>,
    record: Option<Vec<u8>>,
    replayed: bool,
}
pub struct TreeRpc {
    link: Link,
    binding: Binding,
    secret: Vec<u8>,
    nonce: [u8; 32],
    methods: [i16; 7],
    permission: Permission,
    pub consumed_progress: Option<Progress>,
    progress_tick_floor: u64,
    prepared: Option<Plan>,
    prepare_attempted: bool,
    fenced: bool,
    pub unresolved: bool,
    retained: u64,
}
impl TreeRpc {
    pub fn connect(
        binding: Binding,
        secret: Vec<u8>,
        nonce: [u8; 32],
        c: &OperationContext,
        cancellation: BuildCancellation,
        permission: Permission,
    ) -> Result<Self> {
        binding.validate()?;
        require(
            (32..=256).contains(&secret.len()) && nonce != [0; 32],
            "invalid tree credential/nonce",
        )?;
        let check = || {
            permission(false)?;
            authorize(c, &binding, None)
        };
        let mut link = Link::connect_tree_harvest(
            binding.endpoint,
            Duration::from_millis(c.budget.max_wall_millis),
            c.budget.max_bytes,
            cancellation,
            &check,
        )?;
        link.greeting(&check)?;
        let mut methods = [0; 7];
        for (i, name) in METHODS.iter().enumerate() {
            let mut request = Vec::new();
            bytes(&mut request, 1, name.as_bytes());
            bytes(&mut request, 2, b"dfmcp.tree_harvest.v1_21.Request");
            bytes(&mut request, 3, b"dfmcp.tree_harvest.v1_21.Reply");
            bytes(&mut request, 4, b"dfmcp_tree_harvest_v1_21");
            let raw = link.frame_bounded(0, &request, 64, &check)?;
            let reply = Message::parse(&raw, 1)?;
            reply.exact(&[1])?;
            let n = reply.number(1)?;
            require(
                (2..=32767).contains(&n) && !methods[..i].contains(&(n as i16)),
                "aliased tree method binding",
            )?;
            methods[i] = n as i16;
        }
        let mut out = Self {
            link,
            binding,
            secret,
            nonce,
            methods,
            permission,
            consumed_progress: None,
            progress_tick_floor: 0,
            prepared: None,
            prepare_attempted: false,
            fenced: false,
            unresolved: false,
            retained: 0,
        };
        out.call(0, None, None, c)?;
        Ok(out)
    }
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    /// Narrow fresh progress to an independently consumed clock frontier.
    /// This cannot grant authority, renew an original goal or lower the floor.
    pub fn require_progress_at_or_after(&mut self, tick: u64) {
        self.progress_tick_floor = self.progress_tick_floor.max(tick);
    }
    pub fn fence(&mut self) {
        self.prepared = None;
        self.fenced = true;
        self.link.fence();
    }
    fn call(
        &mut self,
        index: usize,
        root: Option<[u32; 3]>,
        plan: Option<&Plan>,
        c: &OperationContext,
    ) -> Result<Reply> {
        let result = (|| {
            require(!self.fenced, "tree connection fenced")?;
            self.link.narrow(
                Duration::from_millis(c.budget.max_wall_millis),
                c.budget.max_bytes,
            )?;
            let write = matches!(index, 2 | 3);
            let check = || {
                (self.permission)(write)?;
                authorize(c, &self.binding, if write { plan } else { None })
            };
            let mut request = Vec::new();
            bytes(&mut request, 1, &self.secret);
            bytes(&mut request, 2, &self.nonce);
            number(&mut request, 3, 1);
            number(&mut request, 4, 21);
            if let Some(root) = root {
                for (tag, n) in [6, 7, 8].into_iter().zip(root) {
                    number(&mut request, tag, u64::from(n));
                }
            }
            if let Some(p) = plan {
                if index != 6 {
                    bytes(&mut request, 9, p.key().as_bytes());
                    bytes(&mut request, 11, p.digest.as_bytes());
                }
                if index == 2 {
                    bytes(&mut request, 10, p.before.witness().as_bytes());
                }
                if matches!(index, 3 | 5) {
                    bytes(&mut request, 12, &p.token);
                }
                if index == 6 {
                    number(&mut request, 13, u64::from(p.before.next_item));
                    number(&mut request, 14, p.before.species as i64 as u64);
                }
            }
            let raw = self
                .link
                .frame_bounded(self.methods[index], &request, 512 * 1024, &check)?;
            let reply = Message::parse_tree_harvest(&raw, 13)?;
            require(
                reply.bytes(3)? == self.nonce && reply.number(4)? == 1 && reply.number(5)? == 21,
                "tree reply profile/nonce",
            )?;
            if !reply.boolean(1)? {
                reply.exact(&[1, 2, 3, 4, 5, 6, 7, 8])?;
                require(
                    (1..=8).contains(&reply.number(2)?)
                        && reply.number(6)? == 0
                        && reply.bytes(7)?.is_empty()
                        && reply.bytes(8)?.is_empty(),
                    "malformed tree refusal",
                )?;
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "native tree request refused; outcome not inferred",
                ));
            }
            let mut fields = vec![1, 2, 3, 4, 5, 6, 7, 8, 12, 13];
            if matches!(index, 1 | 6) {
                fields.push(9);
            }
            if matches!(index, 2 | 3 | 5) || (index == 4 && reply.has(10)) {
                fields.push(10);
            }
            if index == 2 {
                fields.push(11);
            }
            fields.sort_unstable();
            reply.exact(&fields)?;
            require(
                reply.number(2)? == 0
                    && reply.number(6)? == self.binding.generation
                    && reply.bytes(7)? == self.binding.df_version.as_bytes()
                    && reply.bytes(8)? == self.binding.dfhack_version.as_bytes(),
                "tree native software/incarnation differs",
            )?;
            let retained = reply.number(13)?;
            let unresolved = reply.boolean(12)?;
            require(
                retained <= 64 && retained >= self.retained && (retained > 0 || !unresolved),
                "tree retention regressed",
            )?;
            self.retained = retained;
            self.unresolved = unresolved;
            Ok(Reply {
                observation: if reply.has(9) {
                    Some(reply.bytes(9)?.to_vec())
                } else {
                    None
                },
                record: if reply.has(10) {
                    Some(reply.bytes(10)?.to_vec())
                } else {
                    None
                },
                replayed: reply.has(11) && reply.boolean(11)?,
            })
        })();
        if result.is_err() {
            self.fence();
        }
        result
    }
    pub fn observe(&mut self, root: [u32; 3], c: &OperationContext) -> Result<Capture> {
        self.prepared = None;
        let result = (|| {
            let r = self.call(1, Some(root), None, c)?;
            let capture = Capture::decode(
                r.observation
                    .as_deref()
                    .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing tree capture"))?,
            )?;
            self.binding.capture(&capture)?;
            require(capture.root == root, "tree root differs")?;
            let mut actual = c.clone();
            actual.anchor.tick = dfmcp_core::GameTick(capture.tick.max(c.anchor.tick.get()));
            authorize(&actual, &self.binding, None)
                .map_err(|e| e.with_detail("consumed_tree_tick", capture.tick.to_string()))?;
            Ok(capture)
        })();
        if result.is_err() {
            self.fence();
        }
        result
    }
    pub fn prepare(&mut self, p: &Plan, c: &OperationContext) -> Result<Record> {
        require(!self.prepare_attempted, "tree connection already prepared")?;
        self.prepare_attempted = true;
        self.prepared = None;
        let r = self.call(2, Some(p.before.root), Some(p), c)?;
        let record = Record::decode(
            r.record
                .as_deref()
                .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing tree preparation"))?,
            p,
        )?;
        require(
            !r.replayed && record.phase == 0 && !self.unresolved,
            "tree replay cannot grant dispatch",
        )?;
        self.prepared = Some(p.clone());
        Ok(record)
    }
    /// Check the held preparation before the journal records dispatch. This
    /// performs no RPC and never renews the original connection deadline or permit.
    /// Commit repeats these checks at the actual send; later failure is uncertain.
    pub fn preflight_commit(&mut self, p: &Plan, c: &OperationContext) -> Result<()> {
        let result = (|| {
            require(!self.fenced, "tree connection fenced")?;
            require(
                self.prepared.as_ref() == Some(p),
                "tree commit lacks exact same-connection preparation",
            )?;
            (self.permission)(true)?;
            authorize(c, &self.binding, Some(p))?;
            self.link.narrow(
                Duration::from_millis(c.budget.max_wall_millis),
                c.budget.max_bytes,
            )
        })();
        if result.is_err() {
            self.fence();
        }
        result
    }
    pub fn commit(
        &mut self,
        permit: super::journal::Dispatch<'_>,
        c: &OperationContext,
    ) -> Result<Record> {
        let p = permit.plan();
        require(
            self.prepared.take().as_ref() == Some(p),
            "tree commit lacks exact same-connection preparation",
        )?;
        let r = self.call(3, None, Some(p), c)?;
        let record = Record::decode(
            r.record
                .as_deref()
                .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing tree result"))?,
            p,
        )?;
        require(record.phase != 0, "tree commit returned prepared")?;
        Ok(record)
    }
    pub fn query(&mut self, p: &Plan, c: &OperationContext) -> Result<Option<Record>> {
        self.prepared = None;
        let r = self.call(4, None, Some(p), c)?;
        r.record
            .as_deref()
            .map(|v| Record::decode(v, p))
            .transpose()
    }
    pub fn cancel(&mut self, p: &Plan, c: &OperationContext) -> Result<Record> {
        self.prepared = None;
        let r = self.call(5, None, Some(p), c)?;
        Record::decode(
            r.record
                .as_deref()
                .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing tree retirement"))?,
            p,
        )
    }
    pub fn progress(&mut self, p: &Plan, c: &OperationContext) -> Result<Progress> {
        self.prepared = None;
        self.consumed_progress = None;
        let r = self.call(6, Some(p.before.root), Some(p), c)?;
        let progress = Progress::decode(
            r.observation
                .as_deref()
                .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing tree progress"))?,
            p,
        )?;
        require(
            progress.generation == self.binding.generation,
            "tree progress payload/envelope generation differs",
        )?;
        self.consumed_progress = Some(progress.clone());
        if progress.tick < self.progress_tick_floor {
            self.fence();
            return Err(error(
                ErrorCode::StaleAnchor,
                "tree progress predates the independently consumed clock floor",
            )
            .with_detail("consumed_tree_tick", progress.tick.to_string()));
        }
        self.progress_tick_floor = self.progress_tick_floor.max(progress.tick);
        let mut actual = c.clone();
        actual.anchor.tick = dfmcp_core::GameTick(progress.tick.max(c.anchor.tick.get()));
        if let Err(e) = authorize(&actual, &self.binding, None) {
            self.fence();
            return Err(e.with_detail("consumed_tree_tick", progress.tick.to_string()));
        }
        Ok(progress)
    }
}
