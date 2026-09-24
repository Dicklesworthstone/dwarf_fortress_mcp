//! Quality-aware allocation over the same filtered coherent workforce model.
use super::*;
use dfmcp_world::workforce_allocation as weighted;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QualityPlan {
    pub plan: WorkforcePlan,
    pub priorities: Vec<u16>,
    pub quality: weighted::Allocation,
}

/// Priorities are keyed by demand identity, never input position. Missing means
/// zero. Unknown keys are refused rather than silently changing the objective.
/// Analysis, canonicalization, solving and verification share one work allowance.
pub fn plan(
    state: &LiveSpatialCitizenState,
    context: &OperationContext,
    requested: &[WorkforceDemand],
    priorities: &BTreeMap<String, u16>,
    maximum_work: u64,
) -> Result<QualityPlan> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let normalized = normalize(requested)?;
    if priorities.len() > MAX_WORKFORCE_DEMANDS
        || priorities.iter().any(|(key, priority)| {
            *priority > weighted::MAX_PRIORITY || !normalized.iter().any(|d| &d.key == key)
        })
    {
        return Err(invalid(
            "workforce priority must name a requested demand and lie in 0..1000",
        ));
    }
    let mut work = Work::new(context, maximum_work)?;
    let analysis = analyze_inner(state, context, &normalized, &mut work)?;
    let demands: Vec<_> = analysis
        .demands
        .iter()
        .map(|d| weighted::Demand {
            key: d.key.clone(),
            workers: d.workers,
            priority: priorities.get(&d.key).copied().unwrap_or(0),
        })
        .collect();
    let mut candidates = Vec::new();
    for (demand_index, rows) in analysis.candidates.iter().enumerate() {
        for row in rows {
            work.charge(1)?;
            candidates.push(weighted::Candidate {
                worker_id: row.entity_id,
                demand_index,
                effective: row.effective,
                nominal: row.nominal,
                steps: row.steps,
            });
        }
    }
    work.charge((candidates.len() as u64).saturating_mul(32))?;
    candidates.sort_by_key(|c| (c.demand_index, c.worker_id));
    let quality =
        weighted::allocate(&demands, &candidates, work.remaining()).map_err(|e| match e {
            weighted::Error::InvalidInput | weighted::Error::InvalidCertificate => {
                invariant("workforce quality model or certificate disagrees")
            }
            _ => exhausted("workforce quality allocation exceeded bounded work or arithmetic"),
        })?;
    work.charge(quality.work_units)?;
    work.charge((quality.assignments.len() + quality.witness.potentials.len()) as u64)?;
    let allocation = Allocation {
        assignments: quality
            .assignments
            .iter()
            .map(|a| flow::Assignment {
                supply_id: a.worker_id,
                demand_index: a.demand_index,
                units: 1,
            })
            .collect(),
        allocated_by_demand: quality
            .allocated_by_demand
            .iter()
            .map(|&n| u64::from(n))
            .collect(),
        requested_units: u64::from(quality.requested_workers),
        allocated_units: u64::from(quality.assigned_workers),
        cut_capacity: u64::from(quality.cut_capacity),
        work_units: quality.work_units,
        shortage: quality.shortage.as_ref().map(|s| flow::Shortage {
            demand_indices: s.demand_indices.clone(),
            required_units: u64::from(s.required_workers),
            eligible_units: u64::from(s.eligible_workers),
            deficit: u64::from(s.deficit),
        }),
    };
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    work.charge(0)?;
    Ok(QualityPlan {
        plan: WorkforcePlan {
            analysis,
            allocation,
            work_units: work.used,
        },
        priorities: demands.iter().map(|d| d.priority).collect(),
        quality,
    })
}
