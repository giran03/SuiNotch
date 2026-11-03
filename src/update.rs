//! Updates from GitHub Releases. The latest release must carry the NSIS installer
//! (`SuiNotch-<version>-Setup.exe`) and `SHA256SUMS.txt`, both made by scripts/release.bat.
//! The installer is checked against the sum, then run silently; it closes this copy and
//! starts the new one.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
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
static PREPARED: Mutex<Option<Download>> = Mutex::new(None);

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub setup_url: String,
    pub setup_name: String,
    pub sums_url: String,
}

/// An installer whose checksum was verified, checked again immediately before launch.
#[derive(Clone, Debug, PartialEq)]
pub struct Download {
    pub release: Release,
    pub path: std::path::PathBuf,
    pub sha256: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading(String),
    Ready(Download),
    Installing(String),
    Installed(String),
    InstallFailed {
        release: Release,
        message: String,
    },
    Failed(String),
}

impl Status {
    pub fn version(&self) -> Option<&str> {
        match self {
            Self::Available(release) | Self::InstallFailed { release, .. } => Some(&release.version),
            Self::Ready(download) => Some(&download.release.version),
            Self::Downloading(version) | Self::Installing(version) | Self::Installed(version) => Some(version),
            _ => None,
        }
    }
    pub fn tab_visible(&self) -> bool {
        matches!(self, Self::Available(_) | Self::Downloading(_) | Self::Ready(_) | Self::Installing(_) | Self::InstallFailed { .. })
    }

    /// A routine recheck must not flicker the tab or discard an already verified download.
    pub fn receive(&mut self, incoming: Status) {
        if (self.tab_visible() || matches!(self, Self::Installed(_))) && matches!(&incoming, Self::Checking | Self::Failed(_)) {
            return;
        }
        if matches!((&*self, &incoming), (Self::Ready(old), Self::Available(new)) if old.release.version == new.version) {
            return;
        }
        if matches!((&*self, &incoming), (Self::Installed(old), Self::Available(new)) if !is_newer(&new.version, old)) {
            return;
        }
        *self = incoming;
    }
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
                    if mode == MODE_AUTO && wait_until_idle() {
                        run_install(&release);
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

/// Download and verify now, leaving the restart decision to the user.
pub fn download(release: Release) {
    std::thread::spawn(move || run_download(&release, false));
}

pub fn install_ready(download: Download) {
    std::thread::spawn(move || {
        let Some(_busy) = BusyGuard::acquire() else { return };
        launch(&download);
    });
}

fn wait_until_idle() -> bool {
    while crate::sys::idle_seconds() < IDLE_BEFORE_INSTALL_S {
        if MODE.load(Ordering::Relaxed) != MODE_AUTO {
            return false;
        }
        std::thread::sleep(Duration::from_secs(60));
    }
    MODE.load(Ordering::Relaxed) == MODE_AUTO
}

struct BusyGuard;
impl BusyGuard {
    fn acquire() -> Option<Self> {
        if BUSY.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            Some(Self)
        } else {
            None
        }
    }
}
impl Drop for BusyGuard {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::Release);
    }
}

fn run_check() -> Option<Release> {
    let _busy = BusyGuard::acquire()?;
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
    found
}

fn run_install(release: &Release) {
    let prepared = PREPARED.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(download) = prepared.filter(|d| d.release == *release && d.path.is_file()) {
        let Some(_busy) = BusyGuard::acquire() else { return };
        launch(&download);
    } else {
        run_download(release, true);
    }
}

fn run_download(release: &Release, install_after: bool) {
    let Some(_busy) = BusyGuard::acquire() else { return };
    post(Event::Update(Status::Downloading(release.version.clone())));
    match download_verified(release) {
        Ok(download) => {
            *PREPARED.lock().unwrap_or_else(|e| e.into_inner()) = Some(download.clone());
            if install_after {
                launch(&download);
            } else {
                post(Event::Update(Status::Ready(download)));
            }
        }
        Err(msg) => {
            crate::log(&format!("update install failed: {msg}"));
            post(Event::Update(Status::InstallFailed { release: release.clone(), message: msg }));
        }
    }
}

fn verify_download(download: &Download) -> std::result::Result<(), String> {
    let data = std::fs::read(&download.path).map_err(|_| "The downloaded installer is missing. Download the update again.")?;
    let buffer = CryptographicBuffer::CreateFromByteArray(&data).map_err(|e| e.message())?;
    if sha256_hex(&buffer).map_err(|e| e.message())? != download.sha256 {
        return Err("The installer changed after download. Download the update again.".into());
    }
    Ok(())
}

