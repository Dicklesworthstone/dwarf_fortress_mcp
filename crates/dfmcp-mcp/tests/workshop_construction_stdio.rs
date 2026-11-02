//! Real modern consumer process, joined explicit TCP peers and real private files.
//! No game, SDK R1 or admission claim follows from this component execution.
#[test]
fn workshop_construction_server_actual_modern_stdio_positive_negative_and_crash_recovery()
-> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let logs = std::env::temp_dir().join(format!("dfmcp-workshop-stdio-{}", std::process::id()));
    let output = std::process::Command::new("python3")
        .arg("-B")
        .arg(root.join("tests/stdio/workshop.py"))
        .arg("--binary")
        .arg(env!("CARGO_BIN_EXE_dfmcp-workshop-v1-20-dev-server"))
        .arg("--logs")
        .arg(&logs)
        .output()?;
    use std::io::Write;
    for (name, bytes) in [
        ("python.stdout", output.stdout.as_slice()),
        ("python.stderr", output.stderr.as_slice()),
    ] {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(logs.join(name))?
            .write_all(bytes)?;
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
        "stdout={}\nstderr={}\nlogs={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        logs.display()
    );
    let summary = String::from_utf8(output.stdout)?;
    let report: serde_json::Value =
        serde_json::from_str(summary.lines().last().ok_or("missing test counters")?)?;
    assert_eq!(report["tests"], 7);
    assert_eq!(report["failed"], 0);
    assert_eq!(report["errors"], 0);
    assert_eq!(report["skipped"], 0);
    assert_eq!(report["actual_modern_stdio"], true);
    assert_eq!(report["loopback_peers_are_doubles"], true);
    Ok(())
}
