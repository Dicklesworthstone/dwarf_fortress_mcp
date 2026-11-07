//! Read-only import of one exact Tree original into workshop material selection.
//! Native source IDs, endpoints, paths and credentials remain operator inputs.
use super::{Config as WorkshopConfig, Work, area, denied, digest, exhausted, invalid, nonce, runtime_io};
use dfmcp_adapter::build_placement::journal::private_file::PrivateBuildFile;
use dfmcp_adapter::build_placement::rpc::BuildCancellation;
use dfmcp_adapter::tree_harvest::{
    journal::{Journal, TreeMaterialOrigin, open_private},
    rpc::{Binding, TreeRpc},
};
use dfmcp_adapter::workshop_construction::{
    FortressIdentity, WorkshopCapture, WorkshopMaterialDiscovery, WorkshopSelection,
    acknowledgement::WorkshopAcknowledgements,
    journal::{WorkshopMode, WorkshopPin},
};
use dfmcp_core::{Digest32, MapCoord, OperationContext, Result, cuboids_intersect};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

pub(super) const SCHEMA: &str = "dfmcp.workshop-tree-material-mcp-development/1";
pub(super) const OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_WORKSHOP_TREE_MATERIAL";
pub(super) const TOKEN: &str = "DFMCP_TREE_HARVEST_TOKEN";
const CUSTODY_BYTES: u64 = 20 * 1024 * 1024;
const PROJECTION_BYTES: u64 = 1024 * 1024;
const HANDLE_PREFIX: &str = "tree-workshop-material:";

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    directory: PathBuf,
    endpoint: String,
    generation: u64,
    root: [u32; 3],
    scope: [u32; 6],
    read_scope: [u32; 6],
    acknowledged: super::PinInput,
}

#[derive(Clone)]
pub(super) struct Config {
    directory: PathBuf,
    binding: Binding,
    root: [u32; 3],
    pin: WorkshopPin,
}

impl Config {
    pub(super) fn parse(input: Input, workshop: &super::Input) -> Result<Self> {
        let path = input
            .directory
            .to_str()
            .ok_or_else(|| invalid("Tree donor custody path must be UTF-8"))?;
        if !input.directory.is_absolute()
            || path.len() < 2
            || path.len() > 3800
            || path[1..]
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == ".." || s.chars().any(char::is_control))
            || input.directory == workshop.directory
            || (0..3).any(|i| input.root[i] >= workshop.map_dimensions[i])
        {
            return Err(invalid("Tree donor requires a distinct absolute custody directory and an in-map root"));
        }
        let endpoint = dfmcp_adapter::parse_loopback_endpoint(&input.endpoint)?;
        if !endpoint.is_ipv4() || endpoint.port() == 0 || endpoint.to_string() != input.endpoint {
            return Err(invalid("Tree donor requires a canonical IPv4 loopback endpoint"));
        }
        let binding = Binding {
            endpoint,
            source: FortressIdentity::new(&workshop.world_folder, workshop.site_id)?,
            generation: input.generation,
            dimensions: workshop.map_dimensions,
            scope: area(input.scope, workshop.map_dimensions)?,
            df_version: workshop.df_version.clone(),
            dfhack_version: workshop.dfhack_version.clone(),
        };
        binding.validate()?;
        if area(input.read_scope, workshop.map_dimensions)? != binding.read_scope()? {
            return Err(invalid("Tree material discovery requires explicit complete loaded-map read scope"));
        }
        let pin = input.acknowledged.decode()?;
        if pin.frames == 0 {
            return Err(invalid("Tree material donor requires an acknowledged original operation"));
        }
        Ok(Self {
            directory: input.directory,
            binding,
            root: input.root,
            pin,
        })
    }

    fn source(&self, workshop: &WorkshopConfig, c: &OperationContext) -> Result<TreeRpc> {
        workshop.current()?;
        let secret = std::env::var(TOKEN)
            .map_err(|_| denied("read-only Tree material credential unavailable"))?
            .into_bytes();
        let pinned = secret.clone();
        let workshop = workshop.clone();
        TreeRpc::connect(
            self.binding.clone(),
            secret,
            nonce(c)?,
            c,
            BuildCancellation::with_check(Arc::new(runtime_io)),
            Box::new(move |write| {
                workshop.current()?;
                if write {
                    return Err(denied("workshop material discovery never grants Tree mutation authority"));
                }
                if std::env::var(TOKEN).ok().map(String::into_bytes).as_ref() != Some(&pinned) {
                    return Err(denied("read-only Tree material credential changed"));
                }
                Ok(())
            }),
        )
    }
}

