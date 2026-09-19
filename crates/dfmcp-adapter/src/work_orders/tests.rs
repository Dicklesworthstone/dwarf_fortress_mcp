use super::*;

const OBSERVATION: &str = include_str!("../../tests/fixtures/work_order_observation_v1_10.hex");
const PREPARED: &str = include_str!("../../tests/fixtures/work_order_prepared_v1_10.hex");
const CREATED: &str = include_str!("../../tests/fixtures/work_order_created_v1_10.hex");
const REFUSED: &str = include_str!("../../tests/fixtures/work_order_refused_v1_10.hex");
const UNKNOWN: &str = include_str!("../../tests/fixtures/work_order_unknown_v1_10.hex");

fn hex(value: &str) -> Result<Vec<u8>> {
    let value = value.trim();
    require(value.len() % 2 == 0, "odd fixture hex")?;
    value.as_bytes().chunks_exact(2).map(|pair| {
        let a = char::from(pair[0]).to_digit(16).ok_or_else(|| invalid("bad fixture hex"))?;
        let b = char::from(pair[1]).to_digit(16).ok_or_else(|| invalid("bad fixture hex"))?;
        Ok((a * 16 + b) as u8)
    }).collect()
}
fn plan() -> Result<WorkOrderPlan> {
    WorkOrderPlan::new(WorkOrderObservation::decode(&hex(OBSERVATION)?)?, "order-001",
        WorkOrderSpec::new(WorkOrderRecipe::WoodenBed, 5)?)
}
fn observation(ids: &[u32], horizon: u32, paused: u8) -> Vec<u8> {
    let mut out = b"DFMWO010".to_vec();
    for n in [7u64, 0, 12345] { out.extend_from_slice(&n.to_be_bytes()); }
    out.extend_from_slice(&horizon.to_be_bytes()); out.extend_from_slice(&1u32.to_be_bytes()); out.push(paused);
    put_text(&mut out, "region1"); out.extend_from_slice(&(ids.len() as u32).to_be_bytes());
    for id in ids { out.extend_from_slice(&id.to_be_bytes()); }
    out
}
fn rehash(bytes: &mut [u8]) {
    let mut proof = b"dfmcp-work-order-receipt/1\0".to_vec();
    proof.extend_from_slice(&bytes[8..16]); proof.extend_from_slice(&bytes[227..]);
    proof.extend_from_slice(&bytes[73..195]);
    bytes[195..227].copy_from_slice(Digest32::of_bytes(&proof).as_bytes());
}
fn created_for(p: &WorkOrderPlan) -> Result<Vec<u8>> {
    let o = p.observation(); let mut out = b"DFMWOE10".to_vec();
    for n in [o.generation(), o.sequence(), o.tick()] { out.extend_from_slice(&n.to_be_bytes()); }
    out.extend_from_slice(&o.next_order_id().to_be_bytes()); p.spec().append(&mut out);
    out.extend_from_slice(o.witness().as_bytes()); out.extend_from_slice(p.digest().as_bytes());
    out.extend_from_slice(p.prepare_token()); out.extend_from_slice(&[2, 1]); out.extend_from_slice(&o.tick().to_be_bytes());
    out.extend_from_slice(o.expected_after_witness()?.as_bytes()); out.extend_from_slice(p.configuration_witness().as_bytes());
    out.extend_from_slice(&[0; 32]); put_text(&mut out, p.key()); rehash(&mut out); Ok(out)
}

#[test]
fn actual_native_vectors_match_complete_sealed_evidence() -> Result<()> {
    let p = plan()?; let o = p.observation();
    assert_eq!(o.canonical_bytes(), observation(&[2, 6], 10, 1));
    assert_eq!(o.generation(), 7); assert_eq!(o.sequence(), 0); assert_eq!(o.tick(), 12345);
    assert_eq!(o.order_ids(), &[2, 6]); assert_eq!(o.next_order_id(), 10);
    assert_eq!(o.world_folder(), "region1"); assert_eq!(o.site_id(), 1); assert!(o.paused() && o.eligible());
    assert_eq!(created_for(&p)?, hex(CREATED)?);
    for (raw, state) in [(PREPARED, WorkOrderState::Prepared), (UNKNOWN, WorkOrderState::Unknown),
        (CREATED, WorkOrderState::Created), (REFUSED, WorkOrderState::Refused)] {
        let raw = hex(raw)?; let effect = WorkOrderEffect::decode(&raw, &p)?;
        assert_eq!(effect.canonical_bytes(), raw); assert_eq!(effect.state(), state);
        assert_eq!(effect.receipt().is_some(), state.terminal());
        assert_eq!(effect.created_order_id(), (state == WorkOrderState::Created).then_some(10));
        assert_eq!(effect.observed_tick(), (state == WorkOrderState::Created).then_some(12345));
        assert_eq!(effect.after_witness().is_some(), state == WorkOrderState::Created);
        assert_eq!(effect.configuration_witness().is_some(), state == WorkOrderState::Created);
    }
    Ok(())
}

