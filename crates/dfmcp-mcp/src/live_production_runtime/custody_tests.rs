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
fn real_custody_and_owner_refuse_corruption_after_refresh()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    for corrupted in ["plan", "identity"] {
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
        assert_eq!(guarded.routing_evidence()?.anchor(), c.anchor);
        custody.disarm()?;
        assert!(guarded.routing_evidence().is_err());
        custody.arm(&c)?;
        assert!(guarded.routing_evidence().is_err());
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
        *hook.borrow_mut() = Some(if corrupted == "plan" {
            plan_path
        } else {
            identity_path
        });
        assert!(
            guarded
                .refresh_after_capture(&capture, &native_context)
                .is_err()
        );
        assert!(guarded.routing_evidence().is_err());
        assert!(custody.verify(&c).is_err());
        custody.disarm()?;
        assert!(guarded.routing_evidence().is_err());
    }
    Ok(())
}
