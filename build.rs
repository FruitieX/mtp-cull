fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=Cargo.toml");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .set("ProductName", "mtp-cull")
            .set("FileDescription", "Camera photo review and MTP backup")
            .set("CompanyName", "FruitieX")
            .set("OriginalFilename", "mtp-cull.exe")
            .set("InternalName", "mtp-cull")
            .compile()?;
    }
    Ok(())
}
