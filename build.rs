//! Embeds the SuiNotch icon and version information into the executable, and picks the
//! release notes for the version in Cargo.toml (changelogs/v<version>.md).

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=assets/suinotch.ico");
    println!("cargo:rerun-if-changed=changelogs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    embed_changelog();
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

/// Bumping the version switches the app to that version's notes; a version without notes
/// (no file, or only the title the release script starts it with) still builds, with a
/// short placeholder.
fn embed_changelog() {
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let source = format!("changelogs/v{version}.md");
    let written = |t: &String| t.lines().map(str::trim).any(|l| !l.is_empty() && !l.starts_with("# "));
    let text = std::fs::read_to_string(&source).ok().filter(written).unwrap_or_else(|| {
        println!("cargo:warning=no release notes at {source}");
        format!("# SuiNotch {version}\n\n- No release notes for this version yet.\n")
    });
    let out = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("changelog.md");
    std::fs::write(out, text).expect("write changelog");
}
