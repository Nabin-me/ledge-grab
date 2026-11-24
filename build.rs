fn main() {
    println!("cargo:rerun-if-changed=assets/ledge-grab.ico");
    let mut r = winresource::WindowsResource::new();
    r.set_icon("assets/ledge-grab.ico")
        .set("ProductName", "Ledge Grab")
        .set("FileDescription", "Ledge Grab — Screenshots that live on the ledge.");
    r.compile().expect("icon");
}
