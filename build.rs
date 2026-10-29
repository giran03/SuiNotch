//! Embeds the SuiNotch icon and version information into the executable.

fn main() {
    println!("cargo:rerun-if-changed=assets/suinotch.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/suinotch.ico")
        .set("ProductName", "SuiNotch")
        .set("FileDescription", "SuiNotch")
        .set("CompanyName", "CafeTako")
        .set("LegalCopyright", "© 2026 CafeTako");
    if let Err(e) = res.compile() {
        // A missing Windows SDK only costs the icon; the app still builds.
        println!("cargo:warning=icon not embedded: {e}");
    }
}
