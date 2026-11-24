fn main() {
    println!("cargo:rerun-if-changed=../installer/rustify.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../installer/rustify.ico");
        res.set("FileDescription", "Rustify playback service");
        res.set("ProductName", "Rustify");
        res.compile().expect("embed icon");
    }
}
