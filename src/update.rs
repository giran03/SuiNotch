//! Updates from GitHub Releases. The latest release must carry the NSIS installer
//! (`SuiNotch-<version>-Setup.exe`) and `SHA256SUMS.txt`, both made by scripts/release.bat.
//! The installer is checked against the sum, then run silently; it closes this copy and
//! starts the new one.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::Value;
use windows::core::{Result, HSTRING};
use windows::Foundation::Uri;
use windows::Security::Cryptography::Core::{HashAlgorithmNames, HashAlgorithmProvider};
use windows::Security::Cryptography::CryptographicBuffer;
use windows::Storage::Streams::{DataReader, IBuffer};
use windows::Web::Http::{HttpClient, HttpStatusCode};

use crate::events::{post, Event};

pub const REPO: &str = "giran03/SuiNotch";
const USER_AGENT: &str = concat!("SuiNotch/", env!("CARGO_PKG_VERSION"), " (update check)");
const SUMS_NAME: &str = "SHA256SUMS.txt";
const FIRST_CHECK: Duration = Duration::from_secs(90);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// Automatic installs wait until the PC has been left alone this long, so nobody loses the notch mid-task.
const IDLE_BEFORE_INSTALL_S: u32 = 10 * 60;

/// Settings > About > Updates: 0 off, 1 notify, 2 install automatically.
pub const MODE_LABELS: [&str; 3] = ["Off", "Notify me", "Automatic"];
pub const MODE_OFF: usize = 0;
pub const MODE_AUTO: usize = 2;

static MODE: AtomicUsize = AtomicUsize::new(MODE_AUTO);
static BUSY: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub setup_url: String,
    pub setup_name: String,
    pub sums_url: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading(String),
    Failed(String),
}

pub fn set_mode(mode: usize) {
    MODE.store(mode, Ordering::Relaxed);
}

/// Background checks: shortly after launch, then every few hours while the mode allows it.
pub fn start(mode: usize) {
    set_mode(mode);
    std::thread::spawn(|| {
        std::thread::sleep(FIRST_CHECK);
        loop {
            let mode = MODE.load(Ordering::Relaxed);
            if mode != MODE_OFF {
                if let Some(release) = run_check() {
                    if mode == MODE_AUTO {
                        wait_until_idle();
                        if MODE.load(Ordering::Relaxed) == MODE_AUTO {
                            run_install(&release);
                        }
                    }
                }
            }
            std::thread::sleep(CHECK_EVERY);
        }
    });
}

/// "Check now" in Settings.
pub fn check_now() {
    std::thread::spawn(|| {
        run_check();
    });
}

/// "Install and restart" in Settings.
pub fn install(release: Release) {
    std::thread::spawn(move || run_install(&release));
}

