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
        return Some(format!("https://{}", &t[t.len() - rest.len()..]));
    }
    (lower.starts_with("https://") || lower.starts_with("http://")).then(|| t.to_string())
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
                    if let Ok(text) = std::fs::read_to_string(crate::config::data_dir().join(CACHE)) {
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
                        post(Event::Calendar { events: None, error: Some("Couldn't reach the calendar".into()) });
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
    cur.0 = url;
    cur.1 = cur.1.wrapping_add(1);
    s.wake.notify_all();
}

pub fn refresh_now() {
    let s = shared();
    let mut cur = s.url.lock().unwrap_or_else(|e| e.into_inner());
    cur.1 = cur.1.wrapping_add(1);
    s.wake.notify_all();
}

fn fetch(url: &str) -> windows::core::Result<String> {
    let client = HttpClient::new()?;
    let uri = Uri::CreateUri(&HSTRING::from(url))?;
    Ok(client.GetStringAsync(&uri)?.get()?.to_string())
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
}
