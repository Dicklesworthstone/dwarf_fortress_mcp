//! Deferred Manager policy is not a donor. Only the parent-owned completed
//! handoff can populate the ordinary read-only production prerequisite.
use super::*;

impl ProjectProductionPolicy {
    pub(crate) fn matches_manager(
        &self,
        binding: &dfmcp_adapter::manager_appointment::rpc::Binding,
    ) -> Result<()> {
        binding.validate()?;
        if binding.protocol != dfmcp_adapter::manager_appointment::ManagerProtocol::V1_23
            || binding.source.folder() != self.workforce.world_folder
            || binding.source.folder() != self.orders.world_folder
            || binding.source.site() != self.workforce.site_id
            || binding.source.site() != self.orders.site_id
            || binding.df_version != self.workforce.df_version
            || binding.df_version != self.orders.df_version
            || binding.dfhack_version != self.workforce.dfhack_version
            || binding.dfhack_version != self.orders.dfhack_version
        {
            return Err(denied(
                "project Manager and production fortress or software pins differ",
            ));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HandoffConfiguration {
    manager_prerequisite: manager_config::Input,
    #[serde(deserialize_with = "manager_config::checkpoint")]
    manager_prerequisite_checkpoint: Option<ManagerPrerequisiteCheckpoint>,
}

pub(super) fn resolve(
    configured: Option<manager_config::Input>,
    mut checkpoint: Option<ManagerPrerequisiteCheckpoint>,
    recovery: Option<&RecoveryInput>,
    directory: &Path,
    handoff: Option<&ManagerProjectHandoff>,
    owns_manager: bool,
) -> Result<(Option<ManagerConfig>, Option<ManagerPrerequisiteCheckpoint>)> {
    if !owns_manager {
        if handoff.is_some() {
            return Err(denied(
                "a Manager handoff cannot add an unselected project stage",
            ));
        }
        return Ok((
            configured
                .map(|input| input.validate(directory))
                .transpose()?,
            checkpoint,
        ));
    }
    if configured.is_some() {
        return Err(denied(
            "project Manager donor must come from its verified original child handoff",
        ));
    }
    let Some(handoff) = handoff else {
        // This is configuration-only preflight before any original exists.
        // No fabricated ACK or placeholder original reaches an importer.
        if checkpoint.is_some()
            || recovery.is_some_and(|r| {
                r.manager_prerequisite_digest.is_some() || r.manager_prerequisite_floor.is_some()
            })
        {
            return Err(denied(
                "project Manager recovery requires its exact retained handoff",
            ));
        }
        return Ok((None, None));
    };
    let input: HandoffConfiguration = serde_json::from_value(handoff.production_configuration())
        .map_err(|_| invalid("invalid typed Manager handoff configuration"))?;
    let manager = input.manager_prerequisite.validate(directory)?;
    let seed = input
        .manager_prerequisite_checkpoint
        .ok_or_else(|| denied("Manager handoff lost its consumed source floor"))?;
    if seed.origin_witness != handoff.origin_witness() {
        return Err(denied(
            "Manager handoff checkpoint changed its original witness",
        ));
    }
    if let Some(recovery) = recovery {
        let retained = recovery
            .manager_prerequisite_floor
            .ok_or_else(|| denied("project recovery lost its Manager source floors"))?;
        require_floor(retained, seed.floor)?;
    } else if let Some(retained) = checkpoint {
        if retained.origin_witness != seed.origin_witness {
            return Err(denied(
                "project checkpoint belongs to another Manager original",
            ));
        }
        require_floor(retained.floor, seed.floor)?;
    } else {
        checkpoint = Some(seed);
    }
    Ok((Some(manager), checkpoint))
}

/// Every consumed source dimension is independent rejection custody. A later
/// tick cannot hide an older sequence, history vector or entity allocator.
fn require_floor(retained: ManagerRecoveryFloor, seed: ManagerRecoveryFloor) -> Result<()> {
    if retained.tick < seed.tick
        || retained.generation < seed.generation
        || retained.sequence < seed.sequence
        || retained.next_event < seed.next_event
        || retained.history_count < seed.history_count
        || retained.group_next_position < seed.group_next_position
        || retained.group_next_assignment < seed.group_next_assignment
        || retained.civ_next_position < seed.civ_next_position
        || retained.civ_next_assignment < seed.civ_next_assignment
        || (seed.fenced && !retained.fenced)
    {
        return Err(denied(
            "project recovery rolled back a consumed Manager source floor",
        ));
    }
    retained.validate(seed.generation, retained.tick)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_handoff_retains_every_source_floor_and_sticky_fence() -> Result<()> {
        let seed = ManagerRecoveryFloor {
            tick: 200,
            generation: 23,
            sequence: 31,
            next_event: 42,
            history_count: 45,
            group_next_position: 51,
            group_next_assignment: 61,
            civ_next_position: 71,
            civ_next_assignment: 81,
            fenced: false,
        };
        require_floor(seed, seed)?;
        for lower in [
            ManagerRecoveryFloor { tick: 199, ..seed },
            ManagerRecoveryFloor {
                generation: 22,
                ..seed
            },
            ManagerRecoveryFloor {
                sequence: 30,
                ..seed
            },
            ManagerRecoveryFloor {
                next_event: 41,
                ..seed
            },
            ManagerRecoveryFloor {
                history_count: 44,
                ..seed
            },
            ManagerRecoveryFloor {
                group_next_position: 50,
                ..seed
            },
            ManagerRecoveryFloor {
                group_next_assignment: 60,
                ..seed
            },
            ManagerRecoveryFloor {
                civ_next_position: 70,
                ..seed
            },
            ManagerRecoveryFloor {
                civ_next_assignment: 80,
                ..seed
            },
        ] {
            assert!(require_floor(lower, seed).is_err());
        }
        assert!(
            require_floor(
                ManagerRecoveryFloor {
                    generation: 24,
                    ..seed
                },
                seed
            )
            .is_err()
        );
        let fenced = ManagerRecoveryFloor {
            generation: 24,
            fenced: true,
            ..seed
        };
        require_floor(fenced, seed)?;
        assert!(
            require_floor(
                ManagerRecoveryFloor {
                    fenced: false,
                    ..fenced
                },
                fenced
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn deferred_manager_policy_cannot_claim_an_original_or_source_checkpoint() -> Result<()> {
        let result = resolve(
            None,
            None,
            None,
            Path::new("/private/production"),
            None,
            true,
        )?;
        assert!(result.0.is_none() && result.1.is_none());
        let raw = serde_json::json!({
            "origin_witness":Digest32::of_bytes(b"no invented origin").to_hex(),
            "tick":1,"generation":1,"sequence":1,"next_event":1,"history_count":1,
            "group_next_position":1,"group_next_assignment":1,
            "civ_next_position":1,"civ_next_assignment":1,"fenced":false
        });
        #[derive(Deserialize)]
        struct Checkpoint {
            #[serde(deserialize_with = "manager_config::checkpoint")]
            value: Option<ManagerPrerequisiteCheckpoint>,
        }
        let checkpoint: Checkpoint = serde_json::from_value(serde_json::json!({"value":raw}))
            .map_err(|_| invalid("test checkpoint decoding failed"))?;
        assert!(
            resolve(
                None,
                checkpoint.value,
                None,
                Path::new("/private/production"),
                None,
                true
            )
            .is_err()
        );
        Ok(())
    }
}
