//! A small iCalendar (RFC 5545) reader for calendar links from Outlook, Google and others:
//! events with all-day, UTC, floating or zoned times, common repeat rules, exceptions and
//! moved occurrences. Zones are taken from the file's own VTIMEZONE blocks; an unknown zone
//! is read as local time.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Weekday};

/// Upper bound on generated occurrences per event, against runaway rules.
const MAX_STEPS: usize = 5000;

#[derive(Clone, Debug, PartialEq)]
pub struct CalEvent {
    pub id: u64,
    pub title: String,
    /// Unix seconds; all-day events run from local midnight to local midnight.
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
}

#[derive(Clone, Debug)]
struct Prop {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Prop {
    fn param(&self, key: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

#[derive(Clone, Debug, PartialEq)]
enum When {
    Date(NaiveDate),
    Floating(NaiveDateTime),
    Utc(NaiveDateTime),
    Zoned(NaiveDateTime, String),
}

impl When {
    fn naive(&self) -> NaiveDateTime {
        match self {
            When::Date(d) => d.and_time(NaiveTime::MIN),
            When::Floating(t) | When::Utc(t) | When::Zoned(t, _) => *t,
        }
    }

    /// The same kind of time at another wall-clock moment (used for repeats).
    fn with(&self, t: NaiveDateTime) -> When {
        match self {
            When::Date(_) => When::Date(t.date()),
            When::Floating(_) => When::Floating(t),
            When::Utc(_) => When::Utc(t),
            When::Zoned(_, z) => When::Zoned(t, z.clone()),
        }
    }
}

#[derive(Clone, Debug)]
struct TzRule {
    offset_to: i32,
    start: NaiveDateTime,
    /// (month, ordinal, weekday) from a yearly rule such as BYMONTH=3;BYDAY=-1SU.
    yearly: Option<(u32, i32, Weekday)>,
}

#[derive(Clone, Debug, Default)]
struct Zone {
    rules: Vec<TzRule>,
}

impl Zone {
    /// UTC offset in seconds in force at wall-clock time `t`.
    fn offset_at(&self, t: NaiveDateTime) -> Option<i32> {
        let mut best: Option<(NaiveDateTime, i32)> = None;
        for r in &self.rules {
            let mut instants = vec![r.start];
            if let Some((month, n, wd)) = r.yearly {
                for year in [t.year() - 1, t.year()] {
                    if let Some(d) = nth_weekday(year, month, n, wd) {
                        let at = d.and_time(r.start.time());
                        if at >= r.start {
                            instants.push(at);
                        }
                    }
                }
            }
            for at in instants.into_iter().filter(|at| *at <= t) {
                if best.is_none_or(|(b, _)| at > b) {
                    best = Some((at, r.offset_to));
                }
            }
        }
        best.map(|(_, o)| o).or_else(|| self.rules.iter().min_by_key(|r| r.start).map(|r| r.offset_to))
    }
}

/// Reads every event and expands repeats into occurrences that overlap `from..to` (unix seconds).
pub fn parse(text: &str, from: i64, to: i64) -> Vec<CalEvent> {
    let lines = unfold(text);
    let mut zones: HashMap<String, Zone> = HashMap::new();
    let mut events: Vec<Vec<Prop>> = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut current: Vec<Prop> = Vec::new();
    let mut zone_id = String::new();
    let mut zone = Zone::default();
    let mut rule: Vec<Prop> = Vec::new();
    for line in &lines {
        let Some(p) = parse_prop(line) else { continue };
        match p.name.as_str() {
            "BEGIN" => {
                let kind = p.value.to_ascii_uppercase();
                match kind.as_str() {
                    "VEVENT" => current.clear(),
                    "VTIMEZONE" => {
                        zone = Zone::default();
                        zone_id.clear();
                    }
                    "STANDARD" | "DAYLIGHT" => rule.clear(),
                    _ => {}
                }
                stack.push(kind);
            }
            "END" => {
                match stack.pop().as_deref() {
                    Some("VEVENT") if stack.last().is_none_or(|s| s == "VCALENDAR") => events.push(std::mem::take(&mut current)),
                    Some("STANDARD" | "DAYLIGHT") => {
                        if let Some(r) = tz_rule(&rule) {
                            zone.rules.push(r);
                        }
                    }
                    Some("VTIMEZONE") => {
                        zones.insert(zone_id.clone(), std::mem::take(&mut zone));
                    }
                    _ => {}
                }
            }
            _ => match stack.last().map(String::as_str) {
                Some("VEVENT") => current.push(p),
                Some("VTIMEZONE") if p.name == "TZID" => zone_id = p.value.clone(),
                Some("STANDARD" | "DAYLIGHT") => rule.push(p),
                _ => {}
            },
        }
    }

    let resolve = |w: &When| to_unix(w, &zones);
    // Occurrences replaced by a moved copy (RECURRENCE-ID) are dropped from their series.
    let mut moved: HashSet<(String, i64)> = HashSet::new();
    for e in &events {
        if let (Some(uid), Some(rid)) = (get(e, "UID"), e.iter().find(|p| p.name == "RECURRENCE-ID").and_then(when)) {
            moved.insert((uid.to_string(), resolve(&rid)));
        }
    }
    let mut out = Vec::new();
    for e in &events {
        expand_event(e, &resolve, &moved, from, to, &mut out);
    }
    out.sort_by_key(|e| (e.start, !e.all_day, e.title.clone()));
    out
}

fn expand_event(e: &[Prop], resolve: &dyn Fn(&When) -> i64, moved: &HashSet<(String, i64)>, from: i64, to: i64, out: &mut Vec<CalEvent>) {
    if get(e, "STATUS").is_some_and(|s| s.eq_ignore_ascii_case("CANCELLED")) {
        return;
    }
    let Some(start) = e.iter().find(|p| p.name == "DTSTART").and_then(when) else { return };
    let all_day = matches!(start, When::Date(_));
    let length = match e.iter().find(|p| p.name == "DTEND").and_then(when) {
        Some(end) => end.naive() - start.naive(),
        None => get(e, "DURATION").and_then(parse_duration).unwrap_or(if all_day { Duration::days(1) } else { Duration::zero() }),
    };
    let uid = get(e, "UID").unwrap_or_default().to_string();
    let title = get(e, "SUMMARY").map(unescape).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "Busy".to_string());
    let is_override = e.iter().any(|p| p.name == "RECURRENCE-ID");
    let excluded: HashSet<i64> = e
        .iter()
        .filter(|p| p.name == "EXDATE")
        .flat_map(|p| p.value.split(',').filter_map(|v| when(&Prop { value: v.to_string(), ..p.clone() })).collect::<Vec<_>>())
        .map(|w| resolve(&w))
        .collect();
    let rule = if is_override { None } else { get(e, "RRULE").map(Rule::parse) };
    let starts: Vec<NaiveDateTime> = match &rule {
        Some(r) => r.occurrences(start.naive(), &|t| resolve(&start.with(t)), from, to),
        None => vec![start.naive()],
    };
    for t in starts {
        let w = start.with(t);
        let s = resolve(&w);
        let end = resolve(&start.with(t + length)).max(s);
        if excluded.contains(&s) || (!is_override && moved.contains(&(uid.clone(), s))) {
            continue;
        }
        if end > from && s < to || (s == end && s >= from && s < to) {
            out.push(CalEvent { id: event_id(&uid, s), title: title.clone(), start: s, end, all_day });
        }
    }
}

fn event_id(uid: &str, start: i64) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    uid.hash(&mut h);
    start.hash(&mut h);
    h.finish()
}

// ---------------------------------------------------------------- repeat rules

#[derive(Clone, Debug, Default)]
struct Rule {
    freq: String,
    interval: i64,
    count: Option<usize>,
    until: Option<When>,
    by_day: Vec<(i32, Weekday)>,
    by_month: Vec<u32>,
    by_month_day: Vec<i32>,
}

impl Rule {
    fn parse(value: &str) -> Rule {
        let mut r = Rule { interval: 1, ..Rule::default() };
        for part in value.split(';') {
            let Some((k, v)) = part.split_once('=') else { continue };
            match k.to_ascii_uppercase().as_str() {
                "FREQ" => r.freq = v.to_ascii_uppercase(),
                "INTERVAL" => r.interval = v.parse().unwrap_or(1).max(1),
                "COUNT" => r.count = v.parse().ok(),
                "UNTIL" => r.until = parse_when(v, None),
                "BYDAY" => r.by_day = v.split(',').filter_map(parse_by_day).collect(),
                "BYMONTH" => r.by_month = v.split(',').filter_map(|m| m.parse().ok()).collect(),
                "BYMONTHDAY" => r.by_month_day = v.split(',').filter_map(|d| d.parse().ok()).collect(),
                _ => {}
            }
        }
        r
    }