fn launch(download: &Download) {
    let result = verify_download(download).and_then(|()| {
        crate::log(&format!("installing update {}", download.release.version));
        let mut child = std::process::Command::new(&download.path).args(["/S", "/UPDATE"]).spawn().map_err(|_| "Couldn't start the installer".to_string())?;
        post(Event::Update(Status::Installing(download.release.version.clone())));
        // Normally the installer closes this process and starts the updated version (whose tab is hidden).
        let exit = child.wait().map_err(|_| "Couldn't read the installer result".to_string())?;
        if !exit.success() {
            return Err("Installation did not complete. Please try again.".into());
        }
        post(Event::Update(Status::Installed(download.release.version.clone())));
        Ok(())
    });
    if let Err(message) = result {
        crate::log(&format!("update install failed: {message}"));
        *PREPARED.lock().unwrap_or_else(|e| e.into_inner()) = None;
        post(Event::Update(Status::InstallFailed { release: download.release.clone(), message }));
    }
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

fn download_verified(release: &Release) -> std::result::Result<Download, String> {
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
    Ok(Download { release: release.clone(), path, sha256: expected })
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
    if !version.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+')) {
        return None;
    }
    parse_version(&version)?;
    let assets = v.get("assets")?.as_array()?;
    let asset = |pred: &dyn Fn(&str) -> bool| {
        assets.iter().find_map(|a| {
            let name = a.get("name")?.as_str()?;
            let url = a.get("browser_download_url")?.as_str()?;
            pred(name).then(|| (name.to_string(), url.to_string()))
        })
    };
    let expected_name = format!("SuiNotch-{version}-Setup.exe");
    let (setup_name, setup_url) = asset(&|n| n.eq_ignore_ascii_case(&expected_name))?;
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

    fn release() -> Release {
        Release {
            version: "99.0.0".into(),
            setup_url: "https://example.invalid/setup".into(),
            setup_name: "SuiNotch-99.0.0-Setup.exe".into(),
            sums_url: "https://example.invalid/sums".into(),
        }
    }
    fn prepared() -> Download {
        Download { release: release(), path: std::path::PathBuf::from("missing-test-installer.exe"), sha256: "0".repeat(64) }
    }

    #[test]
    fn update_tab_tracks_actionable_states_and_disappears_on_completion() {
        for status in [Status::Idle, Status::Checking, Status::UpToDate, Status::Failed("offline".into()), Status::Installed("99.0.0".into())] {
            assert!(!status.tab_visible());
        }
        for status in [
            Status::Available(release()),
            Status::Downloading("99.0.0".into()),
            Status::Ready(prepared()),
            Status::Installing("99.0.0".into()),
            Status::InstallFailed { release: release(), message: "offline".into() },
        ] {
            assert!(status.tab_visible());
        }
        let mut status = Status::Idle;
        status.receive(Status::Available(release()));
        status.receive(Status::Downloading("99.0.0".into()));
        status.receive(Status::Ready(prepared()));
        status.receive(Status::Installing("99.0.0".into()));
        assert!(status.tab_visible());
        status.receive(Status::Installed("99.0.0".into()));
        assert!(!status.tab_visible());
    }

    #[test]
    fn checks_do_not_discard_downloads_or_resurrect_completed_update() {
        let mut status = Status::Ready(prepared());
        for incoming in [Status::Checking, Status::Failed("offline".into()), Status::Available(release())] {
            status.receive(incoming);
            assert!(matches!(status, Status::Ready(_)));
        }
        status.receive(Status::Installed("99.0.0".into()));
        status.receive(Status::Checking);
        status.receive(Status::Available(release()));
        assert!(!status.tab_visible());
        let mut next = release();
        next.version = "99.0.1".into();
        status.receive(Status::Available(next));
        assert!(status.tab_visible());
    }

    #[test]
    fn rejected_busy_guard_never_unlocks_an_active_operation() {
        let first = BusyGuard::acquire().unwrap();
        assert!(BusyGuard::acquire().is_none());
        assert!(BUSY.load(Ordering::Acquire));
        drop(first);
        assert!(!BUSY.load(Ordering::Acquire));
    }

    #[test]
    fn changed_or_missing_installer_is_never_launched() {
        let mut download = prepared();
        assert!(verify_download(&download).is_err());
        let dir = std::env::temp_dir().join(format!("suinotch-verify-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        download.path = dir.join("fixture.bin");
        std::fs::write(&download.path, b"abc").unwrap();
        download.sha256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into();
        assert!(verify_download(&download).is_ok());
        std::fs::write(&download.path, b"changed").unwrap();
        assert!(verify_download(&download).is_err());
        std::fs::remove_file(&download.path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn installer_asset_must_be_the_expected_file_name() {
        for name in ["../SuiNotch-1.2.0-Setup.exe", "Other-Setup.exe", "SuiNotch-1.1.0-Setup.exe"] {
            let value = json!({"tag_name":"v1.2.0", "assets":[{"name":name,"browser_download_url":"https://example.invalid/s"},{"name":"SHA256SUMS.txt","browser_download_url":"https://example.invalid/h"}]});
            assert!(parse_release(&value).is_none());
        }
    }

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
