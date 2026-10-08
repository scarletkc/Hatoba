fn main() {
    let mut attributes = tauri_build::Attributes::new();

    // tauri-build links its Windows manifest into the app binary only, so the lib's unit-test exe
    // lacks the Common Controls v6 dependency and fails to load (`TaskDialogIndirect` missing,
    // STATUS_ENTRYPOINT_NOT_FOUND). On MSVC, embed the same manifest through the linker into every
    // target instead; see tauri-apps/tauri#13419.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());

        let manifest =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }

    embed_worker_bundle();
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}

/// Copies the Worker bundle from `scripts/worker/bundle.mjs` into OUT_DIR for `include_str!`, or
/// an empty file when it has not been generated, so builds without it still compile (spec §6.7).
fn embed_worker_bundle() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("gen")
        .join("worker");
    // Cargo reruns a build script on every build while a watched path is missing, so make sure
    // the directory exists and watch it rather than the file.
    std::fs::create_dir_all(&dir).expect("create gen/worker");
    println!("cargo:rerun-if-changed={}", dir.display());

    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR"))
        .join("worker-bundle.json");
    let bundle = std::fs::read_to_string(dir.join("bundle.json")).unwrap_or_default();
    std::fs::write(out, bundle).expect("write worker-bundle.json");
}
