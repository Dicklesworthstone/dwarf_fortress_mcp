#![forbid(unsafe_code)]

use std::error::Error;

use dfmcp_core::ErrorCode;
use dfmcp_mcp::validate_localhost_bind;

/// TEST: WP-14 Transport-Boundary Admission & Localhost Bind Gating
#[test]
fn test_wp14_localhost_bind_admission_and_negative_rejection() -> Result<(), Box<dyn Error>> {
    // 1. Valid localhost bindings must pass
    validate_localhost_bind("127.0.0.1:8080")?;
    validate_localhost_bind("localhost:3000")?;
    validate_localhost_bind("[::1]:8765")?;
    validate_localhost_bind("127.0.0.1")?;
    validate_localhost_bind("localhost")?;

    // 2. Non-localhost bindings must be strictly rejected with CapabilityDenied
    let non_localhost_cases = vec![
        "0.0.0.0:8080",
        "0.0.0.0",
        "192.168.1.100:8080",
        "10.0.0.1:3000",
        "172.16.0.1:8080",
        "example.com:8080",
        "[2001:db8::1]:8080",
    ];

    for addr in non_localhost_cases {
        let Err(err) = validate_localhost_bind(addr) else {
            return Err(format!("expected non-localhost bind '{addr}' to be rejected").into());
        };
        assert_eq!(err.code, ErrorCode::CapabilityDenied);
        assert!(err.message.contains("transport-boundary admission"));
    }

    Ok(())
}
