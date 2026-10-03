fn main() {
    let vendor = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/rubberband");
    println!("cargo:rerun-if-changed={}", vendor.display());
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file(vendor.join("single/RubberBandSingle.cpp"))
        .include(&vendor)
        .opt_level(3)
        .warnings(false)
        .flag_if_supported("-std=c++14")
        .flag_if_supported("/std:c++14");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        build.define("_USE_MATH_DEFINES", None).define("NOMINMAX", None);
    }
    let windows_gnu = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu");
    if windows_gnu {
        build.cpp_link_stdlib(None);
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // The single-file build picks vDSP for its FFT on Apple, which lives in Accelerate.
        println!("cargo:rustc-link-lib=framework=Accelerate");
    }
    build.compile("rubberband");
    if windows_gnu {
        let found = std::process::Command::new(build.get_compiler().path()).arg("-print-file-name=libstdc++.a").output().ok();
        if let Some(dir) = found.and_then(|out| String::from_utf8(out.stdout).ok()).map(|path| std::path::PathBuf::from(path.trim())).and_then(|path| path.parent().map(std::path::Path::to_path_buf)) {
            println!("cargo:rustc-link-search=native={}", dir.display());
        }
        println!("cargo:rustc-link-lib=static=stdc++");
    }
}
