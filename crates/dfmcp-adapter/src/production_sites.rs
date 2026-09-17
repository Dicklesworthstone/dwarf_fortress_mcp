//! Per-demand spatial eligibility with ONE capacity per physical stack. Local
//! partial allocations are deliberately ignored; all candidate supplies survive
//! into the global task selector. Only routes from the same capture are joined.
use super::*;

pub const MULTISITE_POLICY: &str = "joint-complete-tasks-location-bound-shared-capacity/1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionSites {
    /// Aligned to canonical task order, not input order.
    pub task_origins: Vec<[u32; 3]>,
    /// Aligned to inventory.demands, including reserves at the default origin.
    pub material_origins: Vec<[u32; 3]>,
    /// The default site's report remains ProductionPortfolio.inventory.
    pub additional_inventory: BTreeMap<[u32; 3], SpatialInventory>,
    /// Union of site-eligible candidates. Repeated IDs keep capacity, not sums.
    pub supplies: Vec<flow::Supply>,
}
impl ProductionSites {
    pub fn multiple_origins(&self, default: [u32; 3]) -> bool {
        self.task_origins.iter().any(|origin| *origin != default)
    }
}

pub(super) fn normalize(default: [u32; 3], tasks: &[ProductionTask],
    overrides: &BTreeMap<String, [u32; 3]>) -> Result<Vec<[u32; 3]>> {
    if overrides.len() > selection::MAX_TASKS || default.iter().any(|n| *n >= 32768) {
        return Err(invalid("invalid production default origin or site count"));
    }
    for (key, origin) in overrides {
        if !tasks.iter().any(|task| &task.key == key) || origin.iter().any(|n| *n >= 32768) {
            return Err(invalid("production site override names an absent task or invalid coordinate"));
        }
    }
    Ok(tasks.iter().map(|task| overrides.get(&task.key).copied().unwrap_or(default)).collect())
}

fn site_mask(origins: &[[u32; 3]], site: [u32; 3]) -> u32 {
    origins.iter().enumerate().fold(0u32, |mask, (index, origin)| {
        if *origin == site { mask | (1u32 << index) } else { mask }
    })
}

/// Intersect each local eligibility mask with the demands actually located at
/// that site. An item visible from several sites is still the SAME finite stack.
fn merge_supply(pool: &mut BTreeMap<u64, flow::Supply>, supply: &flow::Supply,
    mask: u32) -> Result<()> {
    let eligible = supply.eligible & mask;
    if eligible == 0 { return Ok(()); }
    match pool.get_mut(&supply.id) {
        Some(prior) => {
            if prior.units != supply.units {
                return Err(invariant("same-capture site reports disagree about stack capacity"));
            }
            prior.eligible |= eligible;
        }
        None => {
            if pool.len() >= flow::MAX_SUPPLIES {
                return Err(exhausted("combined production sites exceed the shared stack bound"));
            }
            pool.insert(supply.id, flow::Supply { id: supply.id, units: supply.units, eligible });
        }
    }
    Ok(())
}

pub(super) fn analyze(state: &LiveSpatialCitizenState, context: &OperationContext,
    default: &SpatialInventory, task_origins: Vec<[u32; 3]>, material_owners: &[usize],
    work: &mut Work) -> Result<ProductionSites> {
    if default.demands.len() != material_owners.len() || default.demands.len() > flow::MAX_DEMANDS {
        return Err(invariant("site material ownership is inconsistent"));
    }
    let material_origins = material_owners.iter().map(|owner| {
        if *owner == selection::RESERVE_OWNER { Ok(default.origin) }
        else { task_origins.get(*owner).copied().ok_or_else(|| invariant("material site owner missing")) }
    }).collect::<Result<Vec<_>>>()?;
    let distinct: BTreeSet<_> = task_origins.iter().copied().collect();
    let mut additional_inventory = BTreeMap::new();
    for site in distinct {
        if site == default.origin { continue; }
        let timed = remaining_context(context, work)?;
        let report = inventory::plan(state, &timed, site, &default.demands, work.remaining())?;
        work.charge(report.work_units)?;
        if report.anchor != default.anchor || report.source_digest != default.source_digest
            || report.demands != default.demands || report.origin != site {
            return Err(invariant("production sites do not share one source and material model"));
        }
        additional_inventory.insert(site, report);
    }
    let mut pool = BTreeMap::new();
    for report in std::iter::once(default).chain(additional_inventory.values()) {
        let mask = site_mask(&material_origins, report.origin);
        work.charge(material_origins.len() as u64)?;
        for supply in &report.supplies {
            work.charge(1)?;
            merge_supply(&mut pool, supply, mask)?;
        }
    }
    work.charge(0)?;
    Ok(ProductionSites { task_origins, material_origins, additional_inventory,
        supplies: pool.into_values().collect() })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_stack_capacity_is_not_multiplied_by_site_count() -> Result<()> {
        let mut pool = BTreeMap::new();
        let supply = flow::Supply { id: 9, units: 3, eligible: 0b111 };
        merge_supply(&mut pool, &supply, 0b001)?;
        merge_supply(&mut pool, &supply, 0b110)?;
        merge_supply(&mut pool, &supply, 0b001)?;
        assert_eq!(pool[&9], supply);
        assert!(merge_supply(&mut pool, &flow::Supply { units: 4, ..supply }, 1).is_err());
        Ok(())
    }
    #[test]
    fn local_visibility_does_not_grant_eligibility_at_another_site() -> Result<()> {
        let origins = [[0,0,5], [4,0,5], [0,0,5]];
        let mut pool = BTreeMap::new();
        merge_supply(&mut pool, &flow::Supply { id: 9, units: 5, eligible: 0b111 }, site_mask(&origins,[0,0,5]))?;
        merge_supply(&mut pool, &flow::Supply { id: 10, units: 2, eligible: 0b011 }, site_mask(&origins,[4,0,5]))?;
        assert_eq!(pool[&9].eligible, 0b101);
        assert_eq!(pool[&10].eligible, 0b010);
        Ok(())
    }
}
