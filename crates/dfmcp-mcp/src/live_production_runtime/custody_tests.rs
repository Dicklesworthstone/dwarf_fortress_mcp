#![allow(dead_code, unused_imports)]
// Real private-file codecs and real citizen evidence owner; the citizen source
// is a deterministic fixture and does not claim live-game evidence.
include!("../semantic_workforce_runtime/fixtures.rs");
use super::{GuardedEvidence, SharedCustody};
use crate::live_production_runtime::request::Request;
use crate::semantic_workforce_runtime::identity::create_private_capsule;
use dfmcp_adapter::plan_archive::create_private_plan_archive;
use dfmcp_adapter::semantic_workforce::evidence_owner::{
    CitizenEvidenceLimits, CitizenEvidenceSource, CitizenWorkforceEvidenceOwner,
};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

struct Source {
    capsule: LiveObservationCapsule,
    corrupt: Rc<RefCell<Option<PathBuf>>>,
}
impl CitizenEvidenceSource for Source {
    fn bridge_manifest(&self) -> BridgeManifest {
        self.capsule.bridge.clone()
    }
    fn read_page(
        &mut self,
        offset: u32,
        maximum: u32,
        names: bool,
        _: &OperationContext,
    ) -> Result<ObservationPage> {
        if let Some(path) = self.corrupt.borrow_mut().take() {
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|e| invalid(e.to_string()))?;
            let mut byte = [0];
            file.read_exact(&mut byte)
                .map_err(|e| invalid(e.to_string()))?;
            byte[0] ^= 1;
            file.seek(SeekFrom::Start(0))
                .map_err(|e| invalid(e.to_string()))?;
            file.write_all(&byte).map_err(|e| invalid(e.to_string()))?;
        }
        let cap = &self.capsule;
        let end = (offset as usize + maximum as usize).min(cap.citizens.len());
        let mut citizens = cap.citizens[offset as usize..end].to_vec();
        if !names {
            for citizen in &mut citizens {
                citizen.name.clear();
            }
        }
        Ok(ObservationPage {
            bridge_generation: cap.bridge.bridge_generation,
            world_loaded: true,
            fortress_mode: true,
            paused: cap.paused,
            current_year: cap.current_year,
            current_year_tick: cap.current_year_tick,
            world_name: cap.world_name.clone(),
            world_folder: cap.world_folder.clone(),
            site_id: cap.site_id,
            citizen_count_total: cap.citizens.len() as u32,
            citizen_offset: offset,
            complete: end == cap.citizens.len(),
            citizens,
        })
    }
}

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn provisional_source_anchors_do_not_replace_canonical_custody_or_rewind_authority()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let (capsule, projection) = observation(&[42])?;
    let mut c = context(&projection.snapshot);
    c.budget.max_bytes = 512 * 1024 * 1024;
    c.budget.max_actions = 2;
    let request = Request {
        expected_observation: "11".repeat(32),
        units: vec!["43".into()],
        labor: "MINE".into(),
        enabled: true,
        recipe: "wooden_bed".into(),
        amount: 1,
        deadline_tick: 300,
        poll_interval_ticks: 5,
        stable_samples: 2,
        objective: None,
    };
    let evidence = PredicateEvidence::scoped(&projection.snapshot, policy(&projection.snapshot))?;
    let plan = request.compile(&evidence, &c)?;
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let directory = Directory(std::env::temp_dir().join(format!(
        "dfmcp-combined-provisional-custody-{}-{id}",
        std::process::id()
    )));
    std::fs::create_dir(&directory.0)?;
    std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o700))?;
    let plan_path = directory.0.join("original.plan");
    let identity_path = directory.0.join("original.citizens");
    let archive = create_private_plan_archive(
        &plan_path,
        &plan,
        dfmcp_adapter::live_production::archive_limits(),
        &c,
    )?;
    let identity = create_private_capsule(&identity_path, &capsule, &c)?;
    let before = (std::fs::read(&plan_path)?, std::fs::read(&identity_path)?);
    let custody = SharedCustody::new(archive, identity, plan);
    custody.arm(&c)?;

    // Independent labor and inventory projections intentionally have different
    // hashes at one proposed cursor. Between them factories still carry the
    // operation's earlier canonical anchor. These are immutable Query reads.
    let mut labor = c.clone();
    labor.anchor.cursor.sequence += 1;
    labor.anchor.tick = GameTick(125);
    labor.anchor.state_hash = Digest32::of_bytes(b"provisional labor projection");
    custody.verify(&labor)?;
    custody.verify(&c)?;
    let mut inventory = labor.clone();
    inventory.anchor.state_hash = Digest32::of_bytes(b"provisional inventory projection");
    custody.verify(&inventory)?;
    let mut later = inventory.clone();
    later.anchor.cursor.sequence += 1;
    later.anchor.tick = GameTick(130);
    later.anchor.state_hash = Digest32::of_bytes(b"later independent projection");
    custody.verify(&later)?;

    // The older canonical anchor cannot restore a grant that expired at the
    // consumed source time, including after the first refused check.
    let mut expired = c.clone();
    for grant in &mut expired.grants {
        if grant.capability == Capability::Query {
            grant.expires_at_tick = Some(GameTick(129));
        }
    }
    assert!(custody.verify(&expired).is_err());
    assert!(custody.verify(&expired).is_err());
    custody.verify(&c)?;
    for changed in ["fortress", "epoch", "tick", "canonical_hash", "request"] {
        let mut bad = c.clone();
        match changed {
            "fortress" => bad.anchor.fortress_id = dfmcp_core::FortressId::new(1),
            "epoch" => bad.anchor.cursor.epoch += 1,
            "tick" => bad.anchor.tick = GameTick(119),
            "canonical_hash" => {
                bad.anchor.state_hash = Digest32::of_bytes(b"replaced canonical state")
            }
            _ => bad.request_id = RequestId::new(99),
        }
        assert!(custody.verify(&bad).is_err(), "changed {changed}");
    }
    custody.disarm()?;
    let mut published = later;
    published.anchor.cursor.sequence += 1;
    published.anchor.state_hash = Digest32::of_bytes(b"published complete parent projection");
    custody.arm(&published)?;
    custody.verify(&published)?;
    assert!(
        custody.verify(&c).is_err(),
        "earlier operation cursor cannot re-enter a newer operation"
    );
    custody.disarm()?;
    assert_eq!(
        before,
        (std::fs::read(&plan_path)?, std::fs::read(&identity_path)?)
    );
    Ok(())
}

