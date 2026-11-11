use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let easter_egg = manifest_dir.join("../../crates/ui/ui/views/simulator_easter_egg.slint");
    let config = slint_build::CompilerConfiguration::new()
        .with_library_paths([("simulator-easter-egg".into(), easter_egg)].into());
    slint_build::compile_with_config("ui/app.slint", config).unwrap();
}
