fn main() {
    println!("cargo:rerun-if-changed=govn.rc");
    println!("cargo:rerun-if-changed=govn.manifest");
    embed_resource::compile("govn.rc", embed_resource::NONE).manifest_required().unwrap();
}
