fn main() {
    println!("cargo:rerun-if-changed=recompiled");
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file("recompiled/bridge.cpp")
        .compile("recompiled_brain");
}
