//! Builds the Roc core (`roc/core/main.roc` on `roc/platform`) into a static archive with the
//! pinned Roc nightly and links it. `ROC` names the compiler; the default is the pinned nightly
//! under `~/.local/share/roc-nightly/current`.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let roc_dir = manifest.join("../../roc");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let roc = std::env::var("ROC").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap();
        format!("{home}/.local/share/roc-nightly/current/roc")
    });
    for file in [
        "core/main.roc",
        "platform/main.roc",
        "platform/Event.roc",
        "platform/Effect.roc",
    ] {
        println!("cargo:rerun-if-changed={}", roc_dir.join(file).display());
    }
    println!("cargo:rerun-if-env-changed=ROC");
    let archive = out.join("librift_core_roc.a");
    let status = Command::new(&roc)
        .current_dir(&roc_dir)
        .args(["build", "--target=arm64mac", "--opt=speed"])
        .arg(format!("--output={}", archive.display()))
        .arg("core/main.roc")
        .status()
        .unwrap_or_else(|error| panic!("could not run {roc}: {error}"));
    assert!(status.success(), "roc build failed");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=rift_core_roc");
}
