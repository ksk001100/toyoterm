fn main() {
    println!("cargo:rerun-if-changed=../../packaging/windows/toyoterm.rc");
    println!("cargo:rerun-if-changed=../../packaging/app-icon.ico");
    embed_resource::compile("../../packaging/windows/toyoterm.rc", embed_resource::NONE);
}
