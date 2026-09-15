/// Stamp identity into the binary so the running app can say which
/// build it is.
///
/// The version alone does not answer that: every build on a given day carries
/// the same `3.15.7`, so "which one am I running" needs the commit and the
/// build time. Emitted as an epoch so no date formatting is needed in Rust --
/// the Options page turns it into local time.
///
/// Both are best-effort. A build from a tarball with no git, or with git
/// missing from PATH, still compiles and reports "unknown".
fn stamp_build_identity() {
    let sha = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    // CONTENT differences only. `git status --porcelain` was used here first
    // and reported this tree permanently dirty: Cargo.toml differs from the
    // index by line endings alone, so every build carried a "+dirty" that meant
    // nothing. `git diff --quiet` compares after normalisation, so a CRLF-only
    // difference is clean and a real edit is not.
    let dirty = std::process::Command::new("git")
        .args(["diff", "--quiet", "HEAD"])
        .status()
        .ok()
        .map(|st| !st.success())
        .unwrap_or(false);

    // A monotonic build number: commits on this branch. It rises with every
    // commit, so a larger number is a later build, which "3.15.7" cannot tell
    // you -- every build this month carries that same version.
    let number = std::process::Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "0".to_string());
    println!("cargo:rustc-env=GBF_BUILD_NUMBER={number}");

    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    println!(
        "cargo:rustc-env=GBF_BUILD_COMMIT={}{}",
        sha,
        if dirty { "+dirty" } else { "" }
    );
    println!("cargo:rustc-env=GBF_BUILD_EPOCH={epoch}");
    // Without this the stamp is baked once and then cached forever, which is
    // exactly the staleness this is meant to cure.
    println!("cargo:rerun-if-changed=src");
}

fn main() {
    stamp_build_identity();
    println!("cargo:rerun-if-changed=.pake/pake.json");
    println!("cargo:rerun-if-changed=.pake/tauri.conf.json");
    println!("cargo:rerun-if-changed=../dist/gbf-sidebar.html");
    println!("cargo:rerun-if-changed=../dist/gbf-options.html");
    println!("cargo:rerun-if-changed=../dist/gbf-about.html");
    tauri_build::build()
}
