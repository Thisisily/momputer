use std::path::PathBuf;

fn main() {
    // Tauri bundles the `mom` CLI next to the app (it's what agents and hooks call). The real
    // release build copies a freshly built mom.exe here first; for everyday `cargo build`, use
    // whatever mom.exe the workspace already built, or a placeholder so the app still builds.
    let target = std::env::var("TARGET").unwrap_or_else(|_| "x86_64-pc-windows-msvc".into());
    let ext = if target.contains("windows") { ".exe" } else { "" };
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let sidecar = manifest.join("binaries").join(format!("mom-{target}{ext}"));
    if !sidecar.exists() {
        let _ = std::fs::create_dir_all(sidecar.parent().unwrap());
        let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".into());
        let built = manifest.join("..").join("..").join("target").join(&profile).join(format!("mom{ext}"));
        if std::fs::copy(&built, &sidecar).is_err() {
            println!("cargo:warning=mom CLI not built yet; bundling a placeholder. Run `cargo build -p mom-cli` (or scripts/build.ps1) for a real one.");
            let _ = std::fs::write(&sidecar, b"");
        }
    }
    tauri_build::build()
}
