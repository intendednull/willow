//! Inject `WILLOW_BUILD_SHA` via `option_env!` so diagnostics can
//! surface the short git SHA. Best-effort: empty in dev builds.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=WILLOW_BUILD_SHA");
    if std::env::var_os("WILLOW_BUILD_SHA").is_some() {
        return; // caller already set it
    }
    if let Ok(out) = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
    {
        if out.status.success() {
            let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !sha.is_empty() {
                println!("cargo:rustc-env=WILLOW_BUILD_SHA={sha}");
            }
        }
    }
}
