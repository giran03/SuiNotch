//! HTTP for the lyrics services: one shared client, a timeout on every request, a small gap between
//! requests to the same service, `Retry-After` honoured, and two lanes. A lookup for the song playing
//! runs in the foreground lane; a lookup ahead of time (see `lyrics_prefetch`) runs in the prefetch lane,
//! which gives way whenever the foreground is busy. Each lane is cancelled on its own.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::core::HSTRING;
use windows::Foundation::Uri;
use windows::Storage::Streams::UnicodeEncoding;
use windows::Web::Http::{HttpClient, HttpResponseMessage, HttpStringContent};

/// Names the app, its version and where to reach its makers, as LRCLIB asks of its clients.
const USER_AGENT: &str = concat!("SuiNotch/", env!("CARGO_PKG_VERSION"), " (+https://github.com/giran03/SuiNotch)");
const TIMEOUT: Duration = Duration::from_secs(8);
const POLL: Duration = Duration::from_millis(20);
/// Least time between two requests to one service, so a burst of lookups stays polite.
const LRCLIB_GAP: Duration = Duration::from_millis(250);
const OTHER_GAP: Duration = Duration::from_millis(150);
/// Waiting longer than this for a service that asked to be left alone fails the request instead.
const MAX_WAIT: Duration = Duration::from_secs(10);
/// A 429 or 503 without a `Retry-After` keeps the service alone for this long.
const DEFAULT_BACKOFF: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_secs(600);
/// `AsyncStatus::Started`: the operation has not settled yet. (The type itself is not exported by `windows`.)
const STARTED: i32 = 0;

/// Why a request gave nothing.
#[derive(Debug)]
pub enum Fail {
    /// The service answered with this status (404: it has no such record).
    Status(u16),
    /// The service asked to be left alone for longer than a lookup waits.
    Busy,
    Timeout,
    /// A newer lookup replaced this one.
    Cancelled,
    Net(windows::core::Error),
}

impl Fail {
    /// Whether the service said it has nothing, rather than failing to answer.
    pub fn is_not_found(&self) -> bool {
        matches!(self, Fail::Status(404))
    }

    /// Whether the service gave a definite answer, so asking again would get the same: any 4xx but a
    /// timeout (408) or "slow down" (429).
    pub fn is_answer(&self) -> bool {
        matches!(self, Fail::Status(code) if (400..500).contains(code) && *code != 408 && *code != 429)
    }
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fail::Status(code) => write!(f, "HTTP {code}"),
            Fail::Busy => write!(f, "service busy"),
            Fail::Timeout => write!(f, "timed out"),
            Fail::Cancelled => write!(f, "cancelled"),
            Fail::Net(e) => write!(f, "{e}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Foreground(u64),
    Prefetch(u64),
}

/// What a lookup's requests share, across the threads it runs on: its lane, whether any request
/// failed (as opposed to finding nothing), and counts for the diagnostics.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub lane: Lane,
    failed: Arc<AtomicBool>,
    requests: Arc<AtomicU32>,
    waited_ms: Arc<AtomicU64>,
}

static FOREGROUND: AtomicU64 = AtomicU64::new(0);
static PREFETCH: AtomicU64 = AtomicU64::new(0);
/// Foreground lookups running; the prefetch lane waits while any are.
static FOREGROUND_BUSY: AtomicUsize = AtomicUsize::new(0);
static CLIENT: OnceLock<HttpClient> = OnceLock::new();
/// Per service: when it was last asked, and until when (if at all) it asked to be left alone.
type Pace = HashMap<String, (Instant, Option<Instant>)>;
static HOSTS: Mutex<Option<Pace>> = Mutex::new(None);

