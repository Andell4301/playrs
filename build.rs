use prost_build::Config;
use protoc_bin_vendored::protoc_bin_path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=res/protos/AcquireApp.proto");
    println!("cargo:rerun-if-changed=res/protos/GooglePlay.proto");

    let mut config = Config::new();
    config.default_package_filename("playproto");
    config.out_dir(&"src");
    config.protoc_executable(protoc_bin_path()?);

    config.type_attribute(".", "#[allow(dead_code)]");
    config.type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]");
    config.compile_protos(&["res/protos/AcquireApp.proto", "res/protos/GooglePlay.proto"], &["res/protos"])?;

    Ok(())
}
