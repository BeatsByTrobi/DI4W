//! Embeds the icon, the manifest (per-monitor DPI awareness v2, Common Controls 6) and the version information into
//! DynamicIsland.exe. The version matters for updates from 1.x: their updater checks the new DynamicIsland.exe's
//! ProductVersion after copying, and restores the old files when it doesn't match.

use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let assets = manifest_dir.join("assets");
    println!("cargo:rerun-if-changed=assets/island.ico");
    println!("cargo:rerun-if-changed=assets/island.manifest");

    // "1.5.2" -> 1,5,2,0 for the numeric version fields.
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let mut numbers: Vec<String> = version.split(['.', '-']).take(3).map(|p| p.parse::<u16>().unwrap_or(0).to_string()).collect();
    numbers.resize(4, "0".into());
    let numeric = numbers.join(",");
    let path = |name: &str| assets.join(name).display().to_string().replace('\\', "\\\\");
    // Icon resource 1 is also the tray icon.
    let rc = format!(
        r#"1 ICON "{icon}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "aangeraakt"
      VALUE "FileDescription", "Dynamic Island"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "DynamicIsland"
      VALUE "LegalCopyright", "MIT License"
      VALUE "OriginalFilename", "DynamicIsland.exe"
      VALUE "ProductName", "Dynamic Island for Windows"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = path("island.ico"),
        manifest = path("island.manifest"),
    );
    let rc_path = out_dir.join("island.rc");
    std::fs::write(&rc_path, rc).unwrap();
    embed_resource::compile(&rc_path, embed_resource::NONE).manifest_required().unwrap();
}