// This filters only the host's already configured spatial policy. Native
// material, floors, workers and connectivity still require independent 1.20 qualification.
fn host_candidate(
    workshop: &WorkshopConfig,
    selection: WorkshopSelection,
    position: [u32; 3],
) -> bool {
    let material = MapCoord::new(position[0] as i32, position[1] as i32, position[2] as i32);
    let [x1, y1, x2, y2, z] = selection.footprint();
    let Ok(footprint) = area([x1, y1, z, x2, y2, z], workshop.input.map_dimensions) else {
        return false;
    };
    workshop.scope.contains(material)
        && workshop.scope.contains_cuboid(footprint)
        && !selection.contains(position)
        && !workshop.protected.iter().any(|region| {
            region.contains(material) || cuboids_intersect(region, &footprint)
        })
}

pub(super) struct Owner {
    config: Config,
    journal: Journal<PrivateBuildFile>,
    acknowledgements: WorkshopAcknowledgements<PrivateBuildFile>,
    origin: TreeMaterialOrigin,
    discovery: Option<WorkshopMaterialDiscovery>,
    high_tick: u64,
    high_sequence: u64,
    high_item: u32,
    fenced: bool,
}

impl Owner {
    pub(super) fn open(config: Config, work: &mut Work) -> Result<Self> {
        let (mut journal, mut acknowledgements) = open_private(
            &config.directory,
            &work.reserve(CUSTODY_BYTES)?,
            WorkshopMode::Offline,
            &config.binding,
            config.root,
            false,
            Some(config.pin),
            [0; 32],
        )?;
        let origin = journal.material_origin(
            &mut acknowledgements,
            &work.reserve(CUSTODY_BYTES)?,
        )?;
        Ok(Self {
            config,
            journal,
            acknowledgements,
            high_tick: origin.high_tick(),
            high_sequence: origin.high_sequence(),
            high_item: origin.high_item(),
            origin,
            discovery: None,
            fenced: false,
        })
    }

    pub(super) fn high_tick(&self) -> u64 {
        self.high_tick
    }

    pub(super) fn verify(&mut self, work: &mut Work) -> Result<()> {
        work.advance_authorization_tick(self.high_tick);
        if self.fenced {
            return Err(denied("Tree material donor continuity was lost; close this session"));
        }
        let origin = self.journal.material_origin(
            &mut self.acknowledgements,
            &work.reserve(CUSTODY_BYTES)?,
        )?;
        if origin.pin() != self.origin.pin()
            || origin.binding() != self.origin.binding()
            || origin.original() != self.origin.original()
        {
            self.fenced = true;
            return Err(denied("the exact retained Tree material donor changed"));
        }
        Ok(())
    }

    pub(super) fn refresh(
        &mut self,
        workshop: &WorkshopConfig,
        work: &mut Work,
    ) -> Result<WorkshopSelection> {
        self.discovery = None;
        self.verify(work)?;
        let mut source = self.config.source(
            workshop,
            &work.reserve(dfmcp_adapter::tree_harvest::rpc::CONNECT_BYTES)?,
        )?;
        let result = source.progress(
            &self.origin.original().plan,
            &work.reserve(dfmcp_adapter::tree_harvest::rpc::CONNECT_BYTES)?,
        );
        // Retain every decoded source clock even when a later authority check
        // refuses publication. A fresh request cannot erase a consumed regression.
        if let Some(progress) = &source.consumed_progress {
            let regressed = progress.generation != self.origin.binding().generation
                || progress.tick < self.high_tick
                || progress.sequence < self.high_sequence
                || progress.next_item < self.high_item;
            self.high_tick = self.high_tick.max(progress.tick);
            self.high_sequence = self.high_sequence.max(progress.sequence);
            self.high_item = self.high_item.max(progress.next_item);
            work.advance_authorization_tick(self.high_tick);
            if regressed {
                self.fenced = true;
                return Err(denied("Tree material source clock, sequence or item horizon regressed"));
            }
        }
        let progress = result?;
        self.verify(work)?;
        let discovery = WorkshopMaterialDiscovery::new(
            &self.origin,
            progress,
            workshop.input.center,
            &work.reserve(PROJECTION_BYTES)?,
        )?;
        let first = discovery
            .offers()
            .iter()
            .find(|offer| host_candidate(workshop, offer.selection(), offer.position()))
            .or_else(|| discovery.offers().first())
            .ok_or_else(|| denied("fresh Tree material discovery found no eligible ground wood"))?
            .selection();
        self.discovery = Some(discovery);
        Ok(first)
    }

