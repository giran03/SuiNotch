//! Events from a calendar link (an Outlook, Google or other iCal address), fetched in the
//! background every few minutes. The last copy is cached so events show offline and at startup.
//! The link is private: it is never written to the log.

use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

use windows::core::HSTRING;
use windows::Foundation::Uri;
use windows::Web::Http::HttpClient;

use crate::events::{post, Event};
use crate::ical::{self, CalEvent};

const REFRESH: Duration = Duration::from_secs(15 * 60);
const PAST_DAYS: i64 = 35;
const AHEAD_DAYS: i64 = 70;
const CACHE: &str = "calendar.ics";

struct Shared {
    url: Mutex<(String, u64)>,
    wake: Condvar,
}

fn shared() -> &'static Shared {
    static S: OnceLock<Shared> = OnceLock::new();
    S.get_or_init(|| Shared { url: Mutex::new((String::new(), 0)), wake: Condvar::new() })
}

/// Turns what the user pasted into a fetchable address, or None if it isn't a web link.
pub fn normalize(text: &str) -> Option<String> {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    if t.contains(char::is_whitespace) {
        return None;
    }
    if let Some(rest) = lower.strip_prefix("webcal://") {
        return normalize(&format!("https://{}", &t[t.len() - rest.len()..]));
    }
    if !lower.starts_with("https://") && !lower.starts_with("http://") { return None; }
    let rest = t.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() { return None; }
    let path = &rest[authority.len()..];
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    if authority.eq_ignore_ascii_case("calendar.google.com") && path == "/calendar/embed" {
        let sources: Vec<&str> = query.split('#').next()?.split('&').filter_map(|pair| pair.split_once('='))
            .filter(|(key, _)| *key == "src").map(|(_, value)| value).collect();
        // A combined embed can contain several calendars; this setting holds one feed.
        if sources.len() != 1 { return None; }
        let id = decode_query(sources[0])?;
        if id.trim().is_empty() || id.chars().any(char::is_control) { return None; }
        return Some(format!("https://calendar.google.com/calendar/ical/{}/public/basic.ics", crate::lyrics::encode(&id)));
    }
    Some(t.to_string())
}

fn decode_query(value: &str) -> Option<String> {
    let mut bytes = value.bytes();
    let mut out = Vec::new();
    while let Some(byte) = bytes.next() {
        out.push(match byte {
            b'%' => (bytes.next()? as char).to_digit(16)? as u8 * 16 + (bytes.next()? as char).to_digit(16)? as u8,
            b'+' => b' ',
            other => other,
        });
    }
    String::from_utf8(out).ok()
}

fn validate_calendar(text: &str) -> Result<(), &'static str> {
    let text = text.trim().trim_start_matches('\u{feff}').trim();
    if text.lines().next().map(str::trim) != Some("BEGIN:VCALENDAR") || text.lines().last().map(str::trim) != Some("END:VCALENDAR") {
        return Err("This link returned a web page, not calendar events. Use an iCal (.ics) link or a public Google Calendar embed link.");
    }
    Ok(())
}

/// Host name for display, e.g. "outlook.office365.com".
pub fn host(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split(['/', '?']).next().unwrap_or_default().to_string()
}

pub fn start(url: String) {
    set_url(url);
    std::thread::spawn(|| {
        let mut cached = true;
        loop {
            let (url, seen) = shared().url.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if url.is_empty() {
                let _ = std::fs::remove_file(crate::config::data_dir().join(CACHE));
                post(Event::Calendar { events: Some(Vec::new()), error: None });
            } else {
                if cached {
                    // Show the last copy straight away while the network catches up.
                    if let Ok(text) = std::fs::read_to_string(crate::config::data_dir().join(CACHE)).and_then(|text| {
                        validate_calendar(&text).map(|_| text).map_err(std::io::Error::other)
                    }) {
                        post(Event::Calendar { events: Some(expand(&text)), error: None });
                    }
                }
                match fetch(&url) {
                    Ok(text) => {
                        let _ = std::fs::write(crate::config::data_dir().join(CACHE), &text);
                        post(Event::Calendar { events: Some(expand(&text)), error: None });
                    }
                    Err(e) => {
                        crate::log(&format!("calendar refresh failed: {e}"));
                        post(Event::Calendar { events: None, error: Some(e) });
                    }
                }
            }
            cached = false;
            // Sleep until the next refresh, or until the link changes or a refresh is asked for.
            let guard = shared().url.lock().unwrap_or_else(|e| e.into_inner());
            let _ = shared().wake.wait_timeout_while(guard, REFRESH, |(_, g)| *g == seen);
        }
    });
}

/// Uses a new link (or none) and refreshes right away.
pub fn set_url(url: String) {
    let s = shared();
    let mut cur = s.url.lock().unwrap_or_else(|e| e.into_inner());
    // Also repairs Google embed links saved by older versions, without requiring another paste.
    cur.0 = normalize(&url).unwrap_or(url);
    cur.1 = cur.1.wrapping_add(1);
    s.wake.notify_all();
}

