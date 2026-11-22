#![forbid(unsafe_code)]
fn main() -> std::process::ExitCode {
    dfmcp_mcp::native_resource_project::run_stdio();
    // Serving owns the process until exit; returning means startup was refused.
    std::process::ExitCode::FAILURE
}
