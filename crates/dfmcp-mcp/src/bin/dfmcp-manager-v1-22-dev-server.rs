#![forbid(unsafe_code)]
fn main() -> std::process::ExitCode {
    dfmcp_mcp::manager_appointment_server::run_stdio();
    std::process::ExitCode::FAILURE
}
