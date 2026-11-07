//! Real private Tree custody and unchanged native-vector codecs exercise the
//! material handoff. These are component fixtures, not native game evidence.
use super::*;
use crate::build_placement::journal::private_file::PrivateBuildFile;
use crate::tree_harvest::{
    self as tree,
    journal::{self as tree_journal, Mode},
};
use crate::workshop_construction::acknowledgement::{
    WorkshopAcknowledgements, open_private_acknowledgements,
};
use crate::workshop_construction::journal::WorkshopPin;
use std::io::Write;

struct Fixture {
    journal: tree_journal::Journal<PrivateBuildFile>,
    acknowledgement: WorkshopAcknowledgements<PrivateBuildFile>,
    context: OperationContext,
    binding: tree::rpc::Binding,
    directory: tree_journal::journal_tests::Directory,
}

fn open_fixture(phase: &str, sample: Option<Progress>, pending: bool) -> Result<Fixture> {
    let directory = tree_journal::journal_tests::Directory::new()?;
    let context = tree_journal::journal_tests::context()?;
    let binding = tree_journal::journal_tests::binding(
        "127.0.0.1:5000".parse().map_err(|_| invalid())?,
    )?;
    let (mut journal, mut acknowledgement) = tree_journal::open_private(
        &directory.0, &context, Mode::Control, &binding, [15, 15, 2], true, None, [9; 32],
    )?;
    let original = tree_journal::Original::new(
        tree::tests::plan()?, tree::tests::capture()?.tick + 12000,
    )?;
    journal.intent(original.clone(), &context, &mut acknowledgement)?;
    journal.prepared(tree::tests::record("prepared")?, &context, &mut acknowledgement)?;
    if phase != "prepared" {
        journal.dispatch(
            original.seal(), &context, &mut acknowledgement, &mut |_| Ok(()),
            |_, _| tree::tests::record(phase),
        )?;
    }
    if let Some(sample) = sample {
        journal.begin_sample(&context, &mut acknowledgement)?;
        journal.sample(sample, &context, &mut acknowledgement)?;
    }
    if pending {
        journal.begin_sample(&context, &mut acknowledgement)?;
    }
    let pin = journal.pin();
    drop(journal);
    drop(acknowledgement);
    let (journal, acknowledgement) = tree_journal::open_private(
        &directory.0, &context, Mode::Offline, &binding, [15, 15, 2], false, Some(pin), [0; 32],
    )?;
    Ok(Fixture { journal, acknowledgement, context, binding, directory })
}

fn origin(fixture: &mut Fixture) -> Result<TreeMaterialOrigin> {
    fixture.journal.material_origin(&mut fixture.acknowledgement, &fixture.context)
}

fn supply() -> Result<Progress> {
    let cap = tree::tests::capture()?;
    tree::tests::progress(cap.tick + 10, cap.generation)
}

fn canonical(progress: &Progress) -> Result<Progress> {
    let mut raw = b"DFMTP021".to_vec();
    for value in [progress.generation, progress.sequence, progress.tick] {
        raw.extend_from_slice(&value.to_be_bytes());
    }
    raw.extend_from_slice(&progress.source.site().to_be_bytes());
    for value in progress.dimensions {
        raw.extend_from_slice(&value.to_be_bytes());
    }
    tree::field(&mut raw, progress.source.folder().as_bytes());
    for value in progress.root {
        raw.extend_from_slice(&value.to_be_bytes());
    }
    raw.push(u8::from(progress.paused));
    raw.push(progress.root_predicate);
    raw.extend_from_slice(&progress.species.to_be_bytes());
    raw.extend_from_slice(&progress.next_item.to_be_bytes());
    raw.extend_from_slice(&progress.nearby_felling_jobs.to_be_bytes());
    raw.extend_from_slice(&(progress.wood.len() as u16).to_be_bytes());
    for wood in &progress.wood {
        raw.extend_from_slice(&wood.id.to_be_bytes());
        for value in wood.position {
            raw.extend_from_slice(&value.to_be_bytes());
        }
        raw.extend_from_slice(&wood.material.to_be_bytes());
        raw.extend_from_slice(&wood.material_index.to_be_bytes());
        raw.extend_from_slice(&wood.flags.to_be_bytes());
    }
    Progress::decode(&raw, &tree::tests::plan()?)
}

