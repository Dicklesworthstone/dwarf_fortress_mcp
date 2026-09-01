#![forbid(unsafe_code)]

//! Loopback-only connector for the isolated protocol-1.1 bridge generation.

use std::net::TcpStream;

use dfmcp_core::{DfmcpError, ErrorCode, Result};

use crate::{
    BridgeCredentialsV1_1, DfHackRpcClientV1_1, FencedLiveSourceV1_1,
    LiveConnectionConfig,
};

pub type AuthenticatedLiveSourceV1_1 =
    FencedLiveSourceV1_1<DfHackRpcClientV1_1<TcpStream>>;

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

pub fn connect_authenticated_live_source_v1_1(
    config: &LiveConnectionConfig,
    credentials: BridgeCredentialsV1_1,
) -> Result<AuthenticatedLiveSourceV1_1> {
    config.validate()?;
    if !config.endpoint.ip().is_loopback() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "protocol-1.1 live bridge endpoint must be numeric loopback",
        ));
    }
    let stream = TcpStream::connect_timeout(&config.endpoint, config.connect_timeout)
        .map_err(|source| {
            error(
                ErrorCode::AdapterUnavailable,
                format!(
                    "protocol-1.1 DFHack bridge connection to {} failed: {source}",
                    config.endpoint
                ),
            )
            .retryable(true)
        })?;
    stream
        .set_read_timeout(Some(config.read_timeout))
        .map_err(|source| {
            error(
                ErrorCode::AdapterFailure,
                format!("cannot set protocol-1.1 bridge read timeout: {source}"),
            )
        })?;
    stream
        .set_write_timeout(Some(config.write_timeout))
        .map_err(|source| {
            error(
                ErrorCode::AdapterFailure,
                format!("cannot set protocol-1.1 bridge write timeout: {source}"),
            )
        })?;
    stream.set_nodelay(true).map_err(|source| {
        error(
            ErrorCode::AdapterFailure,
            format!("cannot enable protocol-1.1 bridge TCP_NODELAY: {source}"),
        )
    })?;
    let client = DfHackRpcClientV1_1::negotiate(
        stream,
        credentials,
        &config.client_name,
        &config.client_version,
    )?;
    Ok(FencedLiveSourceV1_1::new(client))
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::time::Duration;

    use super::*;
    use crate::{MIN_V1_1_BRIDGE_TOKEN_BYTES, MIN_V1_1_NONCE_BYTES};

    fn config(endpoint: SocketAddr) -> LiveConnectionConfig {
        LiveConnectionConfig {
            endpoint,
            connect_timeout: Duration::from_millis(1),
            read_timeout: Duration::from_millis(1),
            write_timeout: Duration::from_millis(1),
            client_name: "dfmcp-test".to_owned(),
            client_version: "0.0.1".to_owned(),
        }
    }

    fn credentials() -> Result<BridgeCredentialsV1_1> {
        BridgeCredentialsV1_1::new(
            vec![b'x'; MIN_V1_1_BRIDGE_TOKEN_BYTES],
            vec![b'n'; MIN_V1_1_NONCE_BYTES],
        )
    }

    #[test]
    fn non_loopback_endpoint_fails_before_network_io() -> Result<()> {
        let endpoint = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), 5000);
        let failure = connect_authenticated_live_source_v1_1(
            &config(endpoint),
            credentials()?,
        );
        assert!(failure.is_err());
        Ok(())
    }

    #[test]
    fn closed_loopback_endpoint_is_retryable_failure() -> Result<()> {
        let endpoint = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9);
        let failure = connect_authenticated_live_source_v1_1(
            &config(endpoint),
            credentials()?,
        );
        assert!(failure.is_err());
        Ok(())
    }
}
