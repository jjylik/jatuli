use std::path::PathBuf;

fn main() {
    // Absolute path so the linker resolves it regardless of its working directory.
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let ld = manifest.join("linker.ld");
    println!("cargo:rustc-link-arg=-T{}", ld.display());
    println!("cargo:rerun-if-changed={}", ld.display());
}