fn discovery(fixture: &mut Fixture) -> Result<WorkshopMaterialDiscovery> {
    WorkshopMaterialDiscovery::new(&origin(fixture)?, supply()?, [25, 25, 2], &fixture.context)
}

fn workshop(discovery: &WorkshopMaterialDiscovery, index: usize) -> Result<WorkshopCapture> {
    let offer = &discovery.offers()[index];
    let mut capture = crate::workshop_construction::tests::capture()?;
    capture.tick = discovery.progress().tick;
    capture.selection = offer.selection();
    let WorkshopItem::Visible(item) = &mut capture.item else {
        return Err(invalid());
    };
    item.position = offer.position();
    item.material = offer.material();
    item.material_index = offer.material_index();
    WorkshopCapture::decode(&capture.encode())
}

fn custody_bytes(fixture: &Fixture) -> Result<(Vec<u8>, Vec<u8>)> {
    let read = |name: &str| {
        std::fs::read(fixture.directory.0.join(name)).map_err(|_| invalid())
    };
    Ok((read("tree-harvest.journal")?, read("tree-harvest.ack")?))
}

#[test]
fn tree_material_discovery_uses_readonly_original_custody_and_fresh_workshop_material() -> Result<()> {
    let mut fixture = open_fixture("designated", None, false)?;
    let before = custody_bytes(&fixture)?;
    let pin = fixture.journal.pin();
    let origin = origin(&mut fixture)?;
    assert_eq!(origin.pin(), pin);
    assert_eq!(origin.original().plan, tree::tests::plan()?);
    assert_eq!(origin.binding(), &fixture.binding);
    assert_eq!(fixture.journal.samples, 0);
    assert_eq!(fixture.journal.achieved_tick, None);
    let observed = WorkshopMaterialDiscovery::new(
        &origin, supply()?, [25, 25, 2], &fixture.context,
    )?;
    assert_eq!(observed.tree_pin(), pin);
    assert_eq!(observed.offers().len(), 2);
    assert_eq!(
        observed.offers().iter().map(|offer| offer.selection().item_id()).collect::<Vec<_>>(),
        [1000, 1001],
    );
    for index in 0..2 {
        let offer = &observed.offers()[index];
        let capture = workshop(&observed, index)?;
        assert_eq!(observed.resolve(offer.handle())?, capture.selection());
        assert_eq!(observed.qualify(offer.handle(), &capture, &fixture.context)?, capture.selection());
        let plan = WorkshopPlan::new("one-original-workshop", capture)?;
        let restored = WorkshopPlan::decode(&plan.canonical_bytes())?;
        assert_eq!(restored, plan);
        assert_eq!(restored.before().selection().item_id(), 1000 + index as u32);
    }
    assert_eq!(custody_bytes(&fixture)?, before);
    assert_eq!(fixture.journal.pin(), pin);
    assert_eq!(fixture.journal.achieved_tick, None);
    assert!(!fixture.journal.dispatchable());
    Ok(())
}

#[test]
fn tree_material_discovery_does_not_renew_or_claim_original_tree_goal() -> Result<()> {
    let mut fixture = open_fixture("designated", None, false)?;
    let origin = origin(&mut fixture)?;
    let deadline = origin.original().deadline_tick;
    let before = custody_bytes(&fixture)?;
    let mut now = supply()?;
    now.tick = deadline + 1;
    let observed = WorkshopMaterialDiscovery::new(
        &origin, canonical(&now)?, [25, 25, 2], &fixture.context,
    )?;
    assert_eq!(observed.progress().tick, deadline + 1);
    assert!(!observed.offers().is_empty());
    assert_eq!(origin.original().deadline_tick, deadline);
    assert_eq!(fixture.journal.achieved_tick, None);
    assert_eq!(fixture.journal.samples, 0);
    assert_eq!(custody_bytes(&fixture)?, before);
    Ok(())
}

