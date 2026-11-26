//! Gives the Windows executable its icon and version details. Does nothing for other targets.

use std::error::Error;
use std::path::PathBuf;
use std::{env, fs};

use embed_resource::CompilationResult;

/// Set in release builds: fail instead of shipping an executable without an icon.
const REQUIRE: &str = "MQTT_VIEWER_REQUIRE_RESOURCES";

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=../../assets/icon.ico");
    println!("cargo::rerun-if-env-changed={REQUIRE}");
    if env::var("CARGO_CFG_TARGET_OS")? != "windows" {
        return Ok(());
    }

    // The script is generated so that it needs no preprocessor and always carries the crate's version.
    let icon = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("../../assets/icon.ico");
    let script = format!(
        r#"1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "ProductName", "MQTT Viewer"
            VALUE "FileDescription", "MQTT Viewer"
            VALUE "CompanyName", "{authors}"
            VALUE "LegalCopyright", "{license} licensed"
            VALUE "OriginalFilename", "mqtt-viewer.exe"
            VALUE "FileVersion", "{version}"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#,
        icon = icon.display().to_string().replace('\\', "/"),
        major = env::var("CARGO_PKG_VERSION_MAJOR")?,
        minor = env::var("CARGO_PKG_VERSION_MINOR")?,
        patch = env::var("CARGO_PKG_VERSION_PATCH")?,
        version = env::var("CARGO_PKG_VERSION")?,
        authors = env::var("CARGO_PKG_AUTHORS")?,
        license = env::var("CARGO_PKG_LICENSE")?,
    );
    let path = PathBuf::from(env::var("OUT_DIR")?).join("mqtt-viewer.rc");
    fs::write(&path, script)?;

    match embed_resource::compile(&path, embed_resource::NONE) {
        // A contributor without a resource compiler still gets a working program.
        CompilationResult::NotAttempted(reason) if env::var_os(REQUIRE).is_none() => {
            println!("cargo::warning=mqtt-viewer.exe gets no icon: {reason}");
        }
        result @ (CompilationResult::NotWindows
        | CompilationResult::Ok
        | CompilationResult::NotAttempted(_)
        | CompilationResult::Failed(_)) => result.manifest_required()?,
    }
    Ok(())
}
