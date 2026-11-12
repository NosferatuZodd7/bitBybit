fn main() {
    // Cupertino std widgets match the Liquid Glass look; colours come from ui/theme.slint.
    let config = slint_build::CompilerConfiguration::new().with_style("cupertino".into());
    slint_build::compile_with_config("ui/app.slint", config)
        .unwrap_or_else(|e| panic!("Slint UI failed to compile: {e}"));
}