#[test]
fn tree_material_origin_refuses_unresolved_pending_or_broken_original_lineage() -> Result<()> {
    for phase in ["prepared", "indeterminate", "refused", "cancelled"] {
        let mut fixture = open_fixture(phase, None, false)?;
        let before = custody_bytes(&fixture)?;
        assert!(origin(&mut fixture).is_err(), "{phase}");
        assert_eq!(custody_bytes(&fixture)?, before);
    }
    let mut pending = open_fixture("designated", None, true)?;
    pending.journal.sample_pending = false;
    assert!(origin(&mut pending).is_err());
    let mut changed = supply()?;
    changed.generation += 1;
    let mut changed = open_fixture("designated", Some(canonical(&changed)?), false)?;
    changed.journal.continuity_lost = false;
    changed.journal.high_generation = changed.binding.generation;
    assert!(origin(&mut changed).is_err());

    // A refused same-generation rollback is not appended as a harmless lower
    // floor. Its already durable sample-start marker remains pending on reopen.
    let mut sampled = supply()?;
    sampled.sequence = 8;
    let Fixture { journal, acknowledgement, context, binding, directory } =
        open_fixture("designated", Some(canonical(&sampled)?), false)?;
    let pin = journal.pin();
    drop(journal);
    drop(acknowledgement);
    let (mut journal, mut acknowledgement) = tree_journal::open_private(
        &directory.0, &context, Mode::Recover, &binding, [15, 15, 2], false, Some(pin), [0; 32],
    )?;
    journal.begin_sample(&context, &mut acknowledgement)?;
    let mut regressed = sampled;
    regressed.tick -= 1;
    assert!(journal.sample(canonical(&regressed)?, &context, &mut acknowledgement).is_err());
    let pin = journal.pin();
    drop(journal);
    drop(acknowledgement);
    let (mut journal, mut acknowledgement) = tree_journal::open_private(
        &directory.0, &context, Mode::Offline, &binding, [15, 15, 2], false, Some(pin), [0; 32],
    )?;
    assert!(journal.material_origin(&mut acknowledgement, &context).is_err());
    Ok(())
}

#[test]
fn tree_material_origin_reconstructs_immutable_bytes_instead_of_mutable_view_fields() -> Result<()> {
    let mut sampled = supply()?;
    sampled.sequence = 8;
    let mut fixture = open_fixture("designated", Some(canonical(&sampled)?), false)?;
    let expected = origin(&mut fixture)?;
    fixture.journal.original = None;
    fixture.journal.native = None;
    fixture.journal.latest = None;
    fixture.journal.state = tree_journal::State::Prepared;
    fixture.journal.high_tick = 0;
    fixture.journal.high_sequence = 0;
    fixture.journal.high_item = 0;
    let restored = origin(&mut fixture)?;
    assert_eq!(restored, expected);
    assert_eq!(restored.high_tick(), sampled.tick);
    assert_eq!(restored.high_sequence(), 8);
    assert_eq!(restored.high_item(), sampled.next_item);

    let mut unresolved = open_fixture("indeterminate", None, false)?;
    unresolved.journal.native = Some(tree::tests::record("designated")?);
    unresolved.journal.state = tree_journal::State::Terminal;
    unresolved.journal.effect_uncertain = false;
    assert!(origin(&mut unresolved).is_err());
    Ok(())
}

