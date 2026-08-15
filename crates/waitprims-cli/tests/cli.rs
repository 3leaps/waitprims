use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_waitprims"))
}

#[test]
fn version_includes_dev_suffix() {
    let output = bin().arg("--version").output().expect("run --version");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0.1.0-dev"),
        "unexpected --version output: {stdout}"
    );
}

#[test]
fn help_mentions_diagnostic_cli() {
    let output = bin().arg("--help").output().expect("run --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Diagnostic CLI"),
        "unexpected --help output: {stdout}"
    );
}
