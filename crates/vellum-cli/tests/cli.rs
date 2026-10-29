// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Runs the real `vellum` binary.

use std::process::Command;

fn vellum() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vellum"));
    let _ = command.env_remove("OTEL_EXPORTER_OTLP_ENDPOINT");
    command
}

#[test]
fn prints_its_version() {
    let output = vellum().arg("--version").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        format!("vellum {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn starts_and_logs_json_to_stderr() {
    let output = vellum().env("VELLUM_LOG", "info").output().unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains(r#""message":"vellum started; no command given""#),
        "{stderr}"
    );
    assert!(stderr.contains(r#""name":"vellum.cli""#), "{stderr}");
}

#[test]
fn rejects_an_invalid_log_filter() {
    let output = vellum().env("VELLUM_LOG", "info,[").output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("invalid log filter"), "{stderr}");
}

#[test]
fn profile_renders_a_folded_file() {
    let dir = std::env::temp_dir().join(format!("vellum-cli-profile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("in.folded");
    let out = dir.join("out.svg");
    std::fs::write(&input, "http.request;db.query 900\n").unwrap();
    let output = vellum()
        .args(["profile", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(std::fs::read_to_string(&out).unwrap().starts_with("<svg"));
    let failed = vellum()
        .args(["profile", "--input"])
        .arg(dir.join("missing"))
        .output()
        .unwrap();
    assert!(!failed.status.success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn budgets_checks_a_routes_file() {
    let dir = std::env::temp_dir().join(format!("vellum-cli-budgets-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("routes.json");
    std::fs::write(
        &input,
        r#"{"minutes":15,"routes":[],"in_flight":0,"process":{},"buffer":{}}"#,
    )
    .unwrap();
    let passed = vellum()
        .args(["budgets", "--input"])
        .arg(&input)
        .output()
        .unwrap();
    assert!(
        passed.status.success(),
        "{}",
        String::from_utf8_lossy(&passed.stderr)
    );
    let failed = vellum()
        .args(["budgets", "--input"])
        .arg(dir.join("missing.json"))
        .output()
        .unwrap();
    assert!(!failed.status.success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mcp_demo_answers_discover_over_stdio() {
    use std::io::Write as _;
    use std::process::Stdio;

    let mut child = vellum()
        .args(["mcp", "--demo"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}"#;
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "{request}").unwrap();
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains(r#""supportedVersions":["2026-07-28","2025-11-25"]"#),
        "{stdout}"
    );
    assert_eq!(
        stdout.lines().count(),
        1,
        "stdout carries only MCP messages"
    );
}
