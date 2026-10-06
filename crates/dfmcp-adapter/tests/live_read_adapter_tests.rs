#![forbid(unsafe_code)]

use dfmcp_adapter::live_adapter::{DWARF_FORTRESS_TICKS_PER_YEAR, LiveReadAdapterConfig};

#[test]
fn calendar_constant_matches_the_protocol_contract() {
    assert_eq!(DWARF_FORTRESS_TICKS_PER_YEAR, 403_200);
}

#[test]
fn live_adapter_config_rejects_zero_page_size() {
    let config = LiveReadAdapterConfig {
        fortress_id: dfmcp_core::FortressId::new(1),
        page_size: 0,
        max_citizens: 1000,
        include_names: true,
        initial_epoch: 0,
    };
    assert!(config.validate().is_err());
}
