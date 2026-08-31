#![forbid(unsafe_code)]

//! Bounded synchronous connection admission for the first live read slice.
//!
//! Socket creation is deliberately separate from the DFHack wire codec and
//! semantic adapter. This module admits only numeric loopback endpoints, applies
//! finite connect/read/write deadlines, negotiates the authenticated protocol,
//! and immediately wraps the client in a fail-closed source fence. It does not
//! read environment variables, mint secrets, spawn threads, or grant semantic
//! capabilities.

use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use dfmcp_core::{DfmcpError, ErrorCode, Result};

use crate::{
    BridgeCredentials, DfHackRpcClient, FencedLiveSource, MAX_CLIENT_NAME_BYTES,
    MAX_CLIENT_VERSION_BYTES,
};

pub const MAX_ENDPOINT_BYTES: usize = 256;
pub const MAX_SOCKET_TIMEOUT_MILLIS: u64 = 60_000;

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

pub fn parse_loopback_endpoint(value: &str) -> Result<SocketAddr> {
    if value.is_empty() || value.len() > MAX_ENDPOINT_BYTES {
        return Err(error(
            ErrorCode::InvalidRequest,
            "bridge endpoint must be a bounded numeric IP:port value",
        ));
    }
    let address = value.parse::<SocketAddr>().map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "bridge endpoint must be a numeric IP:port value",
        )
    })?;
    if !address.ip().is_loopback() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "DFHack bridge connections are restricted to loopback",
        ));
    }
    Ok(address)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveConnectionConfig {
    pub endpoint: SocketAddr,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub write_timeout: Duration,
    pub client_name: String,
    pub client_version: String,
}

impl LiveConnectionConfig {
    pub fn validate(&self) -> Result<()> {
        if !self.endpoint.ip().is_loopback() {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "DFHack bridge connections are restricted to loopback",
            ));
        }
        for (name, timeout) in [
            ("connect", self.connect_timeout),
            ("read", self.read_timeout),
            ("write", self.write_timeout),
        ] {
            let millis = u64::try_from(timeout.as_millis()).map_err(|_| {
                error(
                    ErrorCode::InvalidRequest,
                    format!("{name} timeout cannot be represented in milliseconds"),
                )
            })?;
            if millis == 0 || millis > MAX_SOCKET_TIMEOUT_MILLIS {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    format!(
                        "{name} timeout must be in 1..={MAX_SOCKET_TIMEOUT_MILLIS} milliseconds"
                    ),
                ));
            }
        }
        if self.client_name.is_empty() || self.client_name.len() > MAX_CLIENT_NAME_BYTES {
            return Err(error(
                ErrorCode::InvalidRequest,
                format!(
                    "bridge client name must be in 1..={MAX_CLIENT_NAME_BYTES} bytes"
                ),
            ));
        }
        if self.client_version.is_empty()
            || self.client_version.len() > MAX_CLIENT_VERSION_BYTES
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                format!(
                    "bridge client version must be in 1..={MAX_CLIENT_VERSION_BYTES} bytes"
                ),
            ));
        }
        Ok(())
    }
}

pub type AuthenticatedLiveSource = FencedLiveSource<DfHackRpcClient<TcpStream>>;

pub fn connect_authenticated_live_source(
    config: &LiveConnectionConfig,
    credentials: BridgeCredentials,
) -> Result<AuthenticatedLiveSource> {
    config.validate()?;
    let stream = TcpStream::connect_timeout(&config.endpoint, config.connect_timeout).map_err(
        |source| {
            error(
                ErrorCode::AdapterUnavailable,
                format!("failed to connect to the loopback DFHack bridge: {source}"),
            )
            .retryable(true)
        },
    )?;
    stream
        .set_read_timeout(Some(config.read_timeout))
        .map_err(|source| {
            error(
                ErrorCode::AdapterUnavailable,
                format!("failed to configure the DFHack read deadline: {source}"),
            )
        })?;
    stream
        .set_write_timeout(Some(config.write_timeout))
        .map_err(|source| {
            error(
                ErrorCode::AdapterUnavailable,
                format!("failed to configure the DFHack write deadline: {source}"),
            )
        })?;
    stream.set_nodelay(true).map_err(|source| {
        error(
            ErrorCode::AdapterUnavailable,
            format!("failed to configure DFHack TCP_NODELAY: {source}"),
        )
    })?;
    let client = DfHackRpcClient::negotiate(
        stream,
        credentials,
        &config.client_name,
        &config.client_version,
    )?;
    FencedLiveSource::new(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> LiveConnectionConfig {
        LiveConnectionConfig {
            endpoint: parse_loopback_endpoint("127.0.0.1:5000")
                .map_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 5000)), |value| value),
            connect_timeout: Duration::from_secs(2),
            read_timeout: Duration::from_secs(5),
            write_timeout: Duration::from_secs(5),
            client_name: "dfmcp-test".to_owned(),
            client_version: "0.0.1".to_owned(),
        }
    }

    #[test]
    fn endpoint_parser_is_numeric_and_loopback_only() {
        assert!(parse_loopback_endpoint("127.0.0.1:5000").is_ok());
        assert!(parse_loopback_endpoint("[::1]:5000").is_ok());
        assert!(parse_loopback_endpoint("localhost:5000").is_err());
        assert!(parse_loopback_endpoint("192.0.2.1:5000").is_err());
        assert!(parse_loopback_endpoint("").is_err());
    }

    #[test]
    fn timeout_and_identity_bounds_are_enforced() {
        let valid = config();
        assert!(valid.validate().is_ok());

        let mut zero_timeout = valid.clone();
        zero_timeout.read_timeout = Duration::ZERO;
        assert!(zero_timeout.validate().is_err());

        let mut remote = valid.clone();
        remote.endpoint = SocketAddr::from(([192, 0, 2, 1], 5000));
        assert!(remote.validate().is_err());

        let mut oversized_name = valid;
        oversized_name.client_name = "x".repeat(MAX_CLIENT_NAME_BYTES + 1);
        assert!(oversized_name.validate().is_err());
    }
}
