//! Embeds the application icon, version information and the manifest (per-monitor DPI
//! awareness, Windows 10/11 compatibility, UTF-8 code page) into the executable.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/icon.ico");
    println!("cargo:rerun-if-changed=app.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resources = winresource::WindowsResource::new();
    resources
        .set_icon("../../assets/icon.ico")
        .set_manifest_file("app.manifest")
        .set("ProductName", "BatteryWidget")
        .set("FileDescription", "BatteryWidget")
        .set("InternalName", "BatteryWidget")
        .set("OriginalFilename", "BatteryWidget.exe")
        .set("LegalCopyright", "MIT License");
    if let Err(error) = resources.compile() {
        // A missing resource compiler must not block development builds; the program
        // also requests DPI awareness at run time.
        println!("cargo:warning=resources not embedded: {error}");
    }
}
