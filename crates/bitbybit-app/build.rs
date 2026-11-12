fn main() {
    // Fluent std widgets (native to Windows 10/11; cyan accent in dark mode); colours come from ui/theme.slint.
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", config)
        .unwrap_or_else(|e| panic!("Slint UI failed to compile: {e}"));

    // The bitBybit logo as the executable's icon (Explorer, Start menu shortcut). Only when building on Windows:
    // type-checking the Windows target from Linux has no resource compiler.
    if cfg!(windows) && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=app.rc");
        println!("cargo:rerun-if-changed=app.ico");
        embed_resource::compile("app.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap_or_else(|e| panic!("Icon resource failed to compile: {e}"));
    }
}
