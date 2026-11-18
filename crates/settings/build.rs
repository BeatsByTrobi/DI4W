//! Compiles the .slint UI (Fluent style) and embeds the icon, manifest (per-monitor DPI, Common Controls 6) and
//! version information into DynamicIsland.Settings.exe.

use std::path::PathBuf;

fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("the .slint files compile");

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let assets = manifest_dir.join("assets");
    println!("cargo:rerun-if-changed=assets/island.ico");
    println!("cargo:rerun-if-changed=assets/settings.manifest");

    // "1.5.2" -> 1,5,2,0 for the numeric version fields.
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let mut numbers: Vec<String> = version.split(['.', '-']).take(3).map(|p| p.parse::<u16>().unwrap_or(0).to_string()).collect();
    numbers.resize(4, "0".into());
    let numeric = numbers.join(",");
    let path = |name: &str| assets.join(name).display().to_string().replace('\\', "\\\\");
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
      VALUE "FileDescription", "Dynamic Island Settings"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "DynamicIsland.Settings"
      VALUE "LegalCopyright", "MIT License"
      VALUE "OriginalFilename", "DynamicIsland.Settings.exe"
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
        manifest = path("settings.manifest"),
    );
    let rc_path = out_dir.join("settings.rc");
    std::fs::write(&rc_path, rc).unwrap();
    embed_resource::compile(&rc_path, embed_resource::NONE).manifest_required().unwrap();
}
