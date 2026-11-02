//! Actual modern Rust subprocess, joined TCP doubles and private disk custody.
//! This component test is not native R1, live-game or admission evidence.
#[test]
fn standard_cli_manager_actual_modern_stdio_positive_negative_and_crash_recovery()
-> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let logs = std::env::temp_dir().join(format!("dfmcp-manager-cli-stdio-{}", std::process::id()));
    let output = std::process::Command::new("python3")
        .arg("-B")
        .arg(root.join("tests/stdio/manager_appointment.py"))
        .arg("--binary")
        .arg(env!("CARGO_BIN_EXE_dwarf-fortress-mcp"))
        .arg("--server-arg=serve-gameplay-dev")
        .arg("--server-arg=manager")
        .arg("--logs")
        .arg(&logs)
        .output()?;
    use std::io::Write;
    // Preserve a setup failure too, even if Python never created its exclusive directory.
    if !logs.exists() {
        std::fs::create_dir(&logs)?;
    }
    for (name, raw) in [
        ("python.stdout", output.stdout.as_slice()),
        ("python.stderr", output.stderr.as_slice()),
    ] {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(logs.join(name))?
            .write_all(raw)?;
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(logs.join("python.exit"))?
        .write_all(format!("{}\n", output.status).as_bytes())?;
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        output.status.success(),
        "logs={} stdout={} stderr={}",
        logs.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    let report: serde_json::Value = serde_json::from_str(
        stdout
            .lines()
            .last()
            .ok_or("missing actual test counters")?,
    )?;
    assert_eq!(report["tests"], 5);
    assert_eq!(report["failed"], 0);
    assert_eq!(report["errors"], 0);
    assert_eq!(report["skipped"], 0);
    assert_eq!(report["actual_modern_stdio"], true);
    assert_eq!(report["loopback_peers_are_doubles"], true);
    Ok(())
}
