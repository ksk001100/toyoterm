fn main() {
    println!("cargo:rerun-if-changed=../../vendor/mruby/mruby.c");
    println!("cargo:rerun-if-changed=../../vendor/mruby/mruby.h");
    println!("cargo:rerun-if-changed=src/script/shim.c");

    cc::Build::new()
        .file("../../vendor/mruby/mruby.c")
        .file("src/script/shim.c")
        .include("../../vendor/mruby")
        .warnings(false)
        .compile("toyoterm_mruby");

    if std::env::var("CARGO_CFG_UNIX").is_ok() {
        println!("cargo:rustc-link-lib=m");
    }
}
