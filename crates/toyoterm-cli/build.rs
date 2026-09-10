use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=../../packaging/windows/toyoterm.rc");
    println!("cargo:rerun-if-changed=../../packaging/app-icon.ico");
    println!("cargo:rerun-if-changed=../../vendor/conpty/win-x64/conpty.dll");
    println!("cargo:rerun-if-changed=../../vendor/conpty/win-x64/OpenConsole.exe");
    embed_resource::compile("../../packaging/windows/toyoterm.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86_64")
    {
        copy_conpty_bundle();
    }
}

fn copy_conpty_bundle() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let source = manifest_dir.join("../../vendor/conpty/win-x64");
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let binary_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("Cargo OUT_DIR must be under the target profile directory");

    for name in ["conpty.dll", "OpenConsole.exe"] {
        copy_if_changed(&source.join(name), &binary_dir.join(name));
    }
}

fn copy_if_changed(source: &Path, destination: &Path) {
    let unchanged = std::fs::read(source)
        .ok()
        .zip(std::fs::read(destination).ok())
        .is_some_and(|(source, destination)| source == destination);
    if !unchanged {
        std::fs::copy(source, destination).unwrap_or_else(|error| {
            panic!(
                "copy {} to {}: {error}",
                source.display(),
                destination.display()
            )
        });
    }
}
