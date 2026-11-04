#![forbid(unsafe_code)]
//! Explicitly unadmitted cache-bearing Manager/1.23 startup. No production ticket.
pub fn run_stdio() {
    if std::env::var_os("DFMCP_ADMITTED_BRIDGE_PROTOCOL").is_some() {
        eprintln!("capability_denied: Manager/1.23 development refuses production launch state");
        return;
    }
    crate::manager_appointment_server::run_stdio_for_protocol(
        dfmcp_adapter::manager_appointment::ManagerProtocol::V1_23,
    );
}
