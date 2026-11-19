//! Embeds the icon and the manifest (per-monitor DPI awareness v2, Common Controls 6) into DynamicIsland.exe.
fn main() {
    println!("cargo:rerun-if-changed=assets/island.rc");
    println!("cargo:rerun-if-changed=assets/island.ico");
    println!("cargo:rerun-if-changed=assets/island.manifest");
    embed_resource::compile("assets/island.rc", embed_resource::NONE).manifest_required().unwrap();
}
