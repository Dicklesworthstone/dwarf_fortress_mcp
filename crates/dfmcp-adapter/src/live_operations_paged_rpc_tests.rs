use super::*;
use crate::live_jobs::LiveJobObservation;
use crate::live_operations::LiveItem;
use dfmcp_core::MapCoord;
use std::io::{self, Cursor};

struct Script {
    input: Cursor<Vec<u8>>,
    output: Vec<u8>,
}
impl Read for Script {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = buffer.len().min(7);
        self.input.read(&mut buffer[..count])
    }
}
impl Write for Script {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = bytes.len().min(11);
        self.output.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn source(count: u32) -> LiveOperationsObservation {
    LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 7,
            df_version: "df".to_owned(),
            dfhack_version: "dfhack".to_owned(),
            year: 105,
            year_tick: 3,
            paused: true,
            site_id: 1,
            world_folder: "region1".to_owned(),
            next_job_id: 0,
            jobs: Vec::new(),
        },
        next_building_id: 0,
        next_item_id: count,
        buildings: Vec::new(),
        attachments: Vec::new(),
        items: (0..count)
            .map(|id| LiveItem {
                native_id: id,
                item_type: 1,
                type_key: "item_type_1".to_owned(),
                subtype: -1,
                material_type: 0,
                material_index: 1,
                stack_size: 1,
                raw_position: MapCoord::new(0, 0, 0),
                flags: 64,
                container_native_id: None,
                holder_building_native_id: None,
            })
            .collect(),
    }
}
fn reply() -> Vec<u8> {
    let mut out = Vec::new();
    for (field, value) in [(1, 1), (2, 0), (4, 1), (5, 4), (6, 7)] {
        number(&mut out, field, value);
    }
    bytes(&mut out, 3, &[b'n'; 16]);
    bytes(&mut out, 7, b"df");
    bytes(&mut out, 8, b"dfhack");
    out
}
fn frame(payload: &[u8]) -> Vec<u8> {
    let mut out = crate::live_jobs_rpc::header(-1, payload.len() as i32).to_vec();
    out.extend_from_slice(payload);
    out
}
fn scripted(payload: &[u8], width: usize, fault: Option<&str>) -> Script {
    let mut input = b"DFHack!\n".to_vec();
    input.extend_from_slice(&1i32.to_le_bytes());
    for method in [2, 3] {
        let mut body = Vec::new();
        number(&mut body, 1, method);
        input.extend(frame(&body));
    }
    input.extend(frame(&reply()));
    let digest = Digest32::of_bytes(payload);
    for (index, data) in payload.chunks(width).enumerate() {
        let mut body = reply();
        let mut data = data.to_vec();
        if fault == Some("corruption") && index == 1 {
            data[0] ^= 1;
        }
        bytes(&mut body, 9, &data);
        bytes(
            &mut body,
            10,
            if fault == Some("token") && index == 1 {
                &[2; 16]
            } else {
                &[1; 16]
            },
        );
        number(
            &mut body,
            11,
            if fault == Some("offset") && index == 1 {
                0
            } else {
                (index * width) as u64
            },
        );
        number(&mut body, 12, payload.len() as u64);
        bytes(&mut body, 13, digest.as_bytes());
        number(
            &mut body,
            14,
            u64::from((index + 1) * width >= payload.len()),
        );
        input.extend(frame(&body));
    }
    if fault != Some("release_lost") {
        let mut ack = reply();
        bytes(&mut ack, 10, &[1; 16]);
        input.extend(frame(&ack));
    }
    Script {
        input: Cursor::new(input),
        output: Vec::new(),
    }
}

#[test]
fn actual_wire_assembles_large_fragmented_capture_and_releases_it() -> Result<()> {
    let observation = source(40_000);
    let payload = observation.encode_profile(OperationsProfile::PagedV1_4)?;
    assert_eq!(payload.len(), 2_200_070);
    assert_eq!(
        Digest32::of_bytes(&payload).to_string(),
        "6fb2e8c93943abc31f4b66c496c4aeabf95ab0ac312596068344950c28f3ef09"
    );
    let limits = PagedOperationsLimits {
        page_bytes: MIN_PAGE_BYTES,
        ..PagedOperationsLimits::default()
    };
    let mut client = PagedOperationsRpcClient::negotiate(
        scripted(&payload, limits.page_bytes, None),
        vec![b't'; 32],
        vec![b'n'; 16],
        limits,
    )?;
    assert_eq!(client.read_observation()?, observation);
    assert_eq!(client.last_page_count(), 135);
    assert!(!client.poisoned());
    assert_eq!(
        client.stream.input.position() as usize,
        client.stream.input.get_ref().len()
    );
    Ok(())
}

#[test]
fn mixed_pages_corruption_and_lost_release_fence_without_returning_a_world() -> Result<()> {
    let payload = source(700).encode_profile(OperationsProfile::PagedV1_4)?;
    let limits = PagedOperationsLimits {
        page_bytes: MIN_PAGE_BYTES,
        ..PagedOperationsLimits::default()
    };
    for fault in ["token", "offset", "corruption", "release_lost"] {
        let mut client = PagedOperationsRpcClient::negotiate(
            scripted(&payload, limits.page_bytes, Some(fault)),
            vec![b't'; 32],
            vec![b'n'; 16],
            limits,
        )?;
        assert!(client.read_observation().is_err(), "{fault}");
        assert!(client.poisoned());
        let bytes_sent = client.stream.output.len();
        assert!(client.read_observation().is_err());
        assert_eq!(client.stream.output.len(), bytes_sent);
    }
    Ok(())
}

#[test]
fn complete_transport_does_not_waive_requested_roster_limits() -> Result<()> {
    let payload = source(700).encode_profile(OperationsProfile::PagedV1_4)?;
    let limits = PagedOperationsLimits {
        items: 699,
        ..PagedOperationsLimits::default()
    };
    let mut client = PagedOperationsRpcClient::negotiate(
        scripted(&payload, limits.page_bytes, None),
        vec![b't'; 32],
        vec![b'n'; 16],
        limits,
    )?;
    assert!(
        matches!(client.read_observation(), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    assert!(client.poisoned());
    Ok(())
}

#[test]
fn protocol_confusion_and_duplicate_metadata_are_rejected() {
    let mut body = reply();
    number(&mut body, 5, 3);
    assert!(envelope(&body, &[b'n'; 16]).is_err());
    assert!(envelope(&reply(), &[b'x'; 16]).is_err());
    let mut body = reply();
    bytes(&mut body, 10, &[1; 16]);
    bytes(&mut body, 10, &[2; 16]);
    assert!(page(&body, &[b'n'; 16], MIN_PAGE_BYTES).is_err());
}

#[test]
fn endpoint_and_request_bounds_fail_before_network_access() {
    let limits = PagedOperationsLimits::default();
    assert!(
        PagedOperationsRpcClient::connect(
            SocketAddr::from(([192, 0, 2, 1], 5000)),
            vec![b't'; 32],
            vec![b'n'; 16],
            Duration::from_millis(1),
            limits
        )
        .is_err()
    );
    for limits in [
        PagedOperationsLimits {
            items: 65537,
            ..limits
        },
        PagedOperationsLimits {
            page_bytes: 1,
            ..limits
        },
        PagedOperationsLimits {
            payload_bytes: usize::MAX,
            ..limits
        },
    ] {
        assert!(limits.validate().is_err());
    }
}
