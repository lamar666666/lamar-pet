use image::ImageEncoder;
use std::{fs::File, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=assets/lamar-cat.png");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // GPUI's composed widget trees can exceed Windows' default 1 MiB main
    // stack in unoptimized builds. Pages commit on demand, not up front.
    println!("cargo:rustc-link-arg-bin=lamar-pet=/STACK:8388608");
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
    let major = std::env::var("CARGO_PKG_VERSION_MAJOR").expect("major version");
    let minor = std::env::var("CARGO_PKG_VERSION_MINOR").expect("minor version");
    let patch = std::env::var("CARGO_PKG_VERSION_PATCH").expect("patch version");
    let resource = format!(
        r#"
1 ICON "{}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
 BLOCK "StringFileInfo"
 BEGIN
  BLOCK "040904E4"
  BEGIN
   VALUE "FileDescription", "Lamar Pet\0"
   VALUE "ProductName", "Lamar Pet\0"
   VALUE "ProductVersion", "{major}.{minor}.{patch}\0"
   VALUE "FileVersion", "{major}.{minor}.{patch}.0\0"
   VALUE "InternalName", "lamar-pet\0"
   VALUE "CompanyName", "lamar\0"
   VALUE "OriginalFilename", "lamar-pet.exe\0"
   VALUE "LegalCopyright", "Copyright (c) 2026 lamar\0"
  END
 END
 BLOCK "VarFileInfo"
 BEGIN
  VALUE "Translation", 0x0409, 1252
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
