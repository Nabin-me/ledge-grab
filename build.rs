fn main() {
    println!("cargo:rerun-if-changed=assets/signet.ico");
    let mut r = winresource::WindowsResource::new();
    r.set_icon("assets/signet.ico").set("ProductName", "Signet").set("FileDescription", "Signet");
    r.compile().expect("icon");
}
