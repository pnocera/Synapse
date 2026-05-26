use std::{env, fs::File, io::Write, path::PathBuf};

fn main() {
    let out_dir = env::var_os("OUT_DIR").unwrap_or_else(|| panic!("OUT_DIR is not set"));
    let out = PathBuf::from(out_dir);

    let mut memory = File::create(out.join("memory.x"))
        .unwrap_or_else(|err| panic!("failed to create linker memory.x: {err}"));
    memory
        .write_all(include_bytes!("memory.x"))
        .unwrap_or_else(|err| panic!("failed to write linker memory.x: {err}"));

    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rustc-link-arg-bins=-Tlink-rp.x");

    if env::var_os("CARGO_FEATURE_DEFMT").is_some() {
        println!("cargo:rustc-link-arg-bins=-Tdefmt.x");
    }
}
