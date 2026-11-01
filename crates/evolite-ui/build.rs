//! The commit this build is from, for the journal and problem reports (`EVOLITE_COMMIT`);
//! `unknown` outside a git checkout. `demo_core`: the demo core is in this build — with
//! `dev-tools`, or without `real`, where it is the only core.

fn main() {
    println!("cargo:rustc-check-cfg=cfg(demo_core)");
    if std::env::var_os("CARGO_FEATURE_DEV_TOOLS").is_some()
        || std::env::var_os("CARGO_FEATURE_REAL").is_none()
    {
        println!("cargo:rustc-cfg=demo_core");
    }
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=EVOLITE_COMMIT={commit}");
    // A new commit or branch builds again.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");
    // The app icon in the executable (Windows only; elsewhere the window icon is enough).
    // Before `examples/app_icon.rs` has drawn the ICO there is nothing to embed.
    println!("cargo:rerun-if-changed=evolite.rc");
    println!("cargo:rerun-if-changed=setup.rc");
    println!("cargo:rerun-if-changed=setup.manifest");
    println!("cargo:rerun-if-changed=assets/app/evolite-app.ico");
    #[cfg(windows)]
    if std::path::Path::new("assets/app/evolite-app.ico").exists() {
        embed_resource::compile_for(
            "evolite.rc",
            ["evolite", "evolite-renderer"],
            embed_resource::NONE,
        )
        .manifest_optional()
        .expect("the icon resource");
        // The installer: the icon and a manifest that asks for no administrator rights.
        embed_resource::compile_for("setup.rc", ["evolite-setup"], embed_resource::NONE)
            .manifest_required()
            .expect("the installer's resources");
    }
}