    /// Start times in order, from `start` until COUNT, UNTIL or `to` (unix) is reached.
    /// Without COUNT, daily and weekly series skip ahead to just before `from`.
    fn occurrences(&self, start: NaiveDateTime, unix: &dyn Fn(NaiveDateTime) -> i64, from: i64, to: i64) -> Vec<NaiveDateTime> {
        let until = self.until.as_ref().map(|u| match u {
            // A date-only UNTIL includes that whole day.
            When::Date(d) => unix(d.and_time(NaiveTime::MIN) + Duration::days(1)) - 1,
            other => to_unix(other, &HashMap::new()),
        });
        let behind_days = chrono::DateTime::from_timestamp(from, 0).map_or(0, |f| (f.naive_utc() - start).num_days());
        let period_days = match self.freq.as_str() {
            "DAILY" => 1,
            "WEEKLY" => 7,
            _ => 0,
        };
        let first = if self.count.is_none() && period_days > 0 { (behind_days / (period_days * self.interval) - 1).max(0) } else { 0 };
        let mut out = Vec::new();
        let time = start.time();
        for period in first..first + MAX_STEPS as i64 {
            let mut batch = self.period(start, period * self.interval);
            batch.sort();
            let past_end = batch.first().is_some_and(|t| unix(*t) >= to) || batch.is_empty() && period > 400;
            for d in batch {
                let t = d.date().and_time(time);
                if t < start {
                    continue;
                }
                if until.is_some_and(|u| unix(t) > u) || self.count.is_some_and(|c| out.len() >= c) || unix(t) >= to {
                    return out;
                }
                out.push(t);
            }
            if past_end || out.len() >= MAX_STEPS {
                break;
            }
        }
        out
    }

