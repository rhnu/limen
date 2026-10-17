fn main() {
    // include_dir tracks existing files; Cargo must also notice new asset names.
    println!("cargo::rerun-if-changed=web/dist");
}
