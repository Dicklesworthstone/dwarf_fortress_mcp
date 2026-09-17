use super::*;
use dfmcp_core::{FortressId, GameTick, ObservationCursor};

fn sample(number: u64, epoch: u64, tick: u64, lower: u64, upper: Option<u64>) -> (JournalEntry, Value) {
    (JournalEntry { number, anchor: StateAnchor { fortress_id: FortressId::new(7),
        cursor: ObservationCursor {epoch,sequence:number}, tick: GameTick(tick), state_hash: Digest32::of_bytes(&number.to_be_bytes()) },
        source_digest: Digest32::of_bytes(b"source"), record_digest: Digest32::of_bytes(&number.to_be_bytes()),
        previous_digest: Digest32::ZERO, offset: 80, encoded_bytes: 400 },
        json!({"quantity":{"quantity_min":lower,"quantity_max":upper}}))
}

#[test]
fn finite_delta_bounds_equal_exhaustive_endpoint_differences() {
    for a in 0..8u64 { for b in a..8 { for c in 0..8u64 { for d in c..8 {
        let old = QuantityBounds { lower:a,upper:Some(b) };
        let new = QuantityBounds { lower:c,upper:Some(d) };
        let possible: Vec<_> = (a..=b).flat_map(|x|(c..=d).map(move |y|i128::from(y)-i128::from(x))).collect();
        assert_eq!(new.difference(old),(possible.iter().min().copied(),possible.iter().max().copied()));
    }}}}
}

#[test]
fn unknown_upper_bounds_remain_open_and_can_still_prove_a_direction() -> Result<()> {
    let known = sample(1,1,10,10,Some(10));
    let open = sample(2,1,20,11,None);
    let value = transition(&known,&open)?;
    assert_eq!(value["status"],"definite_increase");
    assert_eq!(value["net_change"]["minimum"],"1");
    assert!(value["net_change"]["maximum"].is_null());
    let old = sample(1,1,10,11,None);
    let next = sample(2,1,20,0,Some(10));
    let value = transition(&old,&next)?;
    assert_eq!(value["status"],"definite_decrease");
    assert!(value["net_change"]["minimum"].is_null());
    assert_eq!(value["net_change"]["maximum"],"-1");
    let next = sample(2,1,20,0,None);
    let value = transition(&old,&next)?;
    assert_eq!(value["status"],"indeterminate_change");
    assert!(value["net_change"]["minimum"].is_null());
    assert!(value["net_change"]["maximum"].is_null());
    assert_eq!(value["net_change"]["exact"],false); Ok(())
}

#[test]
fn rates_retain_full_u64_differences_without_floating_point_or_signed_overflow() -> Result<()> {
    for (old,new,numerator) in [(u64::MAX,0,"-18446744073709551615"),(0,u64::MAX,"18446744073709551615")] {
        let value=transition(&sample(1,1,0,old,Some(old)),&sample(2,1,u64::MAX,new,Some(new)))?;
        assert_eq!(value["net_change"]["minimum"],numerator);
        assert_eq!(value["net_rate"]["numerator_minimum"],numerator);
        assert_eq!(value["net_rate"]["numerator_maximum"],numerator);
        assert_eq!(value["net_rate"]["denominator_game_ticks"],u64::MAX);
        assert!(serde_json::to_string(&value).is_ok());
    }
    Ok(())
}

#[test]
fn same_tick_still_has_a_delta_but_never_a_division_by_zero() -> Result<()> {
    let value=transition(&sample(1,1,10,7,Some(7)),&sample(2,1,10,5,Some(5)))?;
    assert_eq!(value["status"],"definite_decrease");
    assert_eq!(value["net_change"]["minimum"],"-2");
    assert_eq!(value["elapsed_game_ticks"],0);
    assert!(value["net_rate"].is_null());
    assert_eq!(value["rate_unavailable_reason"],"same_game_tick"); Ok(())
}

#[test]
fn reset_or_regressed_clock_is_not_a_large_resource_loss() -> Result<()> {
    let first=sample(1,1,100,100,Some(100));
    for next in [sample(2,2,120,1,Some(1)),sample(2,1,99,1,Some(1))] {
        let value=transition(&first,&next)?;
        assert_eq!(value["status"],"epoch_or_clock_discontinuity");
        assert!(value["net_change"].is_null()); assert!(value["net_rate"].is_null());
    }
    assert!(transition(&first,&sample(3,1,120,1,Some(1))).is_err()); Ok(())
}

#[test]
fn equal_uncertain_bounds_do_not_claim_unchanged_quantity() -> Result<()> {
    let value=transition(&sample(1,1,1,5,Some(10)),&sample(2,1,2,5,Some(10)))?;
    assert_eq!(value["status"],"indeterminate_change");
    assert_eq!(value["net_change"]["minimum"],"-5");
    assert_eq!(value["net_change"]["maximum"],"5");
    let value=transition(&sample(1,1,1,5,Some(5)),&sample(2,1,2,5,Some(5)))?;
    assert_eq!(value["status"],"unchanged_quantity"); Ok(())
}

#[test]
fn timeline_continuations_bind_identity_and_require_canonical_forward_progress() -> Result<()> {
    let id=Digest32::of_bytes(b"session range measurement and archive");
    let token=continuation(id,2);
    assert_eq!(start_record(Some(&token),id,1,4)?,2);
    assert_eq!(start_record(None,id,1,4)?,1);
    assert!(matches!(start_record(Some(&token),Digest32::ZERO,1,4),Err(e) if e.code==ErrorCode::StaleAnchor));
    for bad in [token.replacen(":2:",":02:",1),continuation(id,1),continuation(id,5),"x".repeat(129),"hs1:+2:bad".into()] {
        assert!(start_record(Some(&bad),id,1,4).is_err());
    }
    Ok(())
}

#[test]
fn malformed_internal_measurement_never_becomes_a_zero_or_exact_value() {
    for value in [json!({}),json!({"quantity":{"quantity_min":0}}),
        json!({"quantity":{"quantity_min":10,"quantity_max":9}}),
        json!({"quantity":{"quantity_min":-1,"quantity_max":0}}),
        json!({"quantity":{"quantity_min":0,"quantity_max":"unknown"}})] {
        assert!(QuantityBounds::read(&value).is_err());
    }
}

#[cfg(unix)]
#[path = "spatial_history_series_handler_tests.rs"]
mod handler_tests;
