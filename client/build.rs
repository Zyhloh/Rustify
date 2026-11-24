fn main() {
    if std::env::var_os("CARGO_FEATURE_DEV").is_some() {
        unsafe { std::env::set_var("SLINT_LIVE_PREVIEW", "1") };
    }
    let config = slint_build::CompilerConfiguration::new().with_style("fluent-dark".into());
    slint_build::compile_with_config("ui/app.slint", config).unwrap();
}
