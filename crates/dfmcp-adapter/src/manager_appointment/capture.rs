use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub unit: u32,
    pub figure: u32,
}
impl Selection {
    pub fn new(unit: u32, figure: u32) -> Result<Self> {
        require(
            unit < i32::MAX as u32 && figure < i32::MAX as u32,
            "manager identity bound",
        )?;
        Ok(Self { unit, figure })
    }
    pub fn encode(self) -> Vec<u8> {
        let mut out = self.unit.to_be_bytes().to_vec();
        out.extend_from_slice(&self.figure.to_be_bytes());
        out
    }
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self> {
        Self::new(r.u32()?, r.u32()?)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assignment {
    pub id: i32,
    pub position: i32,
    pub figure: i32,
    pub last_figure: i32,
    pub cache: i32,
    pub squad: i32,
    pub site: i32,
    pub abstract_building: i32,
    pub vassal_entity: i32,
    pub vassal_assignment: i32,
    pub temp: i32,
    pub flags: Vec<u8>,
    pub claims: Vec<i32>,
}
impl Assignment {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let mut a = Self {
            id: i32(r)?,
            position: i32(r)?,
            figure: i32(r)?,
            last_figure: i32(r)?,
            cache: i32(r)?,
            squad: i32(r)?,
            site: i32(r)?,
            abstract_building: i32(r)?,
            vassal_entity: i32(r)?,
            vassal_assignment: i32(r)?,
            temp: i32(r)?,
            flags: read_field(r, 16)?.to_vec(),
            claims: vec![],
        };
        let n = u16(r)?;
        require(
            n <= 32 && a.id >= 0 && a.position >= 0,
            "manager assignment bound",
        )?;
        for _ in 0..n {
            a.claims.push(i32(r)?);
        }
        Ok(a)
    }
    fn append(&self, out: &mut Vec<u8>) {
        put(
            out,
            [
                self.id,
                self.position,
                self.figure,
                self.last_figure,
                self.cache,
                self.squad,
                self.site,
                self.abstract_building,
                self.vassal_entity,
                self.vassal_assignment,
                self.temp,
            ],
        );
        field(out, &self.flags);
        out.extend_from_slice(&(self.claims.len() as u16).to_be_bytes());
        put(out, self.claims.iter().copied());
    }
    pub fn ordinary_active(&self) -> bool {
        self.flags.first().is_some_and(|v| v & 0x0f == 1)
            && self.squad == -1
            && self.site == -1
            && self.abstract_building == -1
            && self.vassal_entity == -1
            && self.vassal_assignment == -1
            && self.temp == 0
            && self.claims.is_empty()
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entity {
    pub id: i32,
    pub index: i32,
    pub next_position: i32,
    pub next_assignment: i32,
    pub positions_digest: Digest32,
    pub assignments: Vec<Assignment>,
    pub role_buckets: Vec<Vec<i32>>,
}
impl Entity {
    fn read(r: &mut Reader<'_>, protocol: ManagerProtocol) -> Result<Self> {
        let mut e = Self {
            id: i32(r)?,
            index: i32(r)?,
            next_position: i32(r)?,
            next_assignment: i32(r)?,
            positions_digest: Digest32::from_bytes(r.array()?),
            assignments: vec![],
            role_buckets: vec![],
        };
        let n = u16(r)?;
        require(
            n <= 256 && e.id >= 0 && e.index >= 0 && e.next_position >= 0 && e.next_assignment >= 0,
            "manager entity bound",
        )?;
        for _ in 0..n {
            let a = Assignment::read(r)?;
            require(
                e.assignments.last().is_none_or(|p| p.id < a.id),
                "manager assignment ordering",
            )?;
            e.assignments.push(a);
        }
        if protocol == ManagerProtocol::V1_23 {
            let count = u16(r)?;
            require(count <= MAX_ROLE_BUCKETS, "manager role bucket bound")?;
            let mut total = 0;
            for _ in 0..count {
                let n = u16(r)?;
                total += n;
                require(
                    n <= MAX_ROLE_ENTRIES && total <= MAX_TOTAL_ROLE_ENTRIES,
                    "manager role entry bound",
                )?;
                let mut bucket = Vec::with_capacity(n);
                for _ in 0..n {
                    let id = i32(r)?;
                    require(
                        !bucket.contains(&id) && e.assignments.iter().any(|a| a.id == id),
                        "manager role entry is duplicate or foreign",
                    )?;
                    bucket.push(id);
                }
                e.role_buckets.push(bucket);
            }
        }
        Ok(e)
    }
    fn append(&self, out: &mut Vec<u8>, protocol: ManagerProtocol) {
        put(
            out,
            [
                self.id,
                self.index,
                self.next_position,
                self.next_assignment,
            ],
        );
        out.extend_from_slice(self.positions_digest.as_bytes());
        out.extend_from_slice(&(self.assignments.len() as u16).to_be_bytes());
        for a in &self.assignments {
            a.append(out);
        }
        if protocol == ManagerProtocol::V1_23 {
            out.extend_from_slice(&(self.role_buckets.len() as u16).to_be_bytes());
            for bucket in &self.role_buckets {
                out.extend_from_slice(&(bucket.len() as u16).to_be_bytes());
                put(out, bucket.iter().copied());
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub kind: i32,
    pub entity: i32,
    pub entity_cache: i32,
    pub strength: i32,
    pub assignment: i32,
    pub assignment_cache: i32,
    pub start_year: i32,
}
impl Link {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let l = Self {
            kind: i32(r)?,
            entity: i32(r)?,
            entity_cache: i32(r)?,
            strength: i32(r)?,
            assignment: i32(r)?,
            assignment_cache: i32(r)?,
            start_year: i32(r)?,
        };
        require(
            matches!(l.kind, 0 | 10)
                && l.entity >= 0
                && (0..=i16::MAX as i32).contains(&l.strength)
                && (l.kind == 10
                    || (l.assignment == -1 && l.assignment_cache == -1 && l.start_year == -1)),
            "manager HF link",
        )?;
        Ok(l)
    }
    fn append(&self, out: &mut Vec<u8>) {
        put(
            out,
            [
                self.kind,
                self.entity,
                self.entity_cache,
                self.strength,
                self.assignment,
                self.assignment_cache,
                self.start_year,
            ],
        );
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub id: i32,
    pub year: i32,
    pub seconds: i32,
    pub kind: i32,
    pub flags: Vec<u8>,
    pub position: bool,
    pub added: bool,
    pub entity: i32,
    pub figure: i32,
    pub position_id: i32,
    pub appointer: i32,
    pub promise: i32,
}
impl Event {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let e = Self {
            id: i32(r)?,
            year: i32(r)?,
            seconds: i32(r)?,
            kind: i32(r)?,
            flags: read_field(r, 16)?.to_vec(),
            position: r.boolean()?,
            added: r.boolean()?,
            entity: i32(r)?,
            figure: i32(r)?,
            position_id: i32(r)?,
            appointer: i32(r)?,
            promise: i32(r)?,
        };
        require(e.id >= 0, "manager event ID")?;
        Ok(e)
    }
    pub(crate) fn append(&self, out: &mut Vec<u8>) {
        put(out, [self.id, self.year, self.seconds, self.kind]);
        field(out, &self.flags);
        out.extend([u8::from(self.position), u8::from(self.added)]);
        put(
            out,
            [
                self.entity,
                self.figure,
                self.position_id,
                self.appointer,
                self.promise,
            ],
        );
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    pub protocol: ManagerProtocol,
    pub(super) raw: Vec<u8>,
    pub generation: u64,
    pub sequence: u64,
    pub tick: u64,
    pub year: u32,
    pub year_tick: u32,
    pub source: FortressIdentity,
    pub dimensions: [u32; 3],
    pub selection: Selection,
    pub group: Entity,
    pub civ: Entity,
    pub manager_position: i32,
    pub manager_index: i32,
    pub lookup_unit: i32,
    pub manager_links: u32,
    pub next_event: u32,
    pub history_count: u32,
    pub history_digest: Digest32,
    pub roster_digest: Digest32,
    pub entity_domain_digest: Digest32,
    pub last_event: Option<Event>,
    pub links: Vec<Link>,
    pub paused: bool,
    pub active: bool,
    pub citizen: bool,
    pub adult: bool,
    pub sane: bool,
    pub identity: bool,
    pub member: bool,
    pub role_eligible: bool,
    pub dual_getters: bool,
    pub expedition_leader: bool,
    pub ui_quiescent: bool,
}
impl Capture {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= MAX_CAPTURE_BYTES, "manager capture bound")?;
        let mut r = Reader(bytes);
        let protocol = ManagerProtocol::decode_magic(r.take(8)?, b'C')?;
        let generation = r.u64()?;
        let sequence = r.u64()?;
        let tick = r.u64()?;
        let site = r.u32()?;
        let year = r.u32()?;
        let year_tick = r.u32()?;
        let dimensions = [r.u32()?, r.u32()?, r.u32()?];
        let source = FortressIdentity::new(&text(&mut r, 512)?, site)?;
        let selection = Selection::read(&mut r)?;
        let group = Entity::read(&mut r, protocol)?;
        let civ = Entity::read(&mut r, protocol)?;
        require(
            group
                .role_buckets
                .iter()
                .chain(&civ.role_buckets)
                .map(Vec::len)
                .sum::<usize>()
                <= MAX_TOTAL_ROLE_ENTRIES,
            "manager total role entry bound",
        )?;
        require(
            protocol != ManagerProtocol::V1_23
                || group.role_buckets.len() == civ.role_buckets.len(),
            "manager role domains differ",
        )?;
        let manager_position = i32(&mut r)?;
        let manager_index = i32(&mut r)?;
        let lookup_unit = i32(&mut r)?;
        let manager_links = r.u32()?;
        let next_event = r.u32()?;
        let history_count = r.u32()?;
        let history_digest = Digest32::from_bytes(r.array()?);
        let roster_digest = Digest32::from_bytes(r.array()?);
        let entity_domain_digest = Digest32::from_bytes(r.array()?);
        let last_event = if r.boolean()? {
            Some(Event::read(&mut r)?)
        } else {
            None
        };
        let n = u16(&mut r)?;
        require(n <= 256, "manager link bound")?;
        let mut links = Vec::with_capacity(n);
        for _ in 0..n {
            links.push(Link::read(&mut r)?);
        }
        let out = Self {
            protocol,
            raw: bytes.to_vec(),
            generation,
            sequence,
            tick,
            source,
            year,
            year_tick,
            dimensions,
            selection,
            group,
            civ,
            manager_position,
            manager_index,
            lookup_unit,
            manager_links,
            next_event,
            history_count,
            history_digest,
            roster_digest,
            entity_domain_digest,
            last_event,
            links,
            paused: r.boolean()?,
            active: r.boolean()?,
            citizen: r.boolean()?,
            adult: r.boolean()?,
            sane: r.boolean()?,
            identity: r.boolean()?,
            member: r.boolean()?,
            role_eligible: r.boolean()?,
            dual_getters: r.boolean()?,
            expedition_leader: r.boolean()?,
            ui_quiescent: r.boolean()?,
        };
        r.finish()?;
        require(
            generation > 0
                && generation < u64::MAX
                && sequence < u64::MAX
                && year_tick < 403200
                && tick == u64::from(year) * 403200 + u64::from(year_tick)
                && site < i32::MAX as u32
                && dimensions.iter().all(|v| (1..=32768).contains(v))
                && manager_links <= 1
                && next_event < i32::MAX as u32
                && history_count <= 65536,
            "manager source/clock/count",
        )?;
        require(out.encode() == bytes, "noncanonical manager capture")?;
        Ok(out)
    }
    pub(super) fn encode(&self) -> Vec<u8> {
        let mut out = self.protocol.magic(b'C').to_vec();
        for n in [self.generation, self.sequence, self.tick] {
            out.extend_from_slice(&n.to_be_bytes());
        }
        for n in [self.source.site(), self.year, self.year_tick]
            .into_iter()
            .chain(self.dimensions)
        {
            out.extend_from_slice(&n.to_be_bytes());
        }
        field(&mut out, self.source.folder().as_bytes());
        out.extend(self.selection.encode());
        self.group.append(&mut out, self.protocol);
        self.civ.append(&mut out, self.protocol);
        put(
            &mut out,
            [self.manager_position, self.manager_index, self.lookup_unit],
        );
        for n in [self.manager_links, self.next_event, self.history_count] {
            out.extend_from_slice(&n.to_be_bytes());
        }
        for d in [
            self.history_digest,
            self.roster_digest,
            self.entity_domain_digest,
        ] {
            out.extend_from_slice(d.as_bytes());
        }
        out.push(u8::from(self.last_event.is_some()));
        if let Some(e) = &self.last_event {
            e.append(&mut out);
        }
        out.extend_from_slice(&(self.links.len() as u16).to_be_bytes());
        for l in &self.links {
            l.append(&mut out);
        }
        out.extend(
            [
                self.paused,
                self.active,
                self.citizen,
                self.adult,
                self.sane,
                self.identity,
                self.member,
                self.role_eligible,
                self.dual_getters,
                self.expedition_leader,
                self.ui_quiescent,
            ]
            .map(u8::from),
        );
        out
    }
    pub fn bytes(&self) -> &[u8] {
        &self.raw
    }
    pub fn witness(&self) -> Digest32 {
        Digest32::of_bytes(&self.raw)
    }
    pub fn target(&self) -> Option<(usize, &Assignment)> {
        let mut candidates = self
            .group
            .assignments
            .iter()
            .enumerate()
            .filter(|(_, a)| a.position == self.manager_position);
        let first = candidates.next()?;
        if candidates.next().is_some() {
            None
        } else {
            Some(first)
        }
    }
    pub fn eligible(&self) -> bool {
        let caches_vacant = match self.protocol {
            ManagerProtocol::V1_22 => {
                self.group.role_buckets.is_empty() && self.civ.role_buckets.is_empty()
            }
            ManagerProtocol::V1_23 => {
                self.group
                    .role_buckets
                    .get(MANAGE_PRODUCTION)
                    .is_some_and(Vec::is_empty)
                    && self
                        .civ
                        .role_buckets
                        .get(MANAGE_PRODUCTION)
                        .is_some_and(Vec::is_empty)
            }
        };
        caches_vacant
            && self.paused
            && self.active
            && self.citizen
            && self.adult
            && self.sane
            && self.identity
            && self.member
            && self.role_eligible
            && self.expedition_leader
            && self.ui_quiescent
            && self.dual_getters
            && self.lookup_unit == -1
            && self.manager_links == 0
            && self.group.id != self.civ.id
            && self.target().is_some_and(|(_, a)| {
                a.figure == -1 && a.last_figure == -1 && a.cache == -1 && a.ordinary_active()
            })
            && self.manager_position >= 0
            && self.manager_index >= 0
            && self.next_event + 1 < i32::MAX as u32
            && self.history_count < 65536
            && self.sequence < u64::MAX - 1
    }
    pub fn expected_after(&self) -> Result<Self> {
        require(
            self.eligible() && self.encode() == self.raw,
            "manager offer ineligible or changed",
        )?;
        let (index, a) = self.target().ok_or_else(|| {
            error(
                dfmcp_core::ErrorCode::AdapterRejected,
                "manager target missing",
            )
        })?;
        let mut out = self.clone();
        out.sequence += 1;
        out.group.assignments[index].figure = self.selection.figure as i32;
        out.group.assignments[index].last_figure = self.selection.figure as i32;
        out.group.assignments[index].cache = self.manager_index;
        if self.protocol == ManagerProtocol::V1_23 {
            out.group.role_buckets[MANAGE_PRODUCTION].push(a.id);
        }
        out.links.push(Link {
            kind: 10,
            entity: self.group.id,
            entity_cache: self.group.index,
            strength: 100,
            assignment: a.id,
            assignment_cache: index as i32,
            start_year: self.year as i32,
        });
        let e = Event {
            id: self.next_event as i32,
            year: self.year as i32,
            seconds: self.year_tick as i32,
            kind: 4,
            flags: vec![0],
            position: true,
            added: true,
            entity: self.group.id,
            figure: self.selection.figure as i32,
            position_id: self.manager_position,
            appointer: -1,
            promise: -1,
        };
        let mut history = self.history_digest.as_bytes().to_vec();
        e.append(&mut history);
        out.history_digest = hash(b"dfmcp-manager-history-projection/1", &history);
        out.last_event = Some(e);
        out.history_count += 1;
        out.next_event += 1;
        out.manager_links = 1;
        out.lookup_unit = self.selection.unit as i32;
        out.raw = out.encode();
        Self::decode(&out.raw)
    }
}
