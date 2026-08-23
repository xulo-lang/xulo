//! Builds the `xulo-ui-wasm` layout engine and embeds its bytes into the
//! framework (used by the webview backend). Only runs when the `webview`
//! feature is enabled; the webview backend requires the wasm32 target.

use std::path::PathBuf;
use std::process::Command;

use base64::Engine as _;

fn main() {
    if std::env::var("CARGO_FEATURE_WEBVIEW").is_err() {
        return;
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    // crates/xulo-framework -> workspace root
    let workspace = manifest.parent().and_then(|p| p.parent()).unwrap();
    let wasm_manifest = workspace.join("crates/xulo-ui-wasm/Cargo.toml");
    let wasm_src = workspace.join("crates/xulo-ui-wasm/src/lib.rs");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let nested = out_dir.join("wasm-target");
    let wasm_path = nested.join("wasm32-unknown-unknown/release/xulo_ui_wasm.wasm");

    // Build the wasm engine. A nested target dir avoids the outer cargo's lock.
    let status = Command::new("cargo")
        .env("CARGO_NET_OFFLINE", "true")
        .args([
            "build",
            "--manifest-path",
            wasm_manifest.to_str().unwrap(),
            "--target",
            "wasm32-unknown-unknown",
            "--release",
            "--target-dir",
            nested.to_str().unwrap(),
        ])
        .status();
    if !matches!(status, Ok(s) if s.success()) {
        panic!(
            "xulo-framework: building xulo-ui-wasm failed (needed for --render webview). \
             Ensure the wasm32 target is installed: `rustup target add wasm32-unknown-unknown`."
        );
    }

    let wasm = std::fs::read(&wasm_path).expect("wasm artifact");
    let b64 = base64::engine::general_purpose::STANDARD.encode(&wasm);
    std::fs::write(out_dir.join("xulo_wasm.b64"), b64).expect("write wasm b64");

    println!("cargo:rerun-if-changed={}", wasm_manifest.display());
    println!("cargo:rerun-if-changed={}", wasm_src.display());
}