pub fn refresh_now() {
    let s = shared();
    let mut cur = s.url.lock().unwrap_or_else(|e| e.into_inner());
    cur.1 = cur.1.wrapping_add(1);
    s.wake.notify_all();
}

fn fetch(url: &str) -> Result<String, String> {
    let url = normalize(url).ok_or("Invalid calendar link. Use an iCal (.ics) link or a public Google Calendar embed link.")?;
    // Keep transport error details out of logs: private calendar links can contain access tokens.
    let network_error = |_| "Couldn't reach the calendar. Check your connection and try Refresh.".to_string();
    let client = HttpClient::new().map_err(network_error)?;
    let uri = Uri::CreateUri(&HSTRING::from(url)).map_err(network_error)?;
    let response = client.GetAsync(&uri).map_err(network_error)?.get().map_err(network_error)?;
    let status = response.StatusCode().map_err(network_error)?.0;
    if !(200..300).contains(&status) {
        return Err(format!("Calendar request failed (HTTP {status}). Check calendar sharing or use its private iCal link."));
    }
    let text = response.Content().map_err(network_error)?.ReadAsStringAsync().map_err(network_error)?.get().map_err(network_error)?.to_string();
    validate_calendar(&text)?;
    Ok(text)
}

fn expand(text: &str) -> Vec<CalEvent> {
    let now = crate::store::now();
    ical::parse(text, now - PAST_DAYS * 86_400, now + AHEAD_DAYS * 86_400)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_links_are_checked_and_webcal_becomes_https() {
        assert_eq!(normalize("  webcal://p01.example.com/cal.ics "), Some("https://p01.example.com/cal.ics".into()));
        assert_eq!(normalize("https://calendar.example.com/a/basic.ics"), Some("https://calendar.example.com/a/basic.ics".into()));
        assert_eq!(normalize("Team calendar"), None);
        assert_eq!(normalize("ftp://x"), None);
        assert_eq!(host("https://outlook.office365.com/owa/calendar/x/reachcalendar.ics"), "outlook.office365.com");
    }

    #[test]
    fn google_embed_links_use_the_public_feed_without_changing_ical_links() {
        let feed = "https://calendar.google.com/calendar/ical/person%40example.com/public/basic.ics";
        assert_eq!(normalize("https://calendar.google.com/calendar/embed?src=person%40example.com&ctz=Asia%2FManila"), Some(feed.into()));
        assert_eq!(normalize("https://calendar.google.com/calendar/embed?ctz=Asia%2FManila&src=person@example.com#view"), Some(feed.into()));
        assert_eq!(normalize(feed), Some(feed.into()));
        let private = "https://calendar.google.com/calendar/ical/person%40example.com/private-test/basic.ics";
        assert_eq!(normalize(private), Some(private.into()));
        assert_eq!(normalize("https://calendar.google.com/calendar/embed?src=team%2Bwork%40group.calendar.google.com"), Some("https://calendar.google.com/calendar/ical/team%2Bwork%40group.calendar.google.com/public/basic.ics".into()));
        for bad in ["src=", "src=%XX", "src=%FF", "src=%00", "ctz=UTC", "src=a&src=b"] {
            assert_eq!(normalize(&format!("https://calendar.google.com/calendar/embed?{bad}")), None, "{bad}");
        }
        let other = "https://calendar.google.com.example.org/calendar/embed?src=person@example.com";
        assert_eq!(normalize(other), Some(other.into()), "do not rewrite other hosts");
    }

    #[test]
    fn calendar_pages_and_truncated_feeds_are_not_successful_empty_syncs() {
        for invalid in ["", "<html>Calendar</html>", "<html>BEGIN:VCALENDAR</html>", "BEGIN:VCALENDAR\r\nVERSION:2.0"] {
            assert!(validate_calendar(invalid).is_err());
        }
        assert!(validate_calendar("\u{feff}BEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n").is_ok());
        let calendar = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:test\r\nSUMMARY:Imported event\r\nDTSTART:20261104T010000Z\r\nDTEND:20261104T020000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        assert!(validate_calendar(calendar).is_ok());
        let start = chrono::DateTime::parse_from_rfc3339("2026-11-04T00:00:00Z").unwrap().timestamp();
        let events = ical::parse(calendar, start, start + 86_400);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].start, start + 3600);
    }

    #[test]
    #[ignore = "live calendar request; set SUINOTCH_CALENDAR_TEST_URL to an authorized test calendar"]
    fn live_calendar_link_fetches_and_parses() {
        unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED).ok().unwrap(); }
        let url = std::env::var("SUINOTCH_CALENDAR_TEST_URL").expect("set an authorized calendar test URL");
        let text = fetch(&url).unwrap();
        let events = expand(&text);
        println!("Feed records: {}; occurrences in the 35-day past / 70-day ahead window: {}", text.matches("BEGIN:VEVENT").count(), events.len());
        assert!(validate_calendar(&text).is_ok());
        // A parser failure must not be hidden by the worker: this test calendar should have events.
        assert!(!events.is_empty(), "use a test calendar with events in the current sync window");
    }
}
