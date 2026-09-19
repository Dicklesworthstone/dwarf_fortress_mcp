use super::*;

fn required(base: &[u8], target: &[u8]) -> Result<Vec<u8>, Error> {
    encode(base, target)?.ok_or(Error::InvalidEncoding)
}
fn frame(base: usize, target: usize, count: usize, commands: &[u8]) -> Vec<u8> {
    let mut value = MAGIC.to_vec();
    for n in [base, target, count] { value.extend_from_slice(&(n as u32).to_be_bytes()); }
    value.extend_from_slice(commands);
    value
}
fn hex(value: &str) -> Result<Vec<u8>, Error> {
    value.as_bytes().chunks_exact(2).map(|pair| {
        let text = std::str::from_utf8(pair).map_err(|_| Error::InvalidEncoding)?;
        u8::from_str_radix(text, 16).map_err(|_| Error::InvalidEncoding)
    }).collect()
}

#[test]
fn fixed_vector_agrees_with_independent_python_reference() -> Result<(), Error> {
    let base: Vec<u8> = (0..512).map(|n| (n % 256) as u8).collect();
    let mut target = base.clone();
    target[10] = 99;
    target[290..294].copy_from_slice(b"ABCD");
    let expected = hex("44464d444c543031000002000000020000000004000000000b0001020304050607080963010000000b000001170000000004414243440100000126000000da")?;
    assert_eq!(required(&base, &target)?, expected);
    assert_eq!(decode(&base, &expected, target.len())?, target);
    Ok(())
}

#[test]
fn changing_a_tick_saves_a_large_unchanged_capture() -> Result<(), Error> {
    let base = vec![42; 1024 * 1024];
    let mut target = base.clone();
    target[17] = 19;
    let delta = required(&base, &target)?;
    assert!(delta.len() < 64);
    assert_eq!(decode(&base, &delta, target.len())?, target);
    Ok(())
}

#[test]
fn shifted_suffixes_reconstruct_insertions_and_deletions() -> Result<(), Error> {
    let base: Vec<u8> = (0..4096).map(|n| (n % 251) as u8).collect();
    for start in [0, 17, 511, 2048] {
        let mut inserted = base.clone();
        inserted.splice(start..start, b"new entity".iter().copied());
        let delta = required(&base, &inserted)?;
        assert_eq!(decode(&base, &delta, inserted.len())?, inserted);
        let mut deleted = base.clone();
        deleted.drain(start..start + 13);
        let delta = required(&base, &deleted)?;
        assert_eq!(decode(&base, &delta, deleted.len())?, deleted);
    }
    Ok(())
}

#[test]
fn raw_fallback_handles_small_incompressible_and_overfragmented_payloads() -> Result<(), Error> {
    assert!(encode(b"", b"")?.is_none());
    assert!(encode(b"x", b"x")?.is_none());
    assert!(encode(&[0; 156], &[0; 156])?.is_none());
    assert_eq!(required(&[0; 157], &[0; 157])?.len(), 29);
    assert!(encode(&[0; 4096], &[1; 4096])?.is_none());
    let base = vec![1; MAX_COMMANDS * 33];
    let mut fragmented = base.clone();
    for byte in fragmented.iter_mut().skip(32).step_by(33) { *byte = 2; }
    assert!(encode(&base, &fragmented)?.is_none());
    Ok(())
}

#[test]
fn every_incomplete_prefix_and_trailing_byte_is_rejected() -> Result<(), Error> {
    let base = vec![7; 4096];
    let mut target = base.clone();
    target[10] = 8;
    let delta = required(&base, &target)?;
    for end in 0..delta.len() { assert!(decode(&base, &delta[..end], MAX_PAYLOAD).is_err()); }
    let mut trailing = delta.clone();
    trailing.push(0);
    assert_eq!(decode(&base, &trailing, MAX_PAYLOAD), Err(Error::InvalidEncoding));
    assert_eq!(decode(&base[..4095], &delta, MAX_PAYLOAD), Err(Error::WrongBaseLength));
    Ok(())
}

#[test]
fn output_and_command_bounds_are_validated_before_reconstruction() -> Result<(), Error> {
    let base = vec![9; 512];
    let delta = required(&base, &base)?;
    assert_eq!(decode(&base, &delta, 511), Err(Error::LimitExceeded));
    assert_eq!(decode(&base, &delta, MAX_PAYLOAD + 1), Err(Error::LimitExceeded));
    for commands in [vec![2, 0, 0, 0, 1], vec![0, 0, 0, 0, 0], vec![1, 0, 0, 0, 0, 0, 0, 0, 0],
        vec![1, 255, 255, 255, 255, 0, 0, 2, 0], vec![1, 0, 0, 2, 0, 0, 0, 0, 1],
        vec![1, 0, 0, 0, 0, 0, 0, 2, 1]] {
        assert_eq!(decode(&base, &frame(512, 512, 1, &commands), 512), Err(Error::InvalidEncoding));
    }
    for count in [0, MAX_COMMANDS + 1] {
        assert_eq!(decode(&base, &frame(512, 512, count, &[]), 512), Err(Error::InvalidEncoding));
    }
    let overflow = frame(512, MAX_PAYLOAD + 1, 1, &[1, 0, 0, 0, 0, 0, 0, 2, 0]);
    assert_eq!(decode(&base, &overflow, MAX_PAYLOAD), Err(Error::LimitExceeded));
    Ok(())
}

#[test]
fn maximum_payload_remains_bounded_and_byte_exact() -> Result<(), Error> {
    let base = vec![0; MAX_PAYLOAD];
    let delta = required(&base, &base)?;
    assert_eq!(delta.len(), 29);
    assert_eq!(decode(&base, &delta, MAX_PAYLOAD)?, base);
    let oversized = vec![0; MAX_PAYLOAD + 1];
    assert_eq!(encode(&base, &oversized), Err(Error::LimitExceeded));
    assert_eq!(encode(&oversized, &base), Err(Error::LimitExceeded));
    Ok(())
}

#[test]
fn generated_byte_edits_roundtrip_with_deterministic_raw_fallback() -> Result<(), Error> {
    let mut seed = 0xDF_A118_u64;
    for case in 0..2048usize {
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let length = (next() % 4097) as usize;
        let base: Vec<_> = (0..length).map(|_| next() as u8).collect();
        let mut target = base.clone();
        for _ in 0..(case % 9) {
            let at = (next() as usize) % (target.len() + 1);
            match next() % 3 {
                0 => { target.insert(at, next() as u8); }
                1 if at < target.len() => { target.remove(at); }
                _ if at < target.len() => { target[at] = next() as u8; }
                _ => {}
            }
        }
        let result = encode(&base, &target)?;
        assert_eq!(encode(&base, &target)?, result);
        if let Some(delta) = result {
            assert!(delta.len() + MIN_SAVINGS <= target.len());
            assert_eq!(decode(&base, &delta, target.len())?, target);
        }
    }
    Ok(())
}
