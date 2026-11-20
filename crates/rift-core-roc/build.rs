//! Builds the Roc core (`roc/core/main.roc` on `roc/platform`) into a static archive with the
//! pinned Roc nightly, links it, and generates the Rust glue for the platform's boundary types
//! (`roc glue`, into `OUT_DIR`; never committed). `ROC` names the compiler; the default is the
//! pinned nightly under `~/.local/share/roc-nightly/current`. `ROC_SRC` names a checkout of
//! roc-lang/roc at the same commit, for the glue spec (`src/glue/src/RustGlue.roc`, which the
//! nightly tarball lacks); the default is `~/.local/share/roc-nightly/src`. Only with the `roc`
//! feature. Each run states how long the Roc build and the glue took, so a slow build is
//! explainable.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

fn main() {
    // Without `roc` the crate is empty (see Cargo.toml): nothing to build or link.
    if std::env::var_os("CARGO_FEATURE_ROC").is_none() {
        return;
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let roc_dir = manifest.join("../../roc");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let home = std::env::var("HOME").unwrap();
    let roc = std::env::var("ROC")
        .unwrap_or_else(|_| format!("{home}/.local/share/roc-nightly/current/roc"));
    let roc_src =
        std::env::var("ROC_SRC").unwrap_or_else(|_| format!("{home}/.local/share/roc-nightly/src"));
    // The directories, not a list of modules: cargo rebuilds when any file in them changes, so
    // a new module can't be left out (Scrolling.roc once was, and a stale core was tested).
    for dir in ["core", "platform"] {
        println!("cargo:rerun-if-changed={}", roc_dir.join(dir).display());
    }
    println!("cargo:rerun-if-env-changed=ROC");
    println!("cargo:rerun-if-env-changed=ROC_SRC");

    let started = Instant::now();
    let archive = out.join("librift_core_roc.a");
    let status = Command::new(&roc)
        .current_dir(&roc_dir)
        .args(["build", "--target=arm64mac", "--opt=speed"])
        .arg(format!("--output={}", archive.display()))
        .arg("core/main.roc")
        .status()
        .unwrap_or_else(|error| panic!("could not run {roc}: {error}"));
    assert!(status.success(), "roc build failed");
    let built = started.elapsed();

    let started = Instant::now();
    glue(&roc, Path::new(&roc_src), &roc_dir, &out);
    let glued = started.elapsed();
    println!(
        "cargo:warning=rift-core-roc: roc build {:.1} s, roc glue {:.1} s",
        built.as_secs_f64(),
        glued.as_secs_f64()
    );

    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=rift_core_roc");
}

/// Generates the platform's glue into `out/roc_platform_abi.rs`, ready for `include!`: the
/// generator writes a file meant to be a module (inner doc comments and attributes), which
/// `include!` rejects, so those lines become plain comments (the crate puts the attributes
/// it needs on the module). Fails on a boundary type the generator had to name itself: an
/// anonymous record (`AnonStruct<hash>`) or a structural union it numbered (`…Type<n>`), whose
/// names change as other types come and go. Every boundary type is a named Roc type.
fn glue(roc: &str, roc_src: &Path, roc_dir: &Path, out: &Path) {
    let spec = roc_src.join("src/glue/src/RustGlue.roc");
    assert!(
        spec.is_file(),
        "roc glue needs RustGlue.roc from a roc-lang/roc checkout at the pinned nightly's commit \
         (roc/NIGHTLY): set ROC_SRC to it (looked for {})",
        spec.display()
    );
    let generated = out.join("glue");
    let status = Command::new(roc)
        .current_dir(roc_dir)
        .arg("glue")
        .arg(&spec)
        .arg(&generated)
        .arg("platform/main.roc")
        .status()
        .unwrap_or_else(|error| panic!("could not run {roc} glue: {error}"));
    assert!(status.success(), "roc glue failed");
    let text = std::fs::read_to_string(generated.join("roc_platform_abi.rs"))
        .expect("roc glue wrote roc_platform_abi.rs");
    let unnamed = unnamed_types(&text);
    assert!(
        unnamed.is_empty(),
        "the platform's boundary has types without a name of their own (give each a named Roc \
         type): {unnamed:?}"
    );
    let includable: String = text
        .lines()
        .map(|line| match line.strip_prefix("//!") {
            Some(doc) => format!("//{doc}\n"),
            None if line.starts_with("#![") => format!("// {line}\n"),
            None => format!("{line}\n"),
        })
        .collect();
    std::fs::write(out.join("roc_platform_abi.rs"), includable).expect("glue written");
}

/// The generator's own names in `text`: `AnonStruct` followed by hex digits, and identifiers
/// with `Type` followed by digits after their start (`AskedOrUnaskedType24`, `TypesFactType26`).
fn unnamed_types(text: &str) -> Vec<String> {
    let numbered = |word: &str| {
        word.match_indices("Type")
            .any(|(at, _)| at > 0 && word[at + 4..].starts_with(|c: char| c.is_ascii_digit()))
    };
    let anonymous = |word: &str| {
        word.strip_prefix("AnonStruct")
            .is_some_and(|hash| !hash.is_empty() && hash.chars().all(|c| c.is_ascii_hexdigit()))
    };
    let mut found: Vec<String> = text
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|word| anonymous(word) || numbered(word))
        .map(str::to_owned)
        .collect();
    found.sort();
    found.dedup();
    found
}
