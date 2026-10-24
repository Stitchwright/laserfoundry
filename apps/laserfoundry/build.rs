//! Embeds the application icon in the Windows executable, so File Explorer and the task bar show it.
//! The icon is drawn by `laserfoundry-icon`; nothing is read from an image file. Other targets do nothing.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // A missing icon must not stop the build: warn and carry on with the default one.
    if let Err(e) = embed_icon() {
        println!("cargo::warning=could not put the icon in the executable: {e}");
    }
}

fn embed_icon() -> Result<(), String> {
    let dir = std::env::var_os("OUT_DIR").ok_or("no OUT_DIR")?;
    let path = std::path::Path::new(&dir).join("laserfoundry.ico");
    std::fs::write(&path, laserfoundry_icon::ico()).map_err(|e| e.to_string())?;
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(&path.to_string_lossy());
    resource.set("ProductName", "LaserFoundry");
    resource.set("FileDescription", "LaserFoundry");
    resource.compile().map_err(|e| e.to_string())
}
