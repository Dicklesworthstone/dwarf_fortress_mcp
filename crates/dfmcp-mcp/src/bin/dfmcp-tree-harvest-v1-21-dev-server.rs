#![forbid(unsafe_code)]
fn main() -> std::process::ExitCode {
    dfmcp_mcp::tree_harvest_server::run_stdio();
    // Serving owns the process; returning means startup was refused.
    std::process::ExitCode::FAILURE
}