#[test]
fn real_custody_and_owner_refuse_corruption_after_refresh()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    for (corrupted, during_refresh) in [
        ("plan", true),
        ("identity", true),
        ("plan", false),
        ("identity", false),
    ] {
        let (capsule, projection) = observation(&[42])?;
        let mut c = context(&projection.snapshot);
        c.budget.max_bytes = 512 * 1024 * 1024;
        c.budget.max_actions = 2;
        c.grants.push(CapabilityGrant {
            capability: Capability::Observe,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        });
        let request = Request {
            expected_observation: "11".repeat(32),
            units: vec!["43".into()],
            labor: "MINE".into(),
            enabled: true,
            recipe: "wooden_bed".into(),
            amount: 1,
            deadline_tick: 300,
            poll_interval_ticks: 5,
            stable_samples: 2,
            objective: None,
        };
        let evidence =
            PredicateEvidence::scoped(&projection.snapshot, policy(&projection.snapshot))?;
        let plan = request.compile(&evidence, &c)?;
        dfmcp_adapter::live_production::validate_plan(&plan)?;
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let dir = Directory(std::env::temp_dir().join(format!(
            "dfmcp-combined-custody-{}-{id}",
            std::process::id()
        )));
        std::fs::create_dir(&dir.0)?;
        std::fs::set_permissions(&dir.0, std::fs::Permissions::from_mode(0o700))?;
        let plan_path = dir.0.join("original.plan");
        let identity_path = dir.0.join("original.citizens");
        let archive = create_private_plan_archive(
            &plan_path,
            &plan,
            dfmcp_adapter::live_production::archive_limits(),
            &c,
        )?;
        let identity = create_private_capsule(&identity_path, &capsule, &c)?;
        let custody = SharedCustody::new(archive, identity, plan);
        custody.arm(&c)?;
        let hook = Rc::new(RefCell::new(None));
        let factory_capsule = capsule.clone();
        let factory_hook = hook.clone();
        let factory = move |_: &OperationContext| {
            Ok(Source {
                capsule: factory_capsule.clone(),
                corrupt: factory_hook.clone(),
            })
        };
        let authorizer =
            |_: &LiveObservationCapsule, projection: &LiveWorldProjection, _: &OperationContext| {
                Ok(policy(&projection.snapshot))
            };
        let limits = CitizenEvidenceLimits {
            page_size: 8,
            max_citizens: 8,
        };
        let owner = CitizenWorkforceEvidenceOwner::new(capsule, limits, factory, authorizer, &c)?;
        let mut guarded = GuardedEvidence::new(owner, custody.clone());
        let capture = WorkforceFixture::default().capture()?;
        let mut native_context = c.clone();
        native_context.budget.max_bytes = 16 * 1024 * 1024;
        guarded.refresh_after_capture(&capture, &native_context)?;
        dfmcp_adapter::semantic_work_orders::WorkOrderEvidenceOwner::revalidate_after_external_read(
            &mut guarded, &native_context,
        )?;
        assert_eq!(guarded.routing_evidence()?.anchor(), c.anchor);
        custody.disarm()?;
        assert!(guarded.routing_evidence().is_err());
        custody.arm(&c)?;
        assert!(guarded.routing_evidence().is_err());
        assert!(dfmcp_adapter::semantic_work_orders::WorkOrderEvidenceOwner::revalidate_after_external_read(
            &mut guarded, &native_context,
        ).is_err());
        guarded.refresh_after_capture(&capture, &native_context)?;
        assert_eq!(guarded.routing_evidence()?.anchor(), c.anchor);
        custody.disarm()?;
        let mut limited = c.clone();
        limited.budget.max_bytes = 36 * 1024 * 1024;
        custody.arm(&limited)?;
        custody.verify(&c)?;
        assert!(custody.verify(&c).is_err());
        assert!(guarded.routing_evidence().is_err());
        custody.disarm()?;
        custody.arm(&c)?;
        guarded.refresh_after_capture(&capture, &native_context)?;
        assert_eq!(guarded.routing_evidence()?.anchor(), c.anchor);
        let corrupt_path = if corrupted == "plan" {
            plan_path
        } else {
            identity_path
        };
        if during_refresh {
            *hook.borrow_mut() = Some(corrupt_path);
            assert!(
                guarded
                    .refresh_after_capture(&capture, &native_context)
                    .is_err()
            );
        } else {
            // Model a private-file change during a separate workforce read,
            // after the canonical source refresh has published its valid route.
            let mut bytes = std::fs::read(&corrupt_path)?;
            bytes[0] ^= 1;
            std::fs::write(&corrupt_path, bytes)?;
            assert!(dfmcp_adapter::semantic_work_orders::WorkOrderEvidenceOwner::revalidate_after_external_read(
                &mut guarded, &native_context,
            ).is_err());
        }
        assert!(guarded.routing_evidence().is_err());
        assert!(custody.verify(&c).is_err());
        custody.disarm()?;
        assert!(guarded.routing_evidence().is_err());
    }
    Ok(())
}
