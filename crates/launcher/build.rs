fn main() {
    println!("cargo:rerun-if-changed=../app/assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("../app/assets/icon.ico");
        resource.compile().expect("embed the Windows icon");
    }
}
