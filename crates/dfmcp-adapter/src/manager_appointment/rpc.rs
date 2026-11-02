//! One foreground manager/1.22 connection. No caller chooses methods or profiles.
use super::*;
use crate::build_placement::rpc::{
    BuildCancellation,
    codec::{Message, bytes, number},
    link::Link,
};
use dfmcp_core::{Capability, MapCoord, MapCuboid, OperationContext, RiskTier};
use std::net::SocketAddr;
use std::time::Duration;
pub const METHODS: [&str; 6] = [
    "Handshake",
    "ReadManager",
    "PrepareManager",
    "CommitManager",
    "QueryManager",
    "CancelManager",
];
pub const CONNECT_BYTES: u64 = 8 * 1024 * 1024;
pub type Permission = Box<dyn Fn(bool) -> Result<()> + Send>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub protocol: ManagerProtocol,
    pub endpoint: SocketAddr,
    pub source: FortressIdentity,
    pub generation: u64,
    pub dimensions: [u32; 3],
    pub scope: MapCuboid,
    pub selection: Selection,
    pub df_version: String,
    pub dfhack_version: String,
}
impl Binding {
    pub fn effect_scope(&self) -> Result<MapCuboid> {
        self.read_scope()
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
        Selection::new(self.selection.unit, self.selection.figure)?;
        require(
            self.endpoint.is_ipv4()
                && self.endpoint.ip().is_loopback()
                && self.endpoint.port() != 0
                && self.generation > 0
                && self.generation < u64::MAX
                && self.dimensions.iter().all(|v| (1..=32768).contains(v))
                && self.scope.min == MapCoord::new(0, 0, 0)
                && self.scope.max
                    == MapCoord::new(
                        (self.dimensions[0] - 1) as i32,
                        (self.dimensions[1] - 1) as i32,
                        (self.dimensions[2] - 1) as i32,
                    )
                && self.scope.min.x >= 0
                && self.scope.min.y >= 0
                && self.scope.min.z >= 0
                && self.scope.max.x < self.dimensions[0] as i32
                && self.scope.max.y < self.dimensions[1] as i32
                && self.scope.max.z < self.dimensions[2] as i32
                && [&self.df_version, &self.dfhack_version]
                    .iter()
                    .all(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control)),
            "invalid manager source binding",
        )
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.protocol.magic(b'B').to_vec();
        field(&mut out, self.endpoint.to_string().as_bytes());
        out.extend_from_slice(&self.generation.to_be_bytes());
        out.extend_from_slice(&self.source.site().to_be_bytes());
        field(&mut out, self.source.folder().as_bytes());
        for n in self.dimensions {
            out.extend_from_slice(&n.to_be_bytes());
        }
        out.extend(self.selection.encode());
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
            c.protocol == self.protocol
                && c.source == self.source
                && c.dimensions == self.dimensions
                && c.generation == self.generation
                && c.selection == self.selection,
            "manager capture source differs",
        )
    }
}
fn authorize(c: &OperationContext, b: &Binding, write: Option<&Plan>) -> Result<()> {
    c.budget.validate()?;
    require(
        c.budget.max_wall_millis <= 60000
            && c.budget.max_wall_millis > 0
            && c.budget.max_bytes >= CONNECT_BYTES,
        "manager connection budget",
    )?;
    require(
        c.anchor.fortress_id == b.source.fortress_id(),
        "manager authority source differs",
    )?;
    let area = Some(b.read_scope()?);
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], area)?;
    c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], area)?;
    if let Some(plan) = write {
        b.capture(&plan.before)?;
        require(
            plan.before.selection == b.selection,
            "unoffered manager candidate",
        )?;
        let effect = b.read_scope()?;
        require(
            b.scope.contains_cuboid(effect),
            "manager effect exceeds host scope",
        )?;
        c.authorize(Capability::Plan, RiskTier::Irreversible, &[], Some(effect))?;
        c.authorize(
            Capability::ConfigureProduction,
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
pub struct ManagerRpc {
    link: Link,
    binding: Binding,
    secret: Vec<u8>,
    nonce: [u8; 32],
    methods: [i16; 6],
    permission: Permission,
    prepared: Option<Plan>,
    prepare_attempted: bool,
    fenced: bool,
    pub unresolved: bool,
    retained: u64,
    high_tick: u64,
}
impl ManagerRpc {
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
            "invalid manager credential/nonce",
        )?;
        let check = || {
            permission(false)?;
            authorize(c, &binding, None)
        };
        let mut link = Link::connect_manager(
            binding.endpoint,
            Duration::from_millis(c.budget.max_wall_millis),
            c.budget.max_bytes,
            cancellation,
            &check,
        )?;
        link.greeting(&check)?;
        let mut methods = [0; 6];
        for (i, name) in METHODS.iter().enumerate() {
            let mut request = Vec::new();
            bytes(&mut request, 1, name.as_bytes());
            let (request_type, reply_type, plugin) = match binding.protocol {
                ManagerProtocol::V1_22 => (
                    b"dfmcp.manager.v1_22.Request".as_slice(),
                    b"dfmcp.manager.v1_22.Reply".as_slice(),
                    b"dfmcp_manager_v1_22".as_slice(),
                ),
                ManagerProtocol::V1_23 => (
                    b"dfmcp.manager.v1_23.Request".as_slice(),
                    b"dfmcp.manager.v1_23.Reply".as_slice(),
                    b"dfmcp_manager_v1_23".as_slice(),
                ),
            };
            bytes(&mut request, 2, request_type);
            bytes(&mut request, 3, reply_type);
            bytes(&mut request, 4, plugin);
            let raw = link.frame_bounded(0, &request, 64, &check)?;
            let reply = Message::parse(&raw, 1)?;
            reply.exact(&[1])?;
            let n = reply.number(1)?;
            require(
                (2..=32767).contains(&n) && !methods[..i].contains(&(n as i16)),
                "aliased manager method binding",
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
            prepared: None,
            prepare_attempted: false,
            fenced: false,
            unresolved: false,
            retained: 0,
            high_tick: c.anchor.tick.get(),
        };
        out.call(0, None, None, c)?;
        Ok(out)
    }
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub fn fence(&mut self) {
        self.prepared = None;
        self.fenced = true;
        self.link.fence();
    }
    fn call(
        &mut self,
        index: usize,
        selection: Option<Selection>,
        plan: Option<&Plan>,
        c: &OperationContext,
    ) -> Result<Reply> {
        let result = (|| {
            require(!self.fenced, "manager connection fenced")?;
            // A plan or an earlier observation cannot borrow a caller's stale
            // tick to renew grants. This applies to read/recovery edges too.
            let mut current = c.clone();
            current.anchor.tick = dfmcp_core::GameTick(
                c.anchor
                    .tick
                    .get()
                    .max(self.high_tick)
                    .max(plan.map_or(0, |p| p.before.tick)),
            );
            self.high_tick = current.anchor.tick.get();
            self.link.narrow(
                Duration::from_millis(current.budget.max_wall_millis),
                current.budget.max_bytes,
            )?;
            let write = matches!(index, 2 | 3);
            let check = || {
                (self.permission)(write)?;
                authorize(&current, &self.binding, if write { plan } else { None })
            };
            let mut request = Vec::new();
            bytes(&mut request, 1, &self.secret);
            bytes(&mut request, 2, &self.nonce);
            number(&mut request, 3, 1);
            number(&mut request, 4, self.binding.protocol.minor());
            number(&mut request, 5, current.budget.max_wall_millis);
            if let Some(s) = selection {
                number(&mut request, 6, u64::from(s.unit));
                number(&mut request, 7, u64::from(s.figure));
            }
            if let Some(p) = plan {
                bytes(&mut request, 9, p.key().as_bytes());
                bytes(&mut request, 11, p.digest.as_bytes());
                if index == 2 {
                    bytes(&mut request, 10, p.before.witness().as_bytes());
                }
                if matches!(index, 3 | 5) {
                    bytes(&mut request, 12, &p.token);
                }
            }
            let raw = self
                .link
                .frame_bounded(self.methods[index], &request, 256 * 1024, &check)?;
            let reply = Message::parse_manager(&raw, 13)?;
            require(
                reply.bytes(3)? == self.nonce
                    && reply.number(4)? == 1
                    && reply.number(5)? == self.binding.protocol.minor(),
                "manager reply profile/nonce",
            )?;
            if !reply.boolean(1)? {
                reply.exact(&[1, 2, 3, 4, 5, 6, 7, 8])?;
                require(
                    (1..=8).contains(&reply.number(2)?)
                        && reply.number(6)? == 0
                        && reply.bytes(7)?.is_empty()
                        && reply.bytes(8)?.is_empty(),
                    "malformed manager refusal",
                )?;
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "native manager request refused; outcome not inferred",
                ));
            }
            let mut fields = vec![1, 2, 3, 4, 5, 6, 7, 8, 12, 13];
            if index == 1 {
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
                "manager native software/incarnation differs",
            )?;
            let retained = reply.number(13)?;
            let unresolved = reply.boolean(12)?;
            require(
                retained <= 64 && retained >= self.retained && (retained > 0 || !unresolved),
                "manager retention regressed",
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
    pub fn observe(&mut self, selection: Selection, c: &OperationContext) -> Result<Capture> {
        self.prepared = None;
        let result = (|| {
            let r = self.call(1, Some(selection), None, c)?;
            let capture =
                Capture::decode(r.observation.as_deref().ok_or_else(|| {
                    error(ErrorCode::AdapterRejected, "missing manager capture")
                })?)?;
            self.binding.capture(&capture)?;
            require(capture.selection == selection, "manager candidate differs")?;
            self.high_tick = self.high_tick.max(capture.tick);
            let mut actual = c.clone();
            actual.anchor.tick = dfmcp_core::GameTick(self.high_tick.max(c.anchor.tick.get()));
            authorize(&actual, &self.binding, None)
                .map_err(|e| e.with_detail("consumed_manager_tick", capture.tick.to_string()))?;
            Ok(capture)
        })();
        if result.is_err() {
            self.fence();
        }
        result
    }
    pub fn prepare(&mut self, p: &Plan, c: &OperationContext) -> Result<Record> {
        require(
            !self.prepare_attempted,
            "manager connection already prepared",
        )?;
        self.prepare_attempted = true;
        self.prepared = None;
        let r = self.call(2, Some(p.before.selection), Some(p), c)?;
        let record = Record::decode(
            r.record
                .as_deref()
                .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing manager preparation"))?,
            p,
        )?;
        require(
            !r.replayed && record.phase == 0 && !self.unresolved,
            "manager replay cannot grant dispatch",
        )?;
        self.prepared = Some(p.clone());
        Ok(record)
    }
    /// Check the held preparation before the journal records dispatch. This
    /// performs no RPC and never renews the original connection deadline or permit.
    /// Commit repeats these checks at the actual send; later failure is uncertain.
    pub fn preflight_commit(&mut self, p: &Plan, c: &OperationContext) -> Result<()> {
        let result = (|| {
            require(!self.fenced, "manager connection fenced")?;
            require(
                self.prepared.as_ref() == Some(p),
                "manager commit lacks exact same-connection preparation",
            )?;
            let mut current = c.clone();
            current.anchor.tick =
                dfmcp_core::GameTick(c.anchor.tick.get().max(self.high_tick).max(p.before.tick));
            self.high_tick = current.anchor.tick.get();
            (self.permission)(true)?;
            authorize(&current, &self.binding, Some(p))?;
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
            "manager commit lacks exact same-connection preparation",
        )?;
        let r = self.call(3, None, Some(p), c)?;
        let record = Record::decode(
            r.record
                .as_deref()
                .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing manager result"))?,
            p,
        )?;
        require(record.phase != 0, "manager commit returned prepared")?;
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
                .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing manager retirement"))?,
            p,
        )
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "rpc_tests.rs"]
mod rpc_tests;