#[test]
fn tree_material_origin_requires_its_independent_tree_ack_and_fences_substitution() -> Result<()> {
    for name in ["tree-harvest.journal", "tree-harvest.ack"] {
        let mut fixture = open_fixture("designated", None, false)?;
        let path = fixture.directory.0.join(name);
        let before = std::fs::read(&path).map_err(|_| invalid())?;
        let mut changed = before.clone();
        let last = changed.last_mut().ok_or_else(invalid)?;
        *last ^= 1;
        std::fs::OpenOptions::new().write(true).open(&path)
            .and_then(|mut file| file.write_all(&changed)).map_err(|_| invalid())?;
        assert_eq!(origin(&mut fixture).err().ok_or_else(invalid)?.code, ErrorCode::CorruptLedger);
        std::fs::OpenOptions::new().write(true).open(&path)
            .and_then(|mut file| file.write_all(&before)).map_err(|_| invalid())?;
        assert!(origin(&mut fixture).is_err(), "restoring bytes cannot revive a fenced importer");
    }

    let mut fixture = open_fixture("designated", None, false)?;
    let directory = tree_journal::journal_tests::Directory::new()?;
    let mut context = fixture.context.clone();
    for grant in &mut context.grants { grant.scope.map_area = None; }
    let mut wrong_profile = open_private_acknowledgements(
        &directory.0.join("workshop.ack"), &context, Mode::Control, true, None,
    )?;
    let pin = fixture.journal.pin();
    wrong_profile.acknowledge(WorkshopPin {
        journal_id: pin.journal_id, head: pin.journal_id, frames: 0,
    }, &context)?;
    wrong_profile.acknowledge(pin, &context)?;
    assert_eq!(wrong_profile.latest(), Some(pin));
    assert!(fixture.journal.material_origin(&mut wrong_profile, &context).is_err());
    Ok(())
}

#[test]
fn tree_material_discovery_preserves_all_consumed_floors_and_canonical_progress() -> Result<()> {
    let mut sampled = supply()?;
    sampled.sequence = 8;
    let mut fixture = open_fixture("designated", Some(canonical(&sampled)?), false)?;
    let origin = origin(&mut fixture)?;
    for case in 0..7 {
        let mut progress = sampled.clone();
        match case {
            0 => progress.tick -= 1,
            1 => progress.sequence -= 1,
            2 => {
                progress.next_item -= 1;
                progress.wood.retain(|wood| wood.id < progress.next_item);
            }
            3 => progress.generation += 1,
            4 => progress.paused = false,
            5 => progress.root_predicate = 1,
            _ => progress.nearby_felling_jobs = 1,
        }
        assert!(WorkshopMaterialDiscovery::new(
            &origin, canonical(&progress)?, [25, 25, 2], &fixture.context,
        ).is_err(), "case {case}");
    }
    let mut forged = canonical(&sampled)?;
    forged.wood[0].material_index += 1;
    assert!(WorkshopMaterialDiscovery::new(
        &origin, forged, [25, 25, 2], &fixture.context,
    ).is_err());
    Ok(())
}

#[test]
fn tree_material_handles_bind_complete_discovery_session_and_host_footprint() -> Result<()> {
    let mut fixture = open_fixture("designated", None, false)?;
    let origin = origin(&mut fixture)?;
    let observed = discovery(&mut fixture)?;
    let original_handle = observed.offers()[0].handle();
    assert_ne!(original_handle, observed.offers()[1].handle());
    assert!(observed.resolve(Digest32::ZERO).is_err());
    let same = WorkshopMaterialDiscovery::new(
        &origin, supply()?, [25, 25, 2], &fixture.context,
    )?;
    assert_eq!(observed, same);
    let different_center = WorkshopMaterialDiscovery::new(
        &origin, supply()?, [30, 25, 2], &fixture.context,
    )?;
    assert!(different_center.resolve(original_handle).is_err());
    let mut context = fixture.context.clone();
    context.session_id = SessionId::new(2);
    let other_session = WorkshopMaterialDiscovery::new(
        &origin, supply()?, [25, 25, 2], &context,
    )?;
    assert!(other_session.resolve(original_handle).is_err());
    assert!(observed.qualify(original_handle, &workshop(&observed, 0)?, &context).is_err());
    let mut later = supply()?;
    later.sequence += 1;
    let later = WorkshopMaterialDiscovery::new(
        &origin, canonical(&later)?, [25, 25, 2], &fixture.context,
    )?;
    assert!(later.resolve(original_handle).is_err());
    Ok(())
}