    /// Candidate days in the `n`-th period (day, week, month or year) after the start.
    fn period(&self, start: NaiveDateTime, n: i64) -> Vec<NaiveDateTime> {
        let day0 = start.date();
        let at = |d: NaiveDate| d.and_time(start.time());
        match self.freq.as_str() {
            "DAILY" => vec![at(day0 + Duration::days(n))],
            "WEEKLY" => {
                let monday = day0 - Duration::days(day0.weekday().num_days_from_monday() as i64) + Duration::weeks(n);
                let days: Vec<Weekday> = if self.by_day.is_empty() { vec![day0.weekday()] } else { self.by_day.iter().map(|(_, w)| *w).collect() };
                days.into_iter().map(|w| at(monday + Duration::days(w.num_days_from_monday() as i64))).collect()
            }
            "MONTHLY" => {
                let total = day0.year() as i64 * 12 + day0.month0() as i64 + n;
                let (y, m) = ((total / 12) as i32, (total % 12) as u32 + 1);
                self.days_in_month(y, m, day0).into_iter().map(at).collect()
            }
            "YEARLY" => {
                let y = day0.year() + n as i32;
                let months = if self.by_month.is_empty() { vec![day0.month()] } else { self.by_month.clone() };
                months.into_iter().flat_map(|m| self.days_in_month(y, m, day0)).map(at).collect()
            }
            _ => Vec::new(),
        }
    }

