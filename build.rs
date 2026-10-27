fn main() {
    embed_resource::compile("assets/app.rc", embed_resource::NONE)
        .manifest_required()
        .unwrap();
    cc::Build::new()
        .cpp(true)
        .file("src/xasr_bridge.cpp")
        .include("vendor/runtime/sherpa-onnx-v1.13.8-win-x64-shared-MT-Release-no-tts/include")
        .flag_if_supported("/std:c++17")
        .flag_if_supported("/utf-8")
        .flag_if_supported("/EHsc")
        .static_crt(true)
        .warnings(true)
        .compile("xasr_bridge");
    println!("cargo:rerun-if-changed=src/xasr_bridge.cpp");
    println!(
        "cargo:rerun-if-changed=vendor/runtime/sherpa-onnx-v1.13.8-win-x64-shared-MT-Release-no-tts/include/sherpa-onnx/c-api/c-api.h"
    );
    println!("cargo:rerun-if-changed=assets/app.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");
    println!("cargo:rerun-if-changed=assets/app.rc");
}