fn wait_until_idle() {
    while crate::sys::idle_seconds() < IDLE_BEFORE_INSTALL_S {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn run_check() -> Option<Release> {
    if BUSY.swap(true, Ordering::AcqRel) {
        return None;
    }
    post(Event::Update(Status::Checking));
    let (status, found) = match latest() {
        Ok(Some(r)) if is_newer(&r.version, env!("CARGO_PKG_VERSION")) => (Status::Available(r.clone()), Some(r)),
        Ok(_) => (Status::UpToDate, None),
        Err(msg) => {
            crate::log(&format!("update check failed: {msg}"));
            (Status::Failed(msg), None)
        }
    };
    post(Event::Update(status));
    BUSY.store(false, Ordering::Release);
    found
}

fn run_install(release: &Release) {
    if BUSY.swap(true, Ordering::AcqRel) {
        return;
    }
    post(Event::Update(Status::Downloading(release.version.clone())));
    match download_verified(release) {
        Ok(path) => {
            crate::log(&format!("installing update {}", release.version));
            // The installer closes this copy, replaces it and starts the new version.
            if let Err(e) = std::process::Command::new(&path).args(["/S", "/UPDATE"]).spawn() {
                crate::log(&format!("cannot start update installer: {e}"));
                post(Event::Update(Status::Failed("Couldn't start the installer".into())));
            }
        }
        Err(msg) => {
            crate::log(&format!("update install failed: {msg}"));
            post(Event::Update(Status::Failed(msg)));
        }
    }
    BUSY.store(false, Ordering::Release);
}

// ------------------------------------------------------------------ network

fn client() -> Result<HttpClient> {
    let client = HttpClient::new()?;
    let headers = client.DefaultRequestHeaders()?;
    let _ = headers.UserAgent()?.TryParseAdd(&HSTRING::from(USER_AGENT));
    let _ = headers.TryAppendWithoutValidation(&HSTRING::from("Accept"), &HSTRING::from("application/vnd.github+json"));
    Ok(client)
}

fn get_buffer(client: &HttpClient, url: &str) -> std::result::Result<IBuffer, String> {
    let uri = Uri::CreateUri(&HSTRING::from(url)).map_err(|e| e.message())?;
    let resp = client.GetAsync(&uri).and_then(|op| op.get()).map_err(|_| "No internet connection".to_string())?;
    let code = resp.StatusCode().map_err(|e| e.message())?;
    if code == HttpStatusCode::NotFound {
        return Err("No public release found".into());
    }
    if !resp.IsSuccessStatusCode().unwrap_or(false) {
        return Err(format!("Update server replied {}", code.0));
    }
    resp.Content().and_then(|c| c.ReadAsBufferAsync()).and_then(|op| op.get()).map_err(|e| e.message())
}

fn bytes(buf: &IBuffer) -> std::result::Result<Vec<u8>, String> {
    let len = buf.Length().map_err(|e| e.message())? as usize;
    let mut out = vec![0u8; len];
    let reader = DataReader::FromBuffer(buf).map_err(|e| e.message())?;
    reader.ReadBytes(&mut out).map_err(|e| e.message())?;
    Ok(out)
}

fn latest() -> std::result::Result<Option<Release>, String> {
    let client = client().map_err(|e| e.message())?;
    let body = bytes(&get_buffer(&client, &format!("https://api.github.com/repos/{REPO}/releases/latest"))?)?;
    let json: Value = serde_json::from_slice(&body).map_err(|_| "Unreadable release data".to_string())?;
    Ok(parse_release(&json))
}

fn download_verified(release: &Release) -> std::result::Result<std::path::PathBuf, String> {
    let client = client().map_err(|e| e.message())?;
    let sums = String::from_utf8(bytes(&get_buffer(&client, &release.sums_url)?)?).map_err(|_| "Unreadable checksum file".to_string())?;
    let expected = expected_sum(&sums, &release.setup_name).ok_or("The release has no checksum for its installer")?;
    let setup = get_buffer(&client, &release.setup_url)?;
    if sha256_hex(&setup).map_err(|e| e.message())? != expected {
        return Err("The download was damaged, please try again".into());
    }
    let dir = std::env::temp_dir().join("SuiNotch-update");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(&release.setup_name);
    std::fs::write(&path, bytes(&setup)?).map_err(|e| e.to_string())?;
    Ok(path)
}

fn sha256_hex(buf: &IBuffer) -> Result<String> {
    let provider = HashAlgorithmProvider::OpenAlgorithm(&HashAlgorithmNames::Sha256()?)?;
    let hash = provider.HashData(buf)?;
    Ok(CryptographicBuffer::EncodeToHexString(&hash)?.to_string().to_ascii_lowercase())
}

// ------------------------------------------------------------------ parsing

/// Reads a GitHub "latest release" response; None when it lacks the installer or its sums.
pub fn parse_release(v: &Value) -> Option<Release> {
    let version = v.get("tag_name")?.as_str()?.trim_start_matches(['v', 'V']).to_string();
    parse_version(&version)?;
    let assets = v.get("assets")?.as_array()?;
    let asset = |pred: &dyn Fn(&str) -> bool| {
        assets.iter().find_map(|a| {
            let name = a.get("name")?.as_str()?;
            let url = a.get("browser_download_url")?.as_str()?;
            pred(name).then(|| (name.to_string(), url.to_string()))
        })
    };
    let (setup_name, setup_url) = asset(&|n| n.to_ascii_lowercase().ends_with("-setup.exe"))?;
    let (_, sums_url) = asset(&|n| n.eq_ignore_ascii_case(SUMS_NAME))?;
    Some(Release { version, setup_url, setup_name, sums_url })
}

/// Finds `name` in a `sha256sum`-style file ("<hex>  <name>" per line).
pub fn expected_sum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim().trim_start_matches('*');
        (file.eq_ignore_ascii_case(name) && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).then(|| hash.to_ascii_lowercase())
    })
}

