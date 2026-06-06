use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let ld = manifest.join("user.ld");
    println!("cargo:rustc-link-arg=-T{}", ld.display());
    // Keep segments 4 KiB-granular (not the 64 KiB AArch64 default).
    println!("cargo:rustc-link-arg=-z");
    println!("cargo:rustc-link-arg=max-page-size=4096");
    println!("cargo:rerun-if-changed={}", ld.display());
}
