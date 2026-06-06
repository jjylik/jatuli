use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());

    // Kernel linker script (absolute path).
    let ld = manifest.join("linker.ld");
    println!("cargo:rustc-link-arg=-T{}", ld.display());
    println!("cargo:rerun-if-changed={}", ld.display());

    // Build the userspace crate into an ISOLATED target dir so this nested
    // `cargo` invocation does not deadlock on the parent build's locked target dir.
    let workspace = manifest.parent().unwrap().to_path_buf();
    let user_dir = workspace.join("user");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let user_target = out_dir.join("user-target");
    let profile = std::env::var("PROFILE").unwrap(); // "debug" or "release"

    let mut cmd = Command::new(std::env::var("CARGO").unwrap());
    cmd.current_dir(&workspace)
        .arg("build")
        .arg("-p")
        .arg("user")
        .arg("--target")
        .arg("aarch64-unknown-none")
        .env("CARGO_TARGET_DIR", &user_target);
    if profile == "release" {
        cmd.arg("--release");
    }
    let status = cmd.status().expect("failed to spawn cargo for the user crate");
    assert!(status.success(), "building the user crate failed");

    let elf = user_target
        .join("aarch64-unknown-none")
        .join(&profile)
        .join("user");
    println!("cargo:rustc-env=USER_ELF={}", elf.display());

    println!("cargo:rerun-if-changed={}", user_dir.join("src").display());
    println!("cargo:rerun-if-changed={}", user_dir.join("Cargo.toml").display());
    println!("cargo:rerun-if-changed={}", user_dir.join("user.ld").display());
    println!("cargo:rerun-if-changed={}", user_dir.join("build.rs").display());
}