#[test]
fn catalog_and_quantities_are_closed_and_keys_are_not_plan_authority() -> Result<()> {
    for code in [0, 5, 69, 257, u32::MAX] { assert!(WorkOrderRecipe::from_code(code).is_err()); }
    for amount in [0, 101, 32768, u32::MAX] { assert!(WorkOrderSpec::new(WorkOrderRecipe::WoodenBed, amount).is_err()); }
    let p = plan()?;
    for code in 1..=4 { for amount in 1..=100 {
        let spec = WorkOrderSpec::new(WorkOrderRecipe::from_code(code)?, amount)?;
        assert!(!spec.recipe().as_str().is_empty()); assert_eq!(spec.amount(), amount);
        let selected = WorkOrderPlan::new(p.observation().clone(), "another-key", spec)?;
        assert_eq!(WorkOrderEffect::decode(&created_for(&selected)?, &selected)?.created_order_id(), Some(10));
    }}
    let another = WorkOrderPlan::new(p.observation().clone(), "another-key", p.spec())?;
    assert_eq!(another.digest(), p.digest()); assert_ne!(another.prepare_token(), p.prepare_token());
    assert!(WorkOrderEffect::decode(&hex(CREATED)?, &another).is_err());
    for key in ["", "bad/key", "a b", "é", "x\0y"] { assert!(validate_key(key).is_err()); }
    assert!(validate_key(&"a".repeat(129)).is_err()); validate_key(&"a".repeat(128))?;
    Ok(())
}

#[test]
fn every_bit_of_a_created_record_is_bound_to_the_retained_plan() -> Result<()> {
    let p = plan()?; let raw = hex(CREATED)?;
    for index in 0..raw.len() { for bit in 0..8 {
        let mut bad = raw.clone(); bad[index] ^= 1 << bit;
        assert!(WorkOrderEffect::decode(&bad, &p).is_err(), "bit {index}:{bit} escaped");
    }}
    Ok(())
}

#[test]
fn self_consistent_receipts_for_wrong_queue_template_or_tick_are_rejected() -> Result<()> {
    let p = plan()?; let good = hex(CREATED)?;
    for index in [15, 23, 31, 35, 36, 40, 73, 105, 123, 130, 131, 162, 163, 194, 229] {
        let mut bad = good.clone(); bad[index] ^= 1; rehash(&mut bad);
        assert!(WorkOrderEffect::decode(&bad, &p).is_err(), "rehash forged field {index}");
    }
    let mut absent = good; absent[122] = 0; absent[123..195].fill(0); rehash(&mut absent);
    assert!(WorkOrderEffect::decode(&absent, &p).is_err());
    Ok(())
}

#[test]
fn outcome_classes_and_absent_backing_fields_are_not_interchangeable() -> Result<()> {
    let p = plan()?;
    for raw in [PREPARED, UNKNOWN, REFUSED] {
        let raw = hex(raw)?;
        for index in [122, 123, 131, 163] {
            let mut bad = raw.clone(); bad[index] = 1;
            if raw[121] == 4 { rehash(&mut bad); }
            assert!(WorkOrderEffect::decode(&bad, &p).is_err());
        }
    }
    for code in [3, 5, 255] {
        let mut bad = hex(REFUSED)?; bad[121] = code; rehash(&mut bad);
        assert!(WorkOrderEffect::decode(&bad, &p).is_err());
    }
    for raw in [PREPARED, UNKNOWN] {
        let mut bad = hex(raw)?; bad[195] = 1;
        assert!(WorkOrderEffect::decode(&bad, &p).is_err());
    }
    Ok(())
}