thread_local! {
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

/// A new foreground generation; every earlier foreground lookup stops at its next request.
pub fn next_foreground() -> u64 {
    FOREGROUND.fetch_add(1, Ordering::SeqCst) + 1
}

pub fn cancel_foreground() {
    FOREGROUND.fetch_add(1, Ordering::SeqCst);
}

pub fn next_prefetch() -> u64 {
    PREFETCH.fetch_add(1, Ordering::SeqCst) + 1
}

pub fn cancel_prefetch() {
    PREFETCH.fetch_add(1, Ordering::SeqCst);
}

/// Starts a lookup's context on this thread. A foreground lookup holds the prefetch lane back until
/// the returned guard is dropped.
pub fn enter(lane: Lane) -> Guard {
    adopt(Ctx { lane, failed: Arc::default(), requests: Arc::default(), waited_ms: Arc::default() });
    let foreground = matches!(lane, Lane::Foreground(_));
    if foreground {
        FOREGROUND_BUSY.fetch_add(1, Ordering::SeqCst);
    }
    Guard { foreground }
}

pub struct Guard {
    foreground: bool,
}

impl Drop for Guard {
    fn drop(&mut self) {
        if self.foreground {
            FOREGROUND_BUSY.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// This thread's lookup context, to hand to a helper thread.
pub fn current() -> Option<Ctx> {
    CTX.with(|c| c.borrow().clone())
}

/// Joins a helper thread to a lookup started elsewhere.
pub fn adopt(ctx: Ctx) {
    CTX.with(|c| *c.borrow_mut() = Some(ctx));
}

fn lane_live(lane: Lane) -> bool {
    match lane {
        Lane::Foreground(g) => FOREGROUND.load(Ordering::SeqCst) == g,
        Lane::Prefetch(g) => PREFETCH.load(Ordering::SeqCst) == g,
    }
}

/// Whether this thread's lookup is still wanted; a thread outside any lookup always is.
pub fn live() -> bool {
    current().is_none_or(|c| lane_live(c.lane))
}

/// Whether any request of this lookup failed to get an answer, so "nothing found" may be wrong.
pub fn any_failed() -> bool {
    current().is_some_and(|c| c.failed.load(Ordering::SeqCst))
}

/// Forgets earlier failures, before another search round.
pub fn reset_failed() {
    if let Some(c) = current() {
        c.failed.store(false, Ordering::SeqCst);
    }
}

/// (requests sent, ms spent waiting for a service) so far in this lookup.
pub fn counts() -> (u32, u64) {
    current().map_or((0, 0), |c| (c.requests.load(Ordering::SeqCst), c.waited_ms.load(Ordering::SeqCst)))
}

/// Sleeps in small steps, giving up as soon as the lookup is replaced; false when it was.
pub fn sleep(d: Duration) -> bool {
    let end = Instant::now() + d;
    loop {
        if !live() {
            return false;
        }
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return true;
        }
        std::thread::sleep(left.min(Duration::from_millis(50)));
    }
}

fn client() -> Result<&'static HttpClient, Fail> {
    if let Some(c) = CLIENT.get() {
        return Ok(c);
    }
    let c = HttpClient::new().map_err(Fail::Net)?;
    if let Ok(h) = c.DefaultRequestHeaders().and_then(|h| h.UserAgent()) {
        let _ = h.TryParseAdd(&HSTRING::from(USER_AGENT));
    }
    Ok(CLIENT.get_or_init(|| c))
}

pub fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?']).next().unwrap_or(rest)
}

fn gap_for(host: &str) -> Duration {
    if host.ends_with("lrclib.net") { LRCLIB_GAP } else { OTHER_GAP }
}

/// How long to wait before asking `host` again, and notes that it is being asked then.
fn wait_for(host: &str, now: Instant) -> Result<Duration, Fail> {
    let mut hosts = HOSTS.lock().unwrap_or_else(|e| e.into_inner());
    let map = hosts.get_or_insert_with(HashMap::new);
    let (free_at, blocked) = match map.get(host).copied() {
        Some((last, blocked)) => ((last + gap_for(host)).max(blocked.unwrap_or(now)), blocked),
        None => (now, None),
    };
    let wait = free_at.saturating_duration_since(now);
    if wait > MAX_WAIT {
        return Err(Fail::Busy);
    }
    map.insert(host.to_string(), (now + wait, blocked.filter(|b| *b > now)));
    Ok(wait)
}

/// Leaves `host` alone for `d`, as it asked.
fn back_off(host: &str, d: Duration) {
    let mut hosts = HOSTS.lock().unwrap_or_else(|e| e.into_inner());
    let map = hosts.get_or_insert_with(HashMap::new);
    let now = Instant::now();
    map.entry(host.to_string()).or_insert((now, None)).1 = Some(now + d);
}

/// Waits for the prefetch lane's turn and for the service's gap.
fn before_request(url: &str) -> Result<(), Fail> {
    if !live() {
        return Err(Fail::Cancelled);
    }
    let ctx = current();
    if ctx.as_ref().is_some_and(|c| matches!(c.lane, Lane::Prefetch(_))) {
        while FOREGROUND_BUSY.load(Ordering::SeqCst) > 0 {
            if !sleep(Duration::from_millis(100)) {
                return Err(Fail::Cancelled);
            }
        }
    }
    let wait = wait_for(host_of(url), Instant::now())?;
    if !wait.is_zero() {
        if let Some(c) = &ctx {
            c.waited_ms.fetch_add(wait.as_millis() as u64, Ordering::SeqCst);
        }
        if !sleep(wait) {
            return Err(Fail::Cancelled);
        }
    }
    if let Some(c) = &ctx {
        c.requests.fetch_add(1, Ordering::SeqCst);
    }
    Ok(())
}

/// Waits for a WinRT operation until it settles, the timeout passes or the lookup is replaced.
fn settle(status: impl Fn() -> windows::core::Result<i32>, cancel: impl Fn(), deadline: Instant) -> Result<(), Fail> {
    loop {
        if status().map_err(Fail::Net)? != STARTED {
            return Ok(());
        }
        if !live() {
            cancel();
            return Err(Fail::Cancelled);
        }
        if Instant::now() >= deadline {
            cancel();
            return Err(Fail::Timeout);
        }
        std::thread::sleep(POLL);
    }
}

fn noted<T>(result: Result<T, Fail>) -> Result<T, Fail> {
    if let Err(e) = &result {
        if !e.is_answer() && !matches!(e, Fail::Cancelled) {
            if let Some(c) = current() {
                c.failed.store(true, Ordering::SeqCst);
            }
        }
    }
    result
}

/// The body of a GET, or why there is none. A 404 is "no such record", not a failure.
pub fn get(url: &str) -> Result<String, Fail> {
    noted((|| {
        before_request(url)?;
        let uri = Uri::CreateUri(&HSTRING::from(url)).map_err(Fail::Net)?;
        let op = client()?.GetAsync(&uri).map_err(Fail::Net)?;
        let deadline = Instant::now() + TIMEOUT;
        settle(|| op.Status().map(|s| s.0), || drop(op.Cancel()), deadline)?;
        read_reply(url, op.GetResults().map_err(Fail::Net)?, deadline)
    })())
}

/// A form POST with extra headers, for services that want more than a plain GET.
pub fn post_form(url: &str, form: &str, headers: &[(&str, &str)]) -> Result<String, Fail> {
    noted((|| {
        before_request(url)?;
        // Headers belong to a client, so a request with its own gets a client of its own.
        let client = HttpClient::new().map_err(Fail::Net)?;
        let defaults = client.DefaultRequestHeaders().map_err(Fail::Net)?;
        for (name, value) in headers {
            let _ = defaults.TryAppendWithoutValidation(&HSTRING::from(*name), &HSTRING::from(*value));
        }
        let content = HttpStringContent::CreateFromStringWithEncodingAndMediaType(
            &HSTRING::from(form),
            UnicodeEncoding::Utf8,
            &HSTRING::from("application/x-www-form-urlencoded"),
        )
        .map_err(Fail::Net)?;
        let uri = Uri::CreateUri(&HSTRING::from(url)).map_err(Fail::Net)?;
        let op = client.PostAsync(&uri, &content).map_err(Fail::Net)?;
        let deadline = Instant::now() + TIMEOUT;
        settle(|| op.Status().map(|s| s.0), || drop(op.Cancel()), deadline)?;
        read_reply(url, op.GetResults().map_err(Fail::Net)?, deadline)
    })())
}

fn read_reply(url: &str, reply: HttpResponseMessage, deadline: Instant) -> Result<String, Fail> {
    let code = reply.StatusCode().map_err(Fail::Net)?.0 as u16;
    if code == 429 || code == 503 {
        let after = reply.Headers().ok().and_then(|h| h.RetryAfter().ok()).and_then(|r| r.Delta().ok()).and_then(|d| d.Value().ok());
        let d = after.map_or(DEFAULT_BACKOFF, |t| Duration::from_millis((t.Duration / 10_000).max(0) as u64));
        back_off(host_of(url), d.min(MAX_BACKOFF));
        return Err(Fail::Status(code));
    }
    if !(200..300).contains(&code) {
        return Err(Fail::Status(code));
    }
    let read = reply.Content().and_then(|c| c.ReadAsStringAsync()).map_err(Fail::Net)?;
    settle(|| read.Status().map(|s| s.0), || drop(read.Cancel()), deadline)?;
    Ok(read.GetResults().map_err(Fail::Net)?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_read_from_urls() {
        assert_eq!(host_of("https://lrclib.net/api/get?x=1"), "lrclib.net");
        assert_eq!(host_of("https://itunes.apple.com/search?term=a"), "itunes.apple.com");
        assert_eq!(host_of("lrclib.net"), "lrclib.net");
    }

    #[test]
    fn requests_to_one_service_are_spaced_and_back_off_is_kept() {
        let host = "pace.test";
        let t = Instant::now();
        assert_eq!(wait_for(host, t).unwrap(), Duration::ZERO);
        // A second request right away waits out the gap.
        assert_eq!(wait_for(host, t).unwrap(), OTHER_GAP);
        back_off(host, Duration::from_secs(60));
        assert!(matches!(wait_for(host, Instant::now()), Err(Fail::Busy)));
        assert_eq!(wait_for("other.test", t).unwrap(), Duration::ZERO);
    }

    #[test]
    fn lanes_are_cancelled_on_their_own() {
        let fg = next_foreground();
        let pf = next_prefetch();
        assert!(lane_live(Lane::Foreground(fg)) && lane_live(Lane::Prefetch(pf)));
        cancel_foreground();
        assert!(!lane_live(Lane::Foreground(fg)));
        assert!(lane_live(Lane::Prefetch(pf)), "a new song does not stop a lookup ahead");
        cancel_prefetch();
        assert!(!lane_live(Lane::Prefetch(pf)));
    }

    #[test]
    fn only_answers_other_than_not_found_count_as_failures() {
        assert!(Fail::Status(404).is_not_found());
        assert!(!Fail::Status(500).is_not_found() && !Fail::Timeout.is_not_found());
        assert!(Fail::Status(400).is_answer() && Fail::Status(404).is_answer());
        assert!(!Fail::Status(429).is_answer() && !Fail::Status(408).is_answer() && !Fail::Status(503).is_answer());
        let _ = noted::<()>(Err(Fail::Status(400)));
        assert!(!any_failed(), "a bad request will not get better by asking again");
        // A lane no test bumps, so tests running alongside can't disturb another's liveness.
        let _g = enter(Lane::Prefetch(u64::MAX));
        let _ = noted::<()>(Err(Fail::Status(404)));
        assert!(!any_failed());
        let _ = noted::<()>(Err(Fail::Timeout));
        assert!(any_failed());
        reset_failed();
        assert!(!any_failed());
    }
}
