fn main() {
    println!("cargo:rerun-if-changed=android/tgvoip/tgvoip_shim.cpp");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
        let abi = if arch == "aarch64" {
            "arm64-v8a"
        } else {
            "x86_64"
        };
        let dir = format!(
            "{}/android/app/src/main/jniLibs/{}",
            std::env::var("CARGO_MANIFEST_DIR").unwrap(),
            abi
        );
        println!("cargo:rustc-link-search=native={dir}");
        println!("cargo:rustc-link-lib=dylib=tgvoip");
    }
}