#[test]
fn bounded_decoders_refuse_all_incomplete_prefixes_trailing_bytes_and_bad_membership() -> Result<()> {
    let p = plan()?; let obs = hex(OBSERVATION)?;
    for size in 0..obs.len() { assert!(WorkOrderObservation::decode(&obs[..size]).is_err()); }
    for raw in [PREPARED, UNKNOWN, CREATED, REFUSED] {
        let raw = hex(raw)?;
        for size in 0..raw.len() { assert!(WorkOrderEffect::decode(&raw[..size], &p).is_err()); }
        let mut trailing = raw; trailing.push(0); assert!(WorkOrderEffect::decode(&trailing, &p).is_err());
    }
    let mut trailing = obs; trailing.push(0); assert!(WorkOrderObservation::decode(&trailing).is_err());
    for ids in [vec![2, 2], vec![6, 2], vec![2, 10], vec![u32::MAX]] {
        assert!(WorkOrderObservation::decode(&observation(&ids, 10, 1)).is_err());
    }
    assert!(WorkOrderObservation::decode(&observation(&[2], 10, 2)).is_err());
    assert!(WorkOrderObservation::decode(&vec![0; MAX_OBSERVATION_BYTES + 1]).is_err());
    assert!(WorkOrderEffect::decode(&vec![0; MAX_EFFECT_BYTES + 1], &p).is_err());
    Ok(())
}

#[test]
fn capacity_horizon_clock_and_incarnation_boundaries_fail_closed() -> Result<()> {
    let p = plan()?;
    let full = WorkOrderObservation::decode(&observation(&(0..4096).collect::<Vec<_>>(), 4096, 1))?;
    assert!(!full.eligible()); assert!(WorkOrderPlan::new(full, "full", p.spec()).is_err());
    let almost = WorkOrderObservation::decode(&observation(&(0..4095).collect::<Vec<_>>(), 4095, 1))?;
    let almost = WorkOrderPlan::new(almost, "last-slot", p.spec())?;
    assert_eq!(WorkOrderEffect::decode(&created_for(&almost)?, &almost)?.created_order_id(), Some(4095));
    for (offset, value) in [(8, 0u64), (8, u64::MAX), (16, u64::MAX), (24, MAX_NATIVE_TICK + 1)] {
        let mut bad = hex(OBSERVATION)?; bad[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
        assert!(WorkOrderObservation::decode(&bad).is_err());
    }
    for offset in [32, 36] {
        let mut bad = hex(OBSERVATION)?; bad[offset..offset + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(WorkOrderObservation::decode(&bad).is_err());
    }
    for (ids, horizon, paused) in [(vec![], i32::MAX as u32, 1), (vec![], 0, 0)] {
        let o = WorkOrderObservation::decode(&observation(&ids, horizon, paused))?;
        assert!(!o.eligible()); assert!(WorkOrderPlan::new(o, "ineligible", p.spec()).is_err());
    }
    let last = WorkOrderObservation::decode(&observation(&[], i32::MAX as u32 - 1, 1))?;
    let last = WorkOrderPlan::new(last, "last-id", p.spec())?;
    assert_eq!(WorkOrderEffect::decode(&created_for(&last)?, &last)?.created_order_id(), Some(i32::MAX as u32 - 1));
    Ok(())
}

#[test]
fn observation_ownership_utf8_and_fortress_lineage_are_explicit() -> Result<()> {
    let mut raw = hex(OBSERVATION)?; let observed = WorkOrderObservation::decode(&raw)?;
    raw[43] = b'x'; assert_ne!(WorkOrderObservation::decode(&raw)?.witness(), observed.witness());
    assert_eq!(observed.world_folder(), "region1");
    for byte in [0, 0xff, 0xc0] {
        let mut bad = hex(OBSERVATION)?; bad[43] = byte;
        assert!(WorkOrderObservation::decode(&bad).is_err());
    }
    let job = crate::job_suspension::JobObservation::decode(&hex(include_str!("../../tests/fixtures/job_suspension_observation_v1_9.hex"))?)?;
    assert_eq!(observed.fortress_id(), crate::job_suspension::coordinator::job_fortress_id(&job));
    Ok(())
}