    fn days_in_month(&self, y: i32, m: u32, day0: NaiveDate) -> Vec<NaiveDate> {
        if !self.by_day.is_empty() {
            return self
                .by_day
                .iter()
                .flat_map(|(n, w)| if *n == 0 { (1..=5).filter_map(|k| nth_weekday(y, m, k, *w)).collect() } else { nth_weekday(y, m, *n, *w).into_iter().collect::<Vec<_>>() })
                .collect();
        }
        let days = if self.by_month_day.is_empty() { vec![day0.day() as i32] } else { self.by_month_day.clone() };
        let last = last_day(y, m);
        days.into_iter()
            .filter_map(|d| {
                let d = if d < 0 { last as i32 + 1 + d } else { d };
                (d >= 1).then(|| NaiveDate::from_ymd_opt(y, m, d as u32)).flatten()
            })
            .collect()
    }
}

fn parse_by_day(s: &str) -> Option<(i32, Weekday)> {
    let s = s.trim();
    let (num, code) = s.split_at(s.len().checked_sub(2)?);
    let wd = match code.to_ascii_uppercase().as_str() {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    };
    Some((if num.is_empty() { 0 } else { num.trim_start_matches('+').parse().ok()? }, wd))
}

fn last_day(y: i32, m: u32) -> u32 {
    let next = if m == 12 { NaiveDate::from_ymd_opt(y + 1, 1, 1) } else { NaiveDate::from_ymd_opt(y, m + 1, 1) };
    next.map(|d| (d - Duration::days(1)).day()).unwrap_or(28)
}

/// The `n`-th weekday `wd` of a month; negative counts from the end (-1 = last).
fn nth_weekday(y: i32, m: u32, n: i32, wd: Weekday) -> Option<NaiveDate> {
    if n > 0 {
        let first = NaiveDate::from_ymd_opt(y, m, 1)?;
        let shift = (7 + wd.num_days_from_monday() as i64 - first.weekday().num_days_from_monday() as i64) % 7;
        let d = first + Duration::days(shift + 7 * (n as i64 - 1));
        (d.month() == m).then_some(d)
    } else if n < 0 {
        let last = NaiveDate::from_ymd_opt(y, m, last_day(y, m))?;
        let shift = (7 + last.weekday().num_days_from_monday() as i64 - wd.num_days_from_monday() as i64) % 7;
        let d = last - Duration::days(shift + 7 * (-n as i64 - 1));
        (d.month() == m).then_some(d)
    } else {
        None
    }
}

// ---------------------------------------------------------------- values

fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        match raw.strip_prefix([' ', '\t']) {
            Some(rest) if !lines.is_empty() => lines.last_mut().unwrap().push_str(rest),
            _ => lines.push(raw.to_string()),
        }
    }
    lines
}

fn parse_prop(line: &str) -> Option<Prop> {
    let mut quoted = false;
    let colon = line.char_indices().find(|&(_, c)| {
        if c == '"' {
            quoted = !quoted;
        }
        c == ':' && !quoted
    })?;
    let (head, value) = (&line[..colon.0], &line[colon.0 + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    let params = parts.filter_map(|p| p.split_once('=')).map(|(k, v)| (k.trim().to_ascii_uppercase(), v.trim_matches('"').to_string())).collect();
    Some(Prop { name, params, value: value.to_string() })
}

fn get<'a>(props: &'a [Prop], name: &str) -> Option<&'a str> {
    props.iter().find(|p| p.name == name).map(|p| p.value.as_str())
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push(' '),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn when(p: &Prop) -> Option<When> {
    let v = p.value.trim();
    if p.param("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE")) {
        return parse_date(v).map(When::Date);
    }
    parse_when(v, p.param("TZID"))
}

fn parse_when(v: &str, tzid: Option<&str>) -> Option<When> {
    let v = v.trim();
    if v.len() == 8 {
        return parse_date(v).map(When::Date);
    }
    let (body, utc) = match v.strip_suffix(['Z', 'z']) {
        Some(b) => (b, true),
        None => (v, false),
    };
    let t = NaiveDateTime::parse_from_str(body, "%Y%m%dT%H%M%S").ok()?;
    Some(match (utc, tzid) {
        (true, _) => When::Utc(t),
        (false, Some(z)) => When::Zoned(t, z.to_string()),
        (false, None) => When::Floating(t),
    })
}

fn parse_date(v: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(v.get(..8)?, "%Y%m%d").ok()
}

