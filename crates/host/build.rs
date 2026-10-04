fn main() {
    println!("cargo:rerun-if-changed=host.rc");
    println!("cargo:rerun-if-changed=host.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("cargo gives an out dir"));
    let object = out.join("host.o");
    let windres = if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
        "x86_64-w64-mingw32-windres"
    } else {
        "rc.exe"
    };
    let made = std::process::Command::new(windres)
        .args(["host.rc", "-O", "coff", "-o"])
        .arg(&object)
        .status();
    match made {
        Ok(status) if status.success() => {
            println!("cargo:rustc-link-arg-bins={}", object.display());
        }
        _ => println!("cargo:warning=the plugin host was built without its manifest, so its controls will look like Windows 95"),
    }
}