pub fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let core = v.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u32>().ok());
    let major = parts.next()??;
    let minor = parts.next().unwrap_or(Some(0))?;
    let patch = parts.next().unwrap_or(Some(0))?;
    Some((major, minor, patch))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("1.0.10", "1.0.9"));
        assert!(is_newer("v2.0.0", "1.9.9"));
        assert!(is_newer("1.1", "1.0.5"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
        assert!(!is_newer("nightly", "1.0.0"));
    }

    #[test]
    fn reads_installer_and_sums_from_release() {
        let v = json!({
            "tag_name": "v1.2.0",
            "assets": [
                { "name": "CHANGELOG-v1.2.0.md", "browser_download_url": "https://x/c" },
                { "name": "SuiNotch-1.2.0-Setup.exe", "browser_download_url": "https://x/s" },
                { "name": "SHA256SUMS.txt", "browser_download_url": "https://x/h" }
            ]
        });
        let r = parse_release(&v).unwrap();
        assert_eq!(r.version, "1.2.0");
        assert_eq!(r.setup_name, "SuiNotch-1.2.0-Setup.exe");
        assert_eq!(r.setup_url, "https://x/s");
        assert_eq!(r.sums_url, "https://x/h");
    }

    #[test]
    fn release_without_checksums_is_ignored() {
        let v = json!({ "tag_name": "v1.2.0", "assets": [{ "name": "SuiNotch-1.2.0-Setup.exe", "browser_download_url": "https://x/s" }] });
        assert_eq!(parse_release(&v), None);
    }

    #[test]
    fn hashes_with_sha256() {
        let buf = CryptographicBuffer::ConvertStringToBinary(&HSTRING::from("abc"), windows::Security::Cryptography::BinaryStringEncoding::Utf8).unwrap();
        assert_eq!(sha256_hex(&buf).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    /// Talks to GitHub: `cargo test -- --ignored live_latest_release`.
    #[test]
    #[ignore]
    fn live_latest_release() {
        println!("{:?}", latest());
    }

    #[test]
    fn finds_sum_for_file() {
        let h = "956b4afaf1d2bbcc8c80789a4b4f3dae4a3f0cd285be6452a965bc96ca15e234";
        let sums = format!("{h}  SuiNotch-1.0.0-Setup.exe\r\n{}  other.txt\n", "0".repeat(64));
        assert_eq!(expected_sum(&sums, "SuiNotch-1.0.0-Setup.exe").as_deref(), Some(h));
        assert_eq!(expected_sum(&format!("{} *SuiNotch-1.0.0-Setup.exe", h.to_uppercase()), "SuiNotch-1.0.0-Setup.exe").as_deref(), Some(h));
        assert_eq!(expected_sum("abc  SuiNotch-1.0.0-Setup.exe", "SuiNotch-1.0.0-Setup.exe"), None);
        assert_eq!(expected_sum(&sums, "missing.exe"), None);
    }
}