/// "P1D", "PT1H30M", "P1W" and similar.
fn parse_duration(v: &str) -> Option<Duration> {
    let (neg, v) = match v.trim().strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, v.trim().trim_start_matches('+')),
    };
    let v = v.strip_prefix('P')?;
    let mut total = Duration::zero();
    let mut num = String::new();
    let mut in_time = false;
    for c in v.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' => num.push(c),
            unit => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += match (unit, in_time) {
                    ('W', _) => Duration::weeks(n),
                    ('D', _) => Duration::days(n),
                    ('H', true) => Duration::hours(n),
                    ('M', true) => Duration::minutes(n),
                    ('S', true) => Duration::seconds(n),
                    _ => return None,
                };
            }
        }
    }
    Some(if neg { -total } else { total })
}

fn parse_offset(v: &str) -> Option<i32> {
    let v = v.trim();
    let sign = if v.starts_with('-') { -1 } else { 1 };
    let digits = v.trim_start_matches(['+', '-']);
    let h: i32 = digits.get(0..2)?.parse().ok()?;
    let m: i32 = digits.get(2..4)?.parse().ok()?;
    let s: i32 = digits.get(4..6).and_then(|s| s.parse().ok()).unwrap_or(0);
    Some(sign * (h * 3600 + m * 60 + s))
}

fn tz_rule(props: &[Prop]) -> Option<TzRule> {
    let offset_to = parse_offset(get(props, "TZOFFSETTO")?)?;
    let start = parse_when(get(props, "DTSTART")?, None)?.naive();
    let yearly = get(props, "RRULE").map(Rule::parse).and_then(|r| {
        let month = *r.by_month.first()?;
        let (n, wd) = *r.by_day.first()?;
        Some((month, if n == 0 { 1 } else { n }, wd))
    });
    Some(TzRule { offset_to, start, yearly })
}

