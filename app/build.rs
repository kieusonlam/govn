fn main() {
    println!("cargo:rerun-if-changed=govn.rc");
    println!("cargo:rerun-if-changed=govn.manifest");
    embed_resource::compile("govn.rc", embed_resource::NONE).manifest_required().unwrap();
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/settings.slint", config).unwrap();
}
