// Embeds the app icon and a version resource (from Cargo.toml) into the Windows exe.
// The installer reads ProductVersion from it, so the version is only ever written in Cargo.toml.
fn main() {
    println!("cargo:rerun-if-changed=assets/vsoundboard.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/vsoundboard.ico")
            .compile()
            .expect("embed Windows resources");
    }
}
