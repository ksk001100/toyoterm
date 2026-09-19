fn main() {
    println!("cargo:rerun-if-changed=../../vendor/mruby/mruby.c");
    println!("cargo:rerun-if-changed=../../vendor/mruby/mruby-windows.c");
    println!("cargo:rerun-if-changed=../../vendor/mruby/mruby-windows.h");
    println!("cargo:rerun-if-changed=../../vendor/mruby/mruby.h");
    println!("cargo:rerun-if-changed=src/script/shim.c");

    let mruby_source = if std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        "../../vendor/mruby/mruby-windows.c"
    } else {
        "../../vendor/mruby/mruby.c"
    };
    cc::Build::new()
        .file(mruby_source)
        .file("src/script/shim.c")
        .include("../../vendor/mruby")
        .warnings(false)
        .compile("toyoterm_mruby");

    if std::env::var("CARGO_CFG_UNIX").is_ok() {
        println!("cargo:rustc-link-lib=m");
    }
    if std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        println!("cargo:rustc-link-lib=ws2_32");
    }
}