    pub(super) fn select(
        &mut self,
        workshop: &WorkshopConfig,
        expected: &str,
        material: &str,
        work: &mut Work,
    ) -> Result<(WorkshopSelection, Digest32)> {
        if material.len() != HANDLE_PREFIX.len() + 64 {
            return Err(invalid("choose an exact observed Tree material handle"));
        }
        let handle = digest(material.strip_prefix(HANDLE_PREFIX).ok_or_else(|| {
            invalid("choose an exact observed Tree material handle")
        })?)?;
        let observed = self.discovery.as_ref().ok_or_else(|| {
            denied("observe the original Tree material offers before selecting one")
        })?;
        if digest(expected)? != observed.witness() {
            return Err(denied("Tree material discovery witness differs from the observed offer"));
        }
        let selection = observed.resolve(handle)?;
        let previous = observed
            .offers()
            .iter()
            .find(|offer| offer.handle() == handle)
            .ok_or_else(exhausted)?;
        let identity = (previous.position(), previous.material(), previous.material_index());
        self.refresh(workshop, work)?;
        let fresh = self.discovery.as_ref().ok_or_else(exhausted)?;
        let selected = fresh
            .offers()
            .iter()
            .find(|offer| offer.selection() == selection)
            .ok_or_else(|| denied("selected Tree material is absent from the fresh ground-wood offers"))?;
        if (selected.position(), selected.material(), selected.material_index()) != identity {
            return Err(denied("selected Tree material identity changed after observation"));
        }
        Ok((selection, selected.handle()))
    }

    pub(super) fn qualify(
        &self,
        handle: Digest32,
        capture: &WorkshopCapture,
        work: &mut Work,
    ) -> Result<WorkshopSelection> {
        self.discovery
            .as_ref()
            .ok_or_else(|| denied("Tree material observation was withdrawn"))?
            .qualify(handle, capture, &work.reserve(PROJECTION_BYTES)?)
    }

    pub(super) fn check_capture_clock(&self, capture: &WorkshopCapture) -> Result<()> {
        if capture.tick() < self.high_tick {
            return Err(denied("workshop capture predates consumed Tree material evidence"));
        }
        Ok(())
    }

    pub(super) fn withdraw(&mut self) {
        self.discovery = None;
    }

    pub(super) fn view(&self, workshop: &WorkshopConfig) -> Value {
        let Some(discovery) = &self.discovery else {
            return Value::Null;
        };
        let progress = discovery.progress();
        json!({
            "witness":discovery.witness().to_hex(),
            "acknowledged":super::pin_json(self.origin.pin()),
            "original_key":self.origin.original().plan.key(),
            "original_plan_digest":self.origin.original().plan.digest.to_hex(),
            "source":{"profile":"tree_harvest/1.21","world_folder":progress.source.folder(),
                "site_id":progress.source.site(),"map_dimensions":progress.dimensions,
                "generation":progress.generation,"sequence":progress.sequence,"tick":progress.tick,
                "root":progress.root,"species":progress.species,"paused_at_anchor":progress.paused},
            "original_item_horizon":self.origin.original().plan.before.next_item,
            "current_item_horizon":progress.next_item,
            "harvest_goal_completion_proven":false,"causal_provenance_known":false,
            "workshop_qualification":"each selected offer requires an independent fresh workshop/1.20 capture",
            "canonical_progress_hex":super::hex(progress.bytes()),
            "offers":discovery.offers().iter().map(|offer|json!({
                "material_handle":format!("{HANDLE_PREFIX}{}",offer.handle().to_hex()),
                "item_id":offer.selection().item_id(),"position":offer.position(),
                "material":offer.material(),"material_index":offer.material_index(),
                "center":offer.selection().center(),
                "host_selection_allowed":host_candidate(workshop,offer.selection(),offer.position())
            })).collect::<Vec<_>>()
        })
    }
}

#[cfg(test)]
#[path = "tree_material_tests.rs"]
mod tests;