fn to_unix(w: &When, zones: &HashMap<String, Zone>) -> i64 {
    let local = |t: NaiveDateTime| Local.from_local_datetime(&t).earliest().map(|d| d.timestamp()).unwrap_or_else(|| t.and_utc().timestamp());
    match w {
        When::Date(d) => local(d.and_time(NaiveTime::MIN)),
        When::Floating(t) => local(*t),
        When::Utc(t) => t.and_utc().timestamp(),
        When::Zoned(t, z) => match zones.get(z).and_then(|zone| zone.offset_at(*t)) {
            Some(offset) => t.and_utc().timestamp() - offset as i64,
            None if z.eq_ignore_ascii_case("UTC") || z.eq_ignore_ascii_case("Etc/UTC") => t.and_utc().timestamp(),
            None => local(*t),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap().and_utc().timestamp()
    }

    const RANGE: (i64, i64) = (1_700_000_000, 1_900_000_000);

    fn cal(body: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{body}END:VCALENDAR\r\n")
    }

    #[test]
    fn reads_utc_event_with_folded_escaped_title() {
        let text = cal("BEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Team sync\\, weekly\r\n  notes\r\nDTSTART:20261030T140000Z\r\nDTEND:20261030T143000Z\r\nEND:VEVENT\r\n");
        let ev = parse(&text, RANGE.0, RANGE.1);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].title, "Team sync, weekly notes");
        assert_eq!((ev[0].start, ev[0].end, ev[0].all_day), (utc(2026, 10, 30, 14, 0), utc(2026, 10, 30, 14, 30), false));
    }

    #[test]
    fn zoned_times_follow_the_files_daylight_rules() {
        let tz = "BEGIN:VTIMEZONE\r\nTZID:Europe/Berlin\r\nBEGIN:DAYLIGHT\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200\r\nDTSTART:19700329T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r\nEND:DAYLIGHT\r\nBEGIN:STANDARD\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nDTSTART:19701025T030000\r\nRRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n";
        let ev = "BEGIN:VEVENT\r\nUID:b\r\nSUMMARY:Standup\r\nDTSTART;TZID=Europe/Berlin:20261023T090000\r\nDURATION:PT15M\r\nRRULE:FREQ=WEEKLY;COUNT=2\r\nEND:VEVENT\r\n";
        let out = parse(&cal(&format!("{tz}{ev}")), RANGE.0, RANGE.1);
        // Summer time on 23 Oct (UTC+2), winter time after 25 Oct (UTC+1).
        assert_eq!(out.iter().map(|e| e.start).collect::<Vec<_>>(), vec![utc(2026, 10, 23, 7, 0), utc(2026, 10, 30, 8, 0)]);
        assert_eq!(out[0].end - out[0].start, 15 * 60);
    }

    #[test]
    fn weekly_by_day_with_exception_and_moved_occurrence() {
        let ev = "BEGIN:VEVENT\r\nUID:c\r\nSUMMARY:Gym\r\nDTSTART:20261102T180000Z\r\nDTEND:20261102T190000Z\r\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE;UNTIL=20261111T235959Z\r\nEXDATE:20261104T180000Z\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:c\r\nSUMMARY:Gym (moved)\r\nRECURRENCE-ID:20261109T180000Z\r\nDTSTART:20261110T070000Z\r\nDTEND:20261110T080000Z\r\nEND:VEVENT\r\n";
        let out = parse(&cal(ev), RANGE.0, RANGE.1);
        let got: Vec<(i64, &str)> = out.iter().map(|e| (e.start, e.title.as_str())).collect();
        assert_eq!(got, vec![(utc(2026, 11, 2, 18, 0), "Gym"), (utc(2026, 11, 10, 7, 0), "Gym (moved)"), (utc(2026, 11, 11, 18, 0), "Gym")]);
    }

    #[test]
    fn monthly_nth_weekday_and_cancelled_events() {
        let ev = "BEGIN:VEVENT\r\nUID:d\r\nSUMMARY:Review\r\nDTSTART:20260109T100000Z\r\nRRULE:FREQ=MONTHLY;BYDAY=-1FR;COUNT=3\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:e\r\nSTATUS:CANCELLED\r\nDTSTART:20260110T100000Z\r\nEND:VEVENT\r\n";
        let starts: Vec<i64> = parse(&cal(ev), RANGE.0, RANGE.1).iter().map(|e| e.start).collect();
        // Only the rule's own days count (last Fridays), as calendar apps write them.
        assert_eq!(starts, vec![utc(2026, 1, 30, 10, 0), utc(2026, 2, 27, 10, 0), utc(2026, 3, 27, 10, 0)]);
    }

    #[test]
    fn all_day_events_and_window() {
        let ev = "BEGIN:VEVENT\r\nUID:f\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20261224\r\nDTEND;VALUE=DATE:20261226\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:g\r\nSUMMARY:Long ago\r\nDTSTART:20100101T000000Z\r\nEND:VEVENT\r\n";
        let out = parse(&cal(ev), RANGE.0, RANGE.1);
        assert_eq!(out.len(), 1);
        let day = |d: u32| Local.from_local_datetime(&NaiveDate::from_ymd_opt(2026, 12, d).unwrap().and_time(NaiveTime::MIN)).earliest().unwrap().timestamp();
        assert_eq!((out[0].start, out[0].end, out[0].all_day), (day(24), day(26), true));
    }

    #[test]
    fn old_daily_series_still_reaches_the_window() {
        let ev = "BEGIN:VEVENT\r\nUID:h\r\nSUMMARY:Journal\r\nDTSTART:20000101T200000Z\r\nRRULE:FREQ=DAILY\r\nEND:VEVENT\r\n";
        let from = utc(2026, 10, 30, 0, 0);
        let out = parse(&cal(ev), from, from + 3 * 86_400);
        assert_eq!(out.iter().map(|e| e.start).collect::<Vec<_>>(), vec![utc(2026, 10, 30, 20, 0), utc(2026, 10, 31, 20, 0), utc(2026, 11, 1, 20, 0)]);
    }

    #[test]
    fn values_parse() {
        assert_eq!(parse_duration("PT1H30M"), Some(Duration::minutes(90)));
        assert_eq!(parse_duration("P1W"), Some(Duration::days(7)));
        assert_eq!(parse_offset("-0530"), Some(-(5 * 3600 + 30 * 60)));
        assert_eq!(nth_weekday(2026, 3, -1, Weekday::Sun), NaiveDate::from_ymd_opt(2026, 3, 29));
        assert_eq!(nth_weekday(2026, 3, 2, Weekday::Sun), NaiveDate::from_ymd_opt(2026, 3, 8));
        assert_eq!(nth_weekday(2026, 2, 5, Weekday::Mon), None);
    }
}
