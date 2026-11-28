fn main() {
    // The icon resource only exists for Windows builds.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_icon();
    }
}

#[cfg(not(target_os = "linux"))]
fn windows_icon() {
    println!("cargo:rerun-if-changed=assets/ledge-grab.ico");
    let mut r = winresource::WindowsResource::new();
    r.set_icon("assets/ledge-grab.ico")
        .set("ProductName", "Ledge Grab")
        .set("FileDescription", "Ledge Grab — Screenshots that live on the ledge.");
    r.compile().expect("icon");
}

#[cfg(target_os = "linux")]
fn windows_icon() {}
