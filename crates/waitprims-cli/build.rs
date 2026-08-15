//! Stamp a short git SHA onto the diagnostic binary version.

fn main() {
    let pkg = env!("CARGO_PKG_VERSION");
    let sha = git_short_sha();
    let version = match sha {
        Some(sha) => format!("{pkg}+{sha}"),
        None => pkg.to_string(),
    };
    println!("cargo:rustc-env=WAITPRIMS_VERSION={version}");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");
}

fn git_short_sha() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8(output.stdout).ok()?;
    let sha = sha.trim();
    if sha.is_empty() {
        None
    } else {
        Some(sha.to_string())
    }
}