#[test]
fn tree_material_workshop_join_refuses_changed_item_source_footprint_and_eligibility() -> Result<()> {
    let mut fixture = open_fixture("designated", None, false)?;
    let observed = discovery(&mut fixture)?;
    let handle = observed.offers()[0].handle();
    let original = workshop(&observed, 0)?;
    for case in 0..13 {
        let mut capture = original.clone();
        match case {
            0 => capture.selection.item += 1,
            1 => capture.selection.center[0] += 1,
            2 => capture.tick += 1,
            3 => capture.fortress = FortressIdentity::new("another-world", capture.site())?,
            4 => capture.fortress = FortressIdentity::new(capture.folder(), capture.site() + 1)?,
            5 => capture.dimensions[2] += 1,
            6 => capture.paused = false,
            7 => capture.free_footprint = false,
            8 => capture.workers[0].labor = false,
            _ => {
                let WorkshopItem::Visible(item) = &mut capture.item else { return Err(invalid()); };
                match case {
                    9 => item.position[0] += 1,
                    10 => item.material += 1,
                    11 => item.material_index += 1,
                    _ => item.in_job = true,
                }
            }
        }
        let capture = WorkshopCapture::decode(&capture.encode())?;
        assert!(observed.qualify(handle, &capture, &fixture.context).is_err(), "case {case}");
    }
    // Each family has its own accepted native incarnation; the cross-family
    // join must not compare the numeric generation values as one shared epoch.
    let mut independent = original;
    independent.generation += 7;
    let independent = WorkshopCapture::decode(&independent.encode())?;
    observed.qualify(handle, &independent, &fixture.context)?;
    Ok(())
}

#[test]
fn tree_material_authority_and_work_bounds_cover_retained_and_fresh_ticks() -> Result<()> {
    let mut sampled = supply()?;
    sampled.sequence = 8;
    let mut fixture = open_fixture("designated", Some(canonical(&sampled)?), false)?;
    let before = custody_bytes(&fixture)?;
    let mut expired = fixture.context.clone();
    for grant in &mut expired.grants { grant.expires_at_tick = Some(GameTick(sampled.tick - 1)); }
    assert_eq!(
        fixture.journal.material_origin(&mut fixture.acknowledgement, &expired)
            .err().ok_or_else(invalid)?.code,
        ErrorCode::CapabilityDenied,
    );
    let origin = origin(&mut fixture)?;
    let mut context = fixture.context.clone();
    context.budget.max_bytes = 20 * 1024 * 1024;
    assert_eq!(
        fixture.journal.material_origin(&mut fixture.acknowledgement, &context)?,
        origin,
    );
    context.budget.max_bytes = 1024;
    assert_eq!(
        fixture.journal.material_origin(&mut fixture.acknowledgement, &context)
            .err().ok_or_else(invalid)?.code,
        ErrorCode::BudgetExceeded,
    );
    for case in 0..4 {
        let mut context = fixture.context.clone();
        match case {
            0 => context.cancellation_requested = true,
            1 => context.grants.retain(|grant| grant.capability != Capability::Observe),
            2 => {
                for grant in &mut context.grants {
                    grant.expires_at_tick = Some(GameTick(sampled.tick - 1));
                }
            }
            _ => context.budget.max_bytes = 1024,
        }
        assert!(WorkshopMaterialDiscovery::new(
            &origin, canonical(&sampled)?, [25, 25, 2], &context,
        ).is_err(), "case {case}");
    }
    assert_eq!(custody_bytes(&fixture)?, before);
    Ok(())
}

#[test]
fn tree_material_offer_bound_is_complete_and_never_truncates_native_supply() -> Result<()> {
    let mut fixture = open_fixture("designated", None, false)?;
    let origin = origin(&mut fixture)?;
    let mut progress = supply()?;
    let template = progress.wood[0].clone();
    progress.wood = (1000..1064).map(|id| tree::Wood { id, ..template.clone() }).collect();
    progress.next_item = 1064;
    let observed = WorkshopMaterialDiscovery::new(
        &origin, canonical(&progress)?, [25, 25, 2], &fixture.context,
    )?;
    assert_eq!(observed.offers().len(), WorkshopMaterialDiscovery::MAX_OFFERS);
    assert_eq!(observed.offers().last().ok_or_else(invalid)?.selection().item_id(), 1063);
    progress.wood.push(tree::Wood { id: 1064, ..template });
    progress.next_item = 1065;
    assert!(canonical(&progress).is_err());
    progress.wood.clear();
    assert!(WorkshopMaterialDiscovery::new(
        &origin, canonical(&progress)?, [25, 25, 2], &fixture.context,
    ).is_err());
    Ok(())
}
