#![forbid(unsafe_code)]

use dfmcp_adapter::{
    AnnouncementPage, AnnouncementRecord, AnnouncementSourceIdentity,
    AnnouncementWindowAssembler,
};
use dfmcp_core::Result;

fn source() -> AnnouncementSourceIdentity {
    AnnouncementSourceIdentity {
        bridge_version: "0.2.0".to_owned(),
        dfhack_version: "0.51.11-r1".to_owned(),
        dwarf_fortress_version: "0.51.11".to_owned(),
        protocol_major: 1,
        protocol_minor: 1,
        bridge_generation: 77,
    }
}

fn record(id: i32) -> AnnouncementRecord {
    AnnouncementRecord {
        report_id: id,
        announcement_type: 1,
        text: format!("report-{id}"),
        year: 105,
        year_tick: 12_345,
        has_position: false,
        x: 0,
        y: 0,
        z: 0,
        repeat_count: 0,
        continuation: false,
        unconscious: false,
        announcement: true,
    }
}

fn page(after: i32, maximum: u32, ids: &[i32], complete: bool) -> AnnouncementPage {
    AnnouncementPage {
        bridge_generation: 77,
        requested_after_report_id: after,
        requested_maximum: maximum,
        oldest_retained_report_id: 10,
        latest_retained_report_id: 13,
        window_latest_report_id: 13,
        next_after_report_id: ids.last().copied().unwrap_or(after),
        history_truncated: false,
        complete,
        announcements: ids.iter().copied().map(record).collect(),
    }
}

#[test]
fn public_api_reproduces_identity_across_page_sizes() -> Result<()> {
    let mut one = AnnouncementWindowAssembler::new(source(), 9)?;
    one.push_page(page(9, 4, &[10, 11, 12, 13], true))?;
    let one = one.finalize()?;

    let mut many = AnnouncementWindowAssembler::new(source(), 9)?;
    many.push_page(page(9, 2, &[10, 11], false))?;
    many.push_page(page(11, 2, &[12, 13], true))?;
    let many = many.finalize()?;

    assert_eq!(one.content_digest, many.content_digest);
    assert_eq!(one.canonical_bytes, many.canonical_bytes);
    assert_eq!(one.next_after_report_id, 13);
    assert!(one.can_prove_absence_in_frozen_interval());
    Ok(())
}

#[test]
fn public_api_preserves_partial_history_posture() -> Result<()> {
    let mut assembler = AnnouncementWindowAssembler::new(source(), 3)?;
    let mut retained = page(3, 4, &[10, 11, 12, 13], true);
    retained.history_truncated = true;
    assembler.push_page(retained)?;
    let window = assembler.finalize()?;
    assert!(window.history_truncated);
    assert!(!window.can_prove_absence_in_frozen_interval());
    Ok(())
}
