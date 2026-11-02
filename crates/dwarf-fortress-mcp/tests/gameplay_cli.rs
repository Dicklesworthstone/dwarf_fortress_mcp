//! Real standard executable routes; component peers are not a live game.
#[test]
fn standard_gameplay_cli_routes_existing_modern_profiles_and_refusals()
-> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let logs = std::env::temp_dir().join(format!("dfmcp-gameplay-cli-{}", std::process::id()));
    let output = std::process::Command::new("python3")
        .arg("-B")
        .arg(root.join("tests/stdio/gameplay_cli.py"))
        .arg("--binary")
        .arg(env!("CARGO_BIN_EXE_dwarf-fortress-mcp"))
        .arg("--logs")
        .arg(&logs)
        .output()?;
    use std::io::Write;
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
        "actual CLI logs={}",
        logs.display()
    );
    let stdout = String::from_utf8(output.stdout)?;
    let report: serde_json::Value =
        serde_json::from_str(stdout.lines().last().ok_or("missing CLI counters")?)?;
    assert_eq!(report["tests"], 3);
    assert_eq!(report["failed"], 0);
    assert_eq!(report["errors"], 0);
    assert_eq!(report["skipped"], 0);
    assert_eq!(report["actual_modern_stdio"], true);
    assert_eq!(report["live_game"], false);
    Ok(())
}
