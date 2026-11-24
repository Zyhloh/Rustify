fn main() {
    if std::env::var_os("CARGO_FEATURE_DEV").is_some() {
        unsafe { std::env::set_var("SLINT_LIVE_PREVIEW", "1") };
    }
    let config = slint_build::CompilerConfiguration::new().with_style("fluent-dark".into());
    slint_build::compile_with_config("ui/app.slint", config).unwrap();
    windows_icon("Rustify");
}

fn windows_icon(description: &str) {
    println!("cargo:rerun-if-changed=../installer/rustify.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../installer/rustify.ico");
        res.set("FileDescription", description);
        res.set("ProductName", "Rustify");
        res.compile().expect("embed icon");
    }
}
