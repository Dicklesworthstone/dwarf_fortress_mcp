#![forbid(unsafe_code)]

use dfmcp_core::{Digest32, sha256};

#[test]
fn sha256_matches_standard_known_answer_vectors() {
    let vectors: [(&[u8], &str); 4] = [
        (
            b"",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            b"abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
        (
            b"The quick brown fox jumps over the lazy dog",
            "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592",
        ),
    ];

    for (message, expected) in vectors {
        assert_eq!(sha256(message).to_string(), expected);
        assert_eq!(Digest32::of_bytes(message).to_string(), expected);
    }
}

#[test]
fn sha256_distinguishes_prefix_and_boundary_ambiguities() {
    let cases: [(&[u8], &[u8]); 5] = [
        (b"a", b"a\0"),
        (b"ab", b"a\0b"),
        (b"domain-a\0payload", b"domain-b\0payload"),
        (&[0u8; 55], &[0u8; 56]),
        (&[0u8; 63], &[0u8; 64]),
    ];
    for (left, right) in cases {
        assert_ne!(sha256(left), sha256(right));
    }
}

#[test]
fn digest_display_is_fixed_width_lowercase_hex() {
    let rendered = sha256(b"canonical display").to_string();
    assert_eq!(rendered.len(), 64);
    assert!(
        rendered
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
}

#[test]
fn zero_digest_is_not_the_digest_of_empty_input() {
    assert_ne!(Digest32::ZERO, sha256(b""));
    assert_eq!(Digest32::ZERO.to_string(), "0".repeat(64));
}
