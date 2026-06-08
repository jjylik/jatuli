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

    // Discover the programs from user/src/bin/*.rs (each file is a binary named
    // after its stem) and generate the kernel's program registry: a table of
    // (name, embedded ELF bytes). Adding a program is just dropping a file there.
    let bin_dir = user_dir.join("src").join("bin");
    let profile_dir = user_target.join("aarch64-unknown-none").join(&profile);

    let mut names: Vec<String> = std::fs::read_dir(&bin_dir)
        .expect("user/src/bin not found")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()?.to_str()? == "rs" {
                Some(path.file_stem()?.to_str()?.to_owned())
            } else {
                None
            }
        })
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no programs found in user/src/bin");

    let mut table = String::from("pub static PROGRAMS: &[(&str, &[u8])] = &[\n");
    for name in &names {
        let elf = profile_dir.join(name);
        table.push_str(&format!(
            "    ({:?}, include_bytes!({:?})),\n",
            name,
            elf.to_str().expect("non-UTF-8 ELF path"),
        ));
    }
    table.push_str("];\n");
    std::fs::write(out_dir.join("programs.rs"), table).expect("writing programs.rs");

    println!("cargo:rerun-if-changed={}", user_dir.join("src").display());
    println!("cargo:rerun-if-changed={}", bin_dir.display());
    // The user ELFs also embed the shared ABI crate.
    println!("cargo:rerun-if-changed={}", workspace.join("abi").join("src").display());
    println!("cargo:rerun-if-changed={}", user_dir.join("Cargo.toml").display());
    println!("cargo:rerun-if-changed={}", user_dir.join("user.ld").display());
    println!("cargo:rerun-if-changed={}", user_dir.join("build.rs").display());
}
