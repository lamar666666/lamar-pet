use image::ImageEncoder;
use std::{fs::File, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=assets/lamar-cat.png");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output directory"));
    let sheet = image::load_from_memory(include_bytes!("assets/lamar-cat.png"))
        .expect("sprite PNG")
        .into_rgba8();
    let frame =
        image::imageops::crop_imm(&sheet, 0, 0, sheet.width() / 3, sheet.height() / 3).to_image();
    let icon = image::imageops::resize(&frame, 64, 64, image::imageops::FilterType::Nearest);
    let icon_path = out.join("lamar-pet.ico");
    image::codecs::ico::IcoEncoder::new(File::create(&icon_path).expect("icon output"))
        .write_image(&icon, 64, 64, image::ExtendedColorType::Rgba8)
        .expect("icon encoding");
    let resource = format!(
        r#"
1 ICON "{}"
1 VERSIONINFO
FILEVERSION 0,1,0,0
PRODUCTVERSION 0,1,0,0
BEGIN
 BLOCK "StringFileInfo"
 BEGIN
  BLOCK "040904B0"
  BEGIN
   VALUE "FileDescription", "Lamar Pet\0"
   VALUE "ProductName", "Lamar Pet\0"
   VALUE "ProductVersion", "0.1.0\0"
   VALUE "OriginalFilename", "lamar-pet.exe\0"
   VALUE "LegalCopyright", "Copyright (c) 2026 lamar\0"
  END
 END
 BLOCK "VarFileInfo"
 BEGIN
  VALUE "Translation", 0x0409, 1200
 END
END
"#,
        icon_path.to_string_lossy().replace('\\', "/")
    );
    let resource_path = out.join("lamar-pet.rc");
    std::fs::write(&resource_path, resource).expect("resource source");
    embed_resource::compile(&resource_path, embed_resource::NONE)
        .manifest_required()
        .expect("Windows resources");
}
