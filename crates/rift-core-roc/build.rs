//! Builds the Roc core (`roc/core/main.roc` on `roc/platform`) into a static archive with the
//! pinned Roc nightly (`roc/NIGHTLY`), links it, and generates the Rust glue for the platform's
//! boundary types (`roc glue`, into `OUT_DIR`; never committed). `ROC` names the compiler; the
//! default is the pinned nightly under `~/.local/share/roc-nightly/current`. `ROC_SRC` names
//! roc-lang/roc's source at the same commit, for the glue spec (`src/glue/src/RustGlue.roc`, in
//! the nightly's source tarball, not its compiler tarball); the default is
//! `~/.local/share/roc-nightly/src-current`. `roc/bump.sh` points both at the pinned nightly.
//! Only with the `roc` feature. Each run states how long the Roc build and the glue took, so a
//! slow build is explainable.
//!
//! The archive and the glue are built once per input and shared: every build-script run (each
//! feature set, `check`, `test`, clippy: each has its own `OUT_DIR`) keys them by a hash of
//! everything that goes into them (each Roc source file's path and bytes, the compiler's path,
//! size and mtime, the glue spec's bytes, the build arguments), keeps them under the target
//! directory (`roc-build/<key>/`), and only links its `OUT_DIR` copy to them. An optimized Roc
//! build takes about a minute and has no incremental reuse of its own (LLVM runs on the whole
//! program every time), so the same sources are never built twice; and an `OUT_DIR` copy is
//! rewritten only when its bytes change, so nothing downstream relinks for an unchanged core.

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
    let roc_src = std::env::var("ROC_SRC")
        .unwrap_or_else(|_| format!("{home}/.local/share/roc-nightly/src-current"));
    // The directories, not a list of modules: cargo rebuilds when any file in them changes, so
    // a new module can't be left out (Scrolling.roc once was, and a stale core was tested).
    for dir in ["core", "platform"] {
        println!("cargo:rerun-if-changed={}", roc_dir.join(dir).display());
    }
    // The compiler itself (a new nightly behind the same path is a new build).
    println!("cargo:rerun-if-changed={roc}");
    println!("cargo:rerun-if-env-changed=ROC");
    println!("cargo:rerun-if-env-changed=ROC_SRC");

    // Every build is optimized: the dev backend runs a different program (docs/roc-fork.md, "The
    // dev backend"). `ROC_OPT=dev` exists only for `roc/bump.sh`'s probe of whether it still does.
    println!("cargo:rerun-if-env-changed=ROC_OPT");
    let opt = match std::env::var("ROC_OPT").as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("speed") => "--opt=speed",
        Ok("dev") => "--opt=dev",
        other => panic!("ROC_OPT is speed or dev, not {other:?}"),
    };
    let build = ["build", "--target=arm64mac", opt];
    let spec = Path::new(&roc_src).join("src/glue/src/RustGlue.roc");
    let key = inputs_key(&roc_dir, &roc, &spec, &build);
    // The target directory: `OUT_DIR` is `<target>/<profile>/build/<unit>/out`.
    let target = out.ancestors().nth(4).expect("OUT_DIR is under the target directory");
    let shared = target.join("roc-build").join(format!("{key:016x}"));
    let (archive, abi) = (
        shared.join("librift_core_roc.a"),
        shared.join("roc_platform_abi.rs"),
    );

    if archive.is_file() && abi.is_file() {
        println!("cargo:warning=rift-core-roc: roc build and glue reused ({key:016x})");
    } else {
        let work = shared.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::create_dir_all(&work).expect("a build directory");
        let started = Instant::now();
        let status = Command::new(&roc)
            .current_dir(&roc_dir)
            .args(build)
            .arg(format!("--output={}", work.join("librift_core_roc.a").display()))
            .arg("core/main.roc")
            .status()
            .unwrap_or_else(|error| panic!("could not run {roc}: {error}"));
        assert!(status.success(), "roc build failed");
        let built = started.elapsed();
        let started = Instant::now();
        glue(&roc, &spec, &roc_dir, &work);
        let glued = started.elapsed();
        // Published whole: a concurrent run that built the same key keeps its own (equal) one.
        if std::fs::rename(&work, &shared).is_err() {
            let _ = std::fs::remove_dir_all(&work);
        }
        println!(
            "cargo:warning=rift-core-roc: roc build {:.1} s, roc glue {:.1} s ({key:016x})",
            built.as_secs_f64(),
            glued.as_secs_f64()
        );
    }
    copy_if_changed(&archive, &out.join("librift_core_roc.a"));
    copy_if_changed(&abi, &out.join("roc_platform_abi.rs"));

    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=rift_core_roc");
}

/// Generates the platform's glue into `out/roc_platform_abi.rs`, which the crate `include!`s as
/// written (the generator's output is includable and lint-clean since roc-lang/roc#13349). Fails
/// on a boundary type the generator had to name itself: an
/// anonymous record (`AnonStruct<hash>`) or a structural union it numbered (`…Type<n>`), whose
/// names change as other types come and go. Every boundary type is a named Roc type.
fn glue(roc: &str, spec: &Path, roc_dir: &Path, out: &Path) {
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
        .arg(spec)
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
    std::fs::write(out.join("roc_platform_abi.rs"), text).expect("glue written");
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

/// `from`'s bytes at `to`, written only when they differ (so `to` keeps its mtime, and cargo
/// relinks nothing, when the build is unchanged).
fn copy_if_changed(from: &Path, to: &Path) {
    let bytes = std::fs::read(from).expect("the shared build's file");
    if std::fs::read(to).ok().as_deref() == Some(&bytes[..]) {
        return;
    }
    let staged = to.with_extension("tmp");
    std::fs::write(&staged, &bytes).expect("a copy staged");
    std::fs::rename(&staged, to).expect("the copy in place");
}

/// A key for everything a build of the core reads: each file under `roc/` (path and bytes, in
/// path order), the compiler (path, size, mtime: a new nightly is a new key), the glue spec's
/// bytes, and the build arguments. FNV-1a, so the key is the same in every run.
fn inputs_key(roc_dir: &Path, roc: &str, spec: &Path, args: &[&str]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for &byte in bytes.iter().chain(&[0xff]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    let mut files = Vec::new();
    for dir in ["core", "platform"] {
        collect(&roc_dir.join(dir), &mut files);
    }
    files.sort();
    for file in &files {
        feed(file.strip_prefix(roc_dir).unwrap_or(file).to_string_lossy().as_bytes());
        feed(&std::fs::read(file).expect("a Roc source"));
    }
    let compiler = std::fs::metadata(roc).unwrap_or_else(|error| panic!("{roc}: {error}"));
    let mtime = compiler
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
    feed(roc.as_bytes());
    feed(&compiler.len().to_le_bytes());
    feed(&mtime.map_or(0, |t| t.as_nanos()).to_le_bytes());
    feed(&std::fs::read(spec).unwrap_or_default());
    for arg in args {
        feed(arg.as_bytes());
    }
    hash
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("a Roc source directory").flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, files);
        } else {
            files.push(path);
        }
    }
}
