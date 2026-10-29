//! The notch's pages, overlays, alerts and click targets.

use chrono::NaiveDate;

use crate::pomodoro::Phase;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Page {
    Music,
    Timer,
    Water,
    Shelf,
    Todo,
    Calendar,
    Stats,
}

impl Page {
    pub const ALL: [Page; 7] = [Page::Music, Page::Timer, Page::Water, Page::Shelf, Page::Todo, Page::Calendar, Page::Stats];
    pub fn icon(self) -> char {
        use crate::gfx::icon;
        match self {
            Page::Music => icon::MUSIC,
            Page::Timer => icon::TIMER,
            Page::Water => '\u{E81E}',
            Page::Shelf => icon::FOLDER,
            Page::Todo => icon::CHECK,
            Page::Calendar => icon::CALENDAR,
            Page::Stats => icon::CHART,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Page::Music => "Music",
            Page::Timer => "Focus",
            Page::Water => "Water",
            Page::Shelf => "Shelf",
            Page::Todo => "To-do",
            Page::Calendar => "Calendar",
            Page::Stats => "Stats",
        }
    }
    /// Expanded size in DIPs.
    pub fn size(self, lyrics_view: bool) -> (f32, f32) {
        match self {
            Page::Music if lyrics_view => (680.0, 214.0),
            Page::Music => (580.0, 190.0),
            Page::Timer => (560.0, 212.0),
            Page::Water => (600.0, 204.0),
            Page::Shelf => (620.0, 200.0),
            Page::Todo => (600.0, 250.0),
            Page::Calendar => (640.0, 268.0),
            Page::Stats => (600.0, 226.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Hud {
    Volume { level: f32, muted: bool },
    Brightness(u8),
    Caps(bool),
    Language { short: String, name: String },
    Battery { percent: u8, charging: bool },
    Wifi { connected: bool, name: String },
    Headphones { name: String, connected: bool },
    /// Something new reached the clipboard history: its first line, or an image's size.
    Clipboard { image: bool, preview: String },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AlertKind {
    Todo(u64),
    Water,
    Pomodoro(Phase),
    /// The plain countdown on the Focus tab finished.
    Timer,
    /// A calendar event starts soon (its occurrence id).
    Event(u64),
    LowBattery(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hit {
    Body,
    PageDot(Page),
    Settings,
    Prev,
    PlayPause,
    Next,
    Seek,
    /// Cycles to the next open player.
    Source,
    LyricsToggle,
    Volume,
    VolumeBar,
    LyricLine(u64),
    /// Lyrics shift: show lines earlier or later than the song says.
    LyricsEarlier,
    LyricsLater,
    LyricsRetry,
    FocusMode(u8),
    TimerPreset(u8),
    TimerStep(i8),
    TimerToggle,
    TimerReset,
    WatchToggle,
    WatchLap,
    WatchReset,
    PomoToggle,
    PomoReset,
    PomoSkip,
    PomoPhase(u8),
    Drink,
    WaterUndo,
    ShelfItem(usize),
    /// 0 shows the shelf, 1 the clipboard history.
    ShelfTab(u8),
    ClipItem(usize),
    ClipClear,
    ShelfSelect,
    ShelfClear,
    ShelfRemove,
    TodoCheck(u64),
    /// The whole task row, under its controls, so moving between them keeps them visible.
    TodoRow(u64),
    TodoTitle(u64),
    TodoColor(u64),
    TodoDue(u64),
    TodoDelete(u64),
    TodoAdd,
    TodoField,
    NewColor,
    DueChip(u8),
    DueStep(i8),
    DueClear,
    ShowDone,
    CalPrev,
    CalNext,
    CalToday,
    CalDay(NaiveDate),
    CalAdd,
    AlertPrimary,
    AlertSecondary,
    AlertClose,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditTarget {
    NewTask,
    Rename(u64),
}

#[derive(Clone, Debug)]
pub struct TextField {
    pub target: EditTarget,
    pub text: String,
    pub caret: usize,
}
