fn main() {
    println!("cargo:rerun-if-changed=src/ui");
    let out_dir = std::env::var("OUT_DIR").unwrap();
    elwindui_codegen::compile_dir("src/ui", &out_dir).expect("compiling .elwind sources");
}
