//! The captain's own reminders: "remind me in 20 minutes to check the deploy". The chief of
//! staff sets one with the app's `app_remind` tool and lists them with `app_reminders`; the app,
//! not the model, turns the captain's words into a time, so the same words always mean the same
//! time. A reminder that comes due shows in the window, and in the system's notifications where
//! the app may post them, until the captain marks it done, snoozes it or deletes it.
//!
//! Times are local wall-clock times in the system time zone, worked out with `jiff`, so a
//! reminder for 9:00 AM is at 9:00 AM on both sides of a daylight saving change. These are the
//! captain's personal reminders inside the app: they are never firstmate tasks.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use jiff::civil::{Date, DateTime, Weekday};
use jiff::{SignedDuration, Timestamp, ToSpan, Zoned};
use serde_json::{Value, json};

use crate::store::{self, Saved, Shared};

/// The tool that sets a reminder, and the one that lists them.
pub const TOOL: &str = "app_remind";
pub const LIST_TOOL: &str = "app_reminders";
/// The reminders, in the app's data directory.
pub const FILE: &str = "reminders.json";
pub const SETTINGS_FILE: &str = "reminder-settings.json";
/// The longest reminder text, in characters.
pub const MAX_TEXT: usize = 200;
/// The most reminders that may wait at once, the Later list included.
pub const MAX_PENDING: usize = 100;
/// How far ahead a reminder may be set.
const HORIZON_DAYS: i64 = 366;
/// A reminder that fires this much after its time fired late: the app was not running.
const LATE: SignedDuration = SignedDuration::from_secs(60);

/// What the app reads the time from: the system clock, or a fixed one in tests and scenes.
pub type Clock = Arc<dyn Fn() -> Zoned + Send + Sync>;

pub fn system_clock() -> Clock {
    Arc::new(Zoned::now)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Minutes,
    Hours,
    Days,
}

impl Unit {
    pub const ALL: [Unit; 3] = [Unit::Minutes, Unit::Hours, Unit::Days];

    pub fn key(self) -> &'static str {
        match self {
            Unit::Minutes => "minutes",
            Unit::Hours => "hours",
            Unit::Days => "days",
        }
    }

    fn from_key(key: &str) -> Option<Unit> {
        match key.trim_end_matches('s') {
            "minute" => Some(Unit::Minutes),
            "hour" => Some(Unit::Hours),
            "day" => Some(Unit::Days),
            _ => None,
        }
    }
}

/// When the captain asked to be reminded, in the forms the tool takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    /// Minutes and hours are exact; days keep the clock time, n calendar days on.
    In(i64, Unit),
    LaterToday,
    TomorrowMorning,
    NextWeek,
    /// No time: the Later list.
    Later,
    /// A local clock time the model worked out, for a time the forms above cannot say.
    At(DateTime),
}

impl When {
    /// The snooze menu's fixed choices, keyed for the view.
    pub const SNOOZES: [(&'static str, &'static str, When); 6] = [
        ("10m", "10 minutes", When::In(10, Unit::Minutes)),
        ("1h", "1 hour", When::In(1, Unit::Hours)),
        ("later-today", "Later today", When::LaterToday),
        ("tomorrow", "Tomorrow morning", When::TomorrowMorning),
        ("next-week", "Next week", When::NextWeek),
        ("later", "Later list", When::Later),
    ];
}

/// When a reminder comes due: a moment, or each morning from the Later list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Due {
    At(Timestamp),
    Later,
}

/// The settings that turn the keywords into times.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// `later_today` is this many hours from now...
    pub later_today_hours: i64,
    /// ...but no later than this, in minutes after midnight...
    pub evening: i64,
    /// ...and once that has passed, this many hours from now.
    pub after_evening_hours: i64,
    /// `tomorrow_morning`, in minutes after midnight.
    pub morning: i64,
    /// `next_week`: the next one of this day...
    pub week_day: Weekday,
    /// ...at this time.
    pub week_time: i64,
    /// The Later list comes back each day at this time.
    pub later_time: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            later_today_hours: 3,
            evening: 18 * 60,
            after_evening_hours: 2,
            morning: 9 * 60,
            week_day: Weekday::Monday,
            week_time: 9 * 60,
            later_time: 9 * 60,
        }
    }
}

/// One setting, for the settings page and the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    LaterTodayHours,
    Evening,
    AfterEveningHours,
    Morning,
    WeekDay,
    WeekTime,
    LaterTime,
}

impl Setting {
    pub const ALL: [Setting; 7] = [
        Setting::LaterTodayHours,
        Setting::Evening,
        Setting::AfterEveningHours,
        Setting::Morning,
        Setting::WeekDay,
        Setting::WeekTime,
        Setting::LaterTime,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Setting::LaterTodayHours => "later-today-hours",
            Setting::Evening => "evening",
            Setting::AfterEveningHours => "after-evening-hours",
            Setting::Morning => "morning",
            Setting::WeekDay => "week-day",
            Setting::WeekTime => "week-time",
            Setting::LaterTime => "later-time",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Setting::LaterTodayHours => "Later today is this far from now",
            Setting::Evening => "But no later than",
            Setting::AfterEveningHours => "Once that has passed, this far from now",
            Setting::Morning => "Tomorrow morning is at",
            Setting::WeekDay => "Next week is the next",
            Setting::WeekTime => "Next week at",
            Setting::LaterTime => "The Later list comes back each day at",
        }
    }

    /// The lowest and highest value, and the step of the page's buttons.
    fn range(self) -> (i64, i64, i64) {
        match self {
            Setting::LaterTodayHours => (1, 12, 1),
            Setting::AfterEveningHours => (1, 6, 1),
            Setting::Evening => (12 * 60, 23 * 60 + 30, 30),
            Setting::WeekDay => (1, 7, 1),
            Setting::Morning | Setting::WeekTime | Setting::LaterTime => (5 * 60, 12 * 60, 30),
        }
    }
}

impl Settings {
    pub fn get(&self, setting: Setting) -> i64 {
        match setting {
            Setting::LaterTodayHours => self.later_today_hours,
            Setting::Evening => self.evening,
            Setting::AfterEveningHours => self.after_evening_hours,
            Setting::Morning => self.morning,
            Setting::WeekDay => self.week_day.to_monday_one_offset().into(),
            Setting::WeekTime => self.week_time,
            Setting::LaterTime => self.later_time,
        }
    }

    /// Sets `setting` to `value`, or does nothing when it is out of range.
    fn set(&mut self, setting: Setting, value: i64) -> bool {
        let (low, high, _) = setting.range();
        if !(low..=high).contains(&value) {
            return false;
        }
        match setting {
            Setting::LaterTodayHours => self.later_today_hours = value,
            Setting::Evening => self.evening = value,
            Setting::AfterEveningHours => self.after_evening_hours = value,
            Setting::Morning => self.morning = value,
            Setting::WeekDay => {
                self.week_day = Weekday::from_monday_one_offset(value as i8).expect("in range")
            }
            Setting::WeekTime => self.week_time = value,
            Setting::LaterTime => self.later_time = value,
        }
        true
    }

    /// Moves `setting` one step up or down; the day of the week goes round.
    pub fn step(&mut self, setting: Setting, up: bool) {
        let (low, high, step) = setting.range();
        let mut next = self.get(setting) + if up { step } else { -step };
        if setting == Setting::WeekDay {
            next = (next - low).rem_euclid(high) + low;
        }
        self.set(setting, next);
    }

    /// The setting as the page shows it, such as "3 hours", "6:00 PM" or "Monday".
    pub fn shown(&self, setting: Setting) -> String {
        let value = self.get(setting);
        match setting {
            Setting::LaterTodayHours | Setting::AfterEveningHours => {
                format!("{value} hour{}", if value == 1 { "" } else { "s" })
            }
            Setting::WeekDay => WEEKDAYS[self.week_day.to_monday_zero_offset() as usize].into(),
            _ => clock_words(value),
        }
    }

    /// When `when`, said at `now`, comes due, or why it cannot be set.
    pub fn resolve(&self, when: When, now: &Zoned) -> Result<Due, String> {
        let today = now.date();
        let at = match when {
            When::Later => return Ok(Due::Later),
            When::In(amount, _) if amount < 1 => {
                return Err("The amount must be a whole number of 1 or more.".into());
            }
            When::In(amount, unit) => {
                let per_day = match unit {
                    Unit::Minutes => 24 * 60,
                    Unit::Hours => 24,
                    Unit::Days => 1,
                };
                if amount > HORIZON_DAYS * per_day {
                    return Err(too_far());
                }
                // Minutes and hours are exact durations; days are calendar days in the zone.
                let later = match unit {
                    Unit::Minutes => now.checked_add(SignedDuration::from_mins(amount)),
                    Unit::Hours => now.checked_add(SignedDuration::from_hours(amount)),
                    Unit::Days => now.checked_add(amount.days()),
                };
                later.map_err(bad)?.timestamp()
            }
            When::LaterToday => {
                let now_at = now.timestamp();
                let evening = local(now, today, self.evening)?;
                if now_at >= evening {
                    after(now_at, self.after_evening_hours)?
                } else {
                    after(now_at, self.later_today_hours)?.min(evening)
                }
            }
            When::TomorrowMorning => local(now, today.tomorrow().map_err(bad)?, self.morning)?,
            When::NextWeek => local(
                now,
                today.nth_weekday(1, self.week_day).map_err(bad)?,
                self.week_time,
            )?,
            When::At(datetime) => {
                let at = datetime
                    .to_zoned(now.time_zone().clone())
                    .map_err(bad)?
                    .timestamp();
                if at <= now.timestamp() {
                    return Err(format!(
                        "{} has already passed: it is now {}.",
                        words(at, now),
                        words(now.timestamp(), now)
                    ));
                }
                at
            }
        };
        if at.duration_since(now.timestamp()) > SignedDuration::from_hours(HORIZON_DAYS * 24) {
            return Err(too_far());
        }
        Ok(Due::At(at))
    }

    /// When the Later list comes back on `date`.
    fn later_on(&self, now: &Zoned, date: Date) -> Result<Timestamp, String> {
        local(now, date, self.later_time)
    }
}

fn too_far() -> String {
    format!("A reminder can be at most {HORIZON_DAYS} days ahead.")
}

fn bad(e: jiff::Error) -> String {
    format!("That time could not be worked out: {e}.")
}

fn after(at: Timestamp, hours: i64) -> Result<Timestamp, String> {
    at.checked_add(SignedDuration::from_hours(hours))
        .map_err(bad)
}

/// `minutes` after midnight on `date`, in `now`'s time zone. A time a daylight saving change
/// skips is moved forward by the gap, and one it repeats is the first of the two.
fn local(now: &Zoned, date: Date, minutes: i64) -> Result<Timestamp, String> {
    date.at((minutes / 60) as i8, (minutes % 60) as i8, 0, 0)
        .to_zoned(now.time_zone().clone())
        .map(|z| z.timestamp())
        .map_err(bad)
}

const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `minutes` after midnight as "9:00 AM".
pub fn clock_words(minutes: i64) -> String {
    let (hour, minute) = (minutes / 60 % 24, minutes % 60);
    let twelve = if hour % 12 == 0 { 12 } else { hour % 12 };
    let half = if hour < 12 { "AM" } else { "PM" };
    format!("{twelve}:{minute:02} {half}")
}

/// `at` in plain words from `now`'s side, in its time zone: "Today, 3:20 PM", "Tomorrow, 9:00
/// AM" or "Monday, Nov 16, 9:00 AM".
pub fn words(at: Timestamp, now: &Zoned) -> String {
    let at = at.to_zoned(now.time_zone().clone());
    let (date, today) = (at.date(), now.date());
    let day = if date == today {
        "Today".to_string()
    } else if today.tomorrow().ok() == Some(date) {
        "Tomorrow".to_string()
    } else if today.yesterday().ok() == Some(date) {
        "Yesterday".to_string()
    } else {
        let year = if date.year() == today.year() {
            String::new()
        } else {
            format!(", {}", date.year())
        };
        format!(
            "{}, {} {}{year}",
            WEEKDAYS[date.weekday().to_monday_zero_offset() as usize],
            MONTHS[date.month() as usize - 1],
            date.day()
        )
    };
    format!(
        "{day}, {}",
        clock_words(i64::from(at.hour()) * 60 + i64::from(at.minute()))
    )
}

/// One reminder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder {
    pub id: u64,
    /// What to remind about, in the captain's words, as plain text.
    pub text: String,
    pub set_at: Timestamp,
    pub due: Due,
    /// When it came due and showed. It shows until the captain marks it done, snoozes it or
    /// deletes it.
    pub fired: Option<Timestamp>,
    /// On the Later list: the day it last came back, or was set after that day's time, so it
    /// comes back once a day.
    pub surfaced: Option<Date>,
}

impl Reminder {
    /// It came due while the app was not running, and showed at the next launch.
    pub fn overdue(&self) -> bool {
        match (self.due, self.fired) {
            (Due::At(due), Some(fired)) => fired.duration_since(due) > LATE,
            _ => false,
        }
    }

    /// When it is or was due, in words, such as "Today, 3:20 PM" or "Later list".
    pub fn due_words(&self, now: &Zoned) -> String {
        match self.due {
            Due::At(at) => words(at, now),
            Due::Later => "Later list".into(),
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "text": self.text,
            "set_at": self.set_at.to_string(),
            "due": match self.due {
                Due::At(at) => at.to_string(),
                Due::Later => "later".into(),
            },
            "fired": self.fired.map(|t| t.to_string()),
            "surfaced": self.surfaced.map(|d| d.to_string()),
        })
    }

    fn from_json(value: &Value) -> Option<Reminder> {
        let time = |v: &Value| v.as_str()?.parse::<Timestamp>().ok();
        Some(Reminder {
            id: value["id"].as_u64()?,
            text: clean(value["text"].as_str()?).ok()?,
            set_at: time(&value["set_at"])?,
            due: match value["due"].as_str()? {
                "later" => Due::Later,
                _ => Due::At(time(&value["due"])?),
            },
            fired: match &value["fired"] {
                Value::Null => None,
                v => Some(time(v)?),
            },
            surfaced: match &value["surfaced"] {
                Value::Null => None,
                v => Some(v.as_str()?.parse().ok()?),
            },
        })
    }
}

/// The reminder text as it is kept and shown: one line of plain text, without control or
/// direction-changing characters, at most [`MAX_TEXT`] characters.
pub fn clean(text: &str) -> Result<String, String> {
    let line = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        .collect::<String>();
    match line.chars().count() {
        0 => Err("The reminder has no text.".into()),
        n if n > MAX_TEXT => Err(format!(
            "The reminder is {n} characters long; say it in {MAX_TEXT} or fewer."
        )),
        _ => Ok(line),
    }
}

/// Reads the tool's arguments: the text, and the when in one of its forms.
pub fn parse(arguments: &Value) -> Result<(String, When), String> {
    let text = clean(arguments["text"].as_str().unwrap_or_default())?;
    let when = match arguments["when"].as_str().unwrap_or_default() {
        "in" => {
            let amount = match &arguments["amount"] {
                Value::Number(n) => n.as_i64(),
                Value::String(s) => s.trim().parse().ok(),
                _ => None,
            }
            .ok_or("With when \"in\", give amount as a whole number.")?;
            let unit = arguments["unit"]
                .as_str()
                .and_then(Unit::from_key)
                .ok_or("With when \"in\", give unit as minutes, hours or days.")?;
            When::In(amount, unit)
        }
        "later_today" => When::LaterToday,
        "tomorrow_morning" => When::TomorrowMorning,
        "next_week" => When::NextWeek,
        "later" => When::Later,
        "at" => When::At(
            arguments["at"]
                .as_str()
                .and_then(|at| at.trim().parse().ok())
                .ok_or("With when \"at\", give at as a local date and time, YYYY-MM-DDTHH:MM.")?,
        ),
        _ => {
            return Err(
                "when must be one of in, later_today, tomorrow_morning, next_week, later \
                        or at."
                    .into(),
            );
        }
    };
    Ok((text, when))
}

/// The `app_remind` tool's description: the forms, compactly, so the model needs no more
/// prompting.
pub fn description() -> &'static str {
    "Set a reminder for the captain in Velvet Hammer when they say \"remind me ...\". The app \
     works out the time. text: what to remind them about, in their words. when, from their \
     words: \"in\" with amount and unit (minutes, hours or days) for \"in 20 minutes\", \"in 2 \
     hours\", \"in 3 days\"; \"later_today\"; \"tomorrow_morning\"; \"next_week\"; \"later\" with \
     no time, which puts it on their Later list; \"at\" with at = local YYYY-MM-DDTHH:MM only for \
     a clock time or date none of the others say. Examples: \"remind me in 20 minutes to check \
     the deploy\" is {text: \"check the deploy\", when: \"in\", amount: 20, unit: \"minutes\"}; \
     \"remind me tomorrow morning about the standup\" is {text: \"the standup\", when: \
     \"tomorrow_morning\"}; \"remind me later\" is when: \"later\". Tell the captain the time the \
     result names. These are the captain's own reminders in the app, not firstmate tasks: never \
     create a task for one."
}

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "text": {"type": "string", "description": "What to remind the captain about, in their words."},
            "when": {"type": "string", "enum": ["in", "later_today", "tomorrow_morning", "next_week", "later", "at"]},
            "amount": {"type": "integer", "minimum": 1, "description": "With when \"in\": how many."},
            "unit": {"type": "string", "enum": ["minutes", "hours", "days"], "description": "With when \"in\"."},
            "at": {"type": "string", "description": "With when \"at\": local date and time, YYYY-MM-DDTHH:MM."},
        },
        "required": ["text", "when"],
    })
}

pub fn list_description() -> &'static str {
    "List the captain's reminders in Velvet Hammer, due first, with the Later list last. Read \
     only. Use it when they ask what reminders they have."
}

/// The reminders, kept in the data directory and shared by the app and the chat session's
/// tools. Every change is saved at once.
#[derive(Clone)]
pub struct Reminders {
    book: Arc<Mutex<Book>>,
    settings: Shared<Settings>,
    clock: Clock,
}

struct Book {
    /// None for reminders kept in memory only.
    path: Option<PathBuf>,
    list: Vec<Reminder>,
    next_id: u64,
    /// Why the file could not be read or saved.
    problem: Option<String>,
    /// The file exists but could not be read; the next save keeps it as `<file>.bad`.
    unreadable: bool,
    /// Counts changes, so the app sees one made by the chat session.
    version: u64,
}

impl Reminders {
    /// Reads `dir/reminders.json`. A missing file has no reminders; one that cannot be read
    /// shows why and is kept as `.bad` when the next change saves over it.
    pub fn open(dir: &Path, settings: Shared<Settings>, clock: Clock) -> Reminders {
        let path = dir.join(FILE);
        let read = match fs::read(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e.to_string()),
            Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
                .map_err(|_| "it is not valid JSON".to_string())
                .and_then(|v| read_list(&v)),
        };
        let (list, problem) = match read {
            Ok(list) => (list, None),
            Err(why) => (
                Vec::new(),
                Some(format!(
                    "Velvet Hammer could not read {} ({why}), so no reminders are shown. The \
                     next change keeps the old file as {FILE}.bad.",
                    path.display()
                )),
            ),
        };
        Reminders::with(Some(path), list, problem, settings, clock)
    }

    /// Reminders kept in memory only, with the default settings.
    pub fn in_memory(clock: Clock) -> Reminders {
        let settings = Arc::new(RwLock::new(Settings::default()));
        Reminders::with(None, Vec::new(), None, settings, clock)
    }

    fn with(
        path: Option<PathBuf>,
        list: Vec<Reminder>,
        problem: Option<String>,
        settings: Shared<Settings>,
        clock: Clock,
    ) -> Reminders {
        let next_id = list.iter().map(|r| r.id).max().unwrap_or(0) + 1;
        Reminders {
            book: Arc::new(Mutex::new(Book {
                path,
                list,
                next_id,
                unreadable: problem.is_some(),
                problem,
                version: 0,
            })),
            settings,
            clock,
        }
    }

    fn book(&self) -> MutexGuard<'_, Book> {
        self.book.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn settings(&self) -> Settings {
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn now(&self) -> Zoned {
        (self.clock)()
    }

    /// Every reminder: those that came due first, oldest first, then those still to come by
    /// their time, then the Later list in the order it was made.
    pub fn list(&self) -> Vec<Reminder> {
        let mut list = self.book().list.clone();
        list.sort_by_key(|r| match (r.fired, r.due) {
            (Some(fired), _) => (0, fired, r.id),
            (None, Due::At(at)) => (1, at, r.id),
            (None, Due::Later) => (2, r.set_at, r.id),
        });
        list
    }

    pub fn version(&self) -> u64 {
        self.book().version
    }

    pub fn problem(&self) -> Option<String> {
        self.book().problem.clone()
    }

    /// Sets a reminder, or says why not. The answer states the time in plain words.
    pub fn set(&self, text: &str, when: When) -> Result<String, String> {
        let text = clean(text)?;
        let now = self.now();
        let settings = self.settings();
        let due = settings.resolve(when, &now)?;
        let mut book = self.book();
        let waiting = book.list.iter().filter(|r| r.fired.is_none()).count();
        if waiting >= MAX_PENDING {
            return Err(format!(
                "The captain already has {MAX_PENDING} reminders waiting, the most Velvet Hammer \
                 keeps. Ask them to clear some first."
            ));
        }
        let id = book.next_id;
        book.next_id += 1;
        book.list.push(Reminder {
            id,
            text: text.clone(),
            set_at: now.timestamp(),
            due,
            fired: None,
            surfaced: surfaced(&settings, &now, due),
        });
        book.save();
        Ok(match due {
            Due::At(at) => format!("Reminder set for {}: {text}", words(at, &now)),
            Due::Later => format!(
                "Added to the captain's Later list: {text}. It comes back each day at {} until \
                 they deal with it.",
                clock_words(settings.later_time)
            ),
        })
    }

    /// The tool's answer to `app_remind`.
    pub fn answer(&self, arguments: &Value) -> Result<String, String> {
        let (text, when) = parse(arguments)?;
        self.set(&text, when)
    }

    /// Every reminder in plain text, for `app_reminders`.
    pub fn describe(&self) -> String {
        let now = self.now();
        let list = self.list();
        let mut out = format!("It is now {}.", words(now.timestamp(), &now));
        if list.is_empty() {
            out.push_str(" The captain has no reminders.");
        }
        for r in &list {
            let set = words(r.set_at, &now);
            out.push_str(&match (r.fired, r.due) {
                (Some(fired), _) => format!(
                    "\n- Due now, waiting for the captain since {}: {} (set {set})",
                    words(fired, &now),
                    r.text
                ),
                (None, Due::At(at)) => format!("\n- {}: {} (set {set})", words(at, &now), r.text),
                (None, Due::Later) => format!("\n- Later list: {} (set {set})", r.text),
            });
        }
        out
    }

    /// Marks every reminder whose time has come as due, and brings the Later list back once a
    /// day; returns those that came due now.
    pub fn fire_due(&self) -> Vec<Reminder> {
        let now = self.now();
        let settings = self.settings();
        let today = now.date();
        let back = settings.later_on(&now, today).ok();
        let mut book = self.book();
        let mut fired = Vec::new();
        for r in book.list.iter_mut().filter(|r| r.fired.is_none()) {
            let due = match r.due {
                Due::At(at) => at <= now.timestamp(),
                Due::Later => {
                    r.surfaced != Some(today) && back.is_some_and(|b| b <= now.timestamp())
                }
            };
            if due {
                r.fired = Some(now.timestamp());
                if r.due == Due::Later {
                    r.surfaced = Some(today);
                }
                fired.push(r.clone());
            }
        }
        if !fired.is_empty() {
            book.save();
        }
        fired
    }

    /// The next moment a reminder comes due, if any waits.
    pub fn next_due(&self) -> Option<Timestamp> {
        let now = self.now();
        let settings = self.settings();
        let today = now.date();
        let next_later = || {
            let back = settings.later_on(&now, today).ok()?;
            if back > now.timestamp() {
                Some(back)
            } else {
                settings.later_on(&now, today.tomorrow().ok()?).ok()
            }
        };
        let book = self.book();
        book.list
            .iter()
            .filter(|r| r.fired.is_none())
            .filter_map(|r| match r.due {
                Due::At(at) => Some(at),
                Due::Later if r.surfaced == Some(today) => {
                    settings.later_on(&now, today.tomorrow().ok()?).ok()
                }
                Due::Later => next_later(),
            })
            .min()
    }

    /// The captain dealt with it; it is gone.
    pub fn done(&self, id: u64) {
        self.delete(id)
    }

    pub fn delete(&self, id: u64) {
        let mut book = self.book();
        let before = book.list.len();
        book.list.retain(|r| r.id != id);
        if book.list.len() != before {
            book.save();
        }
    }

    /// Sets the reminder again for `when`, from now; returns the new time in words.
    pub fn snooze(&self, id: u64, when: When) -> Result<String, String> {
        let now = self.now();
        let settings = self.settings();
        let due = settings.resolve(when, &now)?;
        let mut book = self.book();
        let r = book
            .list
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or("That reminder is gone.")?;
        r.due = due;
        r.fired = None;
        r.surfaced = surfaced(&settings, &now, due);
        book.save();
        Ok(match due {
            Due::At(at) => words(at, &now),
            Due::Later => "the Later list".into(),
        })
    }
}

/// The day a Later reminder set at `now` counts as already back: today, once today's time has
/// passed, so it first comes back the next morning.
fn surfaced(settings: &Settings, now: &Zoned, due: Due) -> Option<Date> {
    let back = settings.later_on(now, now.date()).ok()?;
    (due == Due::Later && back <= now.timestamp()).then(|| now.date())
}

fn read_list(value: &Value) -> Result<Vec<Reminder>, String> {
    store::check_version(value)?;
    value["reminders"]
        .as_array()
        .ok_or("it has no list of reminders")?
        .iter()
        .enumerate()
        .map(|(i, r)| Reminder::from_json(r).ok_or(format!("reminder {} is not complete", i + 1)))
        .collect()
}

impl Book {
    /// Writes the file whole, readable by the captain only: a temporary file beside it, synced,
    /// then renamed over it. A failure is shown, and the reminders stay in memory.
    fn save(&mut self) {
        self.version += 1;
        let Some(path) = self.path.clone() else {
            return;
        };
        let reminders: Vec<Value> = self.list.iter().map(Reminder::to_json).collect();
        let bytes =
            serde_json::to_vec_pretty(&json!({"version": store::VERSION, "reminders": reminders}))
                .expect("JSON");
        match write_private(&path, &bytes, self.unreadable) {
            Ok(()) => {
                self.problem = None;
                self.unreadable = false;
            }
            Err(e) => {
                self.problem = Some(format!(
                    "Velvet Hammer could not save {}: {e}. Your reminders are shown but will \
                     not outlast a restart.",
                    path.display()
                ))
            }
        }
    }
}

fn write_private(path: &Path, bytes: &[u8], keep_old: bool) -> io::Result<()> {
    let dir = path.parent().expect("the file is in a directory");
    store::create_private_dir(dir)?;
    let temporary = store::suffixed(path, &format!(".tmp-{}", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options
        .open(&temporary)
        .and_then(|mut file| {
            store::make_private(&temporary)?;
            file.write_all(bytes)?;
            file.sync_all()
        })
        .and_then(|()| {
            if keep_old && path.exists() {
                fs::copy(path, store::suffixed(path, ".bad"))?;
            }
            fs::rename(&temporary, path)
        });
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written?;
    if let Ok(dir) = fs::File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

impl Saved for Settings {
    const FILE: &'static str = SETTINGS_FILE;
    const FALLBACK: &'static str = "the default reminder times are used";

    fn fallback() -> Self {
        Settings::default()
    }

    fn to_json(&self) -> Value {
        let mut out = json!({"version": store::VERSION});
        for setting in Setting::ALL {
            out[setting.key()] = json!(self.get(setting));
        }
        out
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        store::check_version(value)?;
        let mut settings = Settings::default();
        for setting in Setting::ALL {
            let ok = value[setting.key()]
                .as_i64()
                .is_some_and(|v| settings.set(setting, v));
            if !ok {
                return Err(format!("its {} is missing or out of range", setting.key()));
            }
        }
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;
    use jiff::tz::TimeZone;

    /// US Eastern time, with its daylight saving rules, without the system's time zone files.
    fn eastern() -> TimeZone {
        TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap()
    }

    fn at(y: i16, mo: i8, d: i8, h: i8, mi: i8) -> Zoned {
        date(y, mo, d).at(h, mi, 0, 0).to_zoned(eastern()).unwrap()
    }

    /// The local wall-clock time of `due`, as "2026-11-13 14:30".
    fn local_time(due: Due) -> String {
        let Due::At(t) = due else {
            return "later".into();
        };
        t.to_zoned(eastern()).strftime("%Y-%m-%d %H:%M").to_string()
    }

    fn resolve(when: When, now: &Zoned) -> String {
        local_time(Settings::default().resolve(when, now).unwrap())
    }

    // 2026-11-13 is a Friday.
    #[test]
    fn minutes_hours_and_days_are_offsets_from_now() {
        let now = at(2026, 11, 13, 14, 30);
        assert_eq!(
            resolve(When::In(20, Unit::Minutes), &now),
            "2026-11-13 14:50"
        );
        assert_eq!(resolve(When::In(2, Unit::Hours), &now), "2026-11-13 16:30");
        assert_eq!(resolve(When::In(3, Unit::Days), &now), "2026-11-16 14:30");
        assert_eq!(resolve(When::Later, &now), "later");
    }

    #[test]
    fn later_today_is_three_hours_but_never_past_the_evening() {
        // Three hours on is before 6:00 PM.
        assert_eq!(
            resolve(When::LaterToday, &at(2026, 11, 13, 10, 0)),
            "2026-11-13 13:00"
        );
        // Three hours on would pass 6:00 PM, so 6:00 PM.
        assert_eq!(
            resolve(When::LaterToday, &at(2026, 11, 13, 16, 0)),
            "2026-11-13 18:00"
        );
        // Exactly 6:00 PM or later: two hours on.
        assert_eq!(
            resolve(When::LaterToday, &at(2026, 11, 13, 18, 0)),
            "2026-11-13 20:00"
        );
        assert_eq!(
            resolve(When::LaterToday, &at(2026, 11, 13, 22, 30)),
            "2026-11-14 00:30"
        );
        // The settings change each number.
        let settings = Settings {
            later_today_hours: 1,
            evening: 20 * 60,
            after_evening_hours: 4,
            ..Settings::default()
        };
        let due = |h| {
            local_time(
                settings
                    .resolve(When::LaterToday, &at(2026, 11, 13, h, 0))
                    .unwrap(),
            )
        };
        assert_eq!(due(19), "2026-11-13 20:00");
        assert_eq!(due(16), "2026-11-13 17:00");
        assert_eq!(due(21), "2026-11-14 01:00");
    }

    #[test]
    fn tomorrow_morning_is_nine_tomorrow() {
        assert_eq!(
            resolve(When::TomorrowMorning, &at(2026, 11, 13, 14, 30)),
            "2026-11-14 09:00"
        );
        assert_eq!(
            resolve(When::TomorrowMorning, &at(2026, 11, 13, 0, 5)),
            "2026-11-14 09:00"
        );
        assert_eq!(
            resolve(When::TomorrowMorning, &at(2026, 12, 31, 23, 0)),
            "2027-01-01 09:00"
        );
    }

    #[test]
    fn next_week_is_the_next_monday_at_nine() {
        // Friday, Sunday, and Monday itself, which goes to the Monday after.
        assert_eq!(
            resolve(When::NextWeek, &at(2026, 11, 13, 14, 30)),
            "2026-11-16 09:00"
        );
        assert_eq!(
            resolve(When::NextWeek, &at(2026, 11, 15, 23, 0)),
            "2026-11-16 09:00"
        );
        assert_eq!(
            resolve(When::NextWeek, &at(2026, 11, 16, 8, 0)),
            "2026-11-23 09:00"
        );
        let settings = Settings {
            week_day: Weekday::Wednesday,
            week_time: 10 * 60 + 30,
            ..Settings::default()
        };
        let due = settings
            .resolve(When::NextWeek, &at(2026, 11, 13, 14, 30))
            .unwrap();
        assert_eq!(local_time(due), "2026-11-18 10:30");
    }

    #[test]
    fn daylight_saving_changes_keep_local_clock_times() {
        // Clocks go forward at 2:00 AM on Sunday, 2026-03-08, and back at 2:00 AM on Sunday,
        // 2026-11-01. Days keep the clock time; hours are exact.
        let saturday = at(2026, 3, 7, 21, 0);
        assert_eq!(
            resolve(When::In(1, Unit::Days), &saturday),
            "2026-03-08 21:00"
        );
        assert_eq!(
            resolve(When::In(6, Unit::Hours), &saturday),
            "2026-03-08 04:00"
        );
        assert_eq!(
            resolve(When::TomorrowMorning, &saturday),
            "2026-03-08 09:00"
        );
        let Due::At(due) = Settings::default()
            .resolve(When::TomorrowMorning, &saturday)
            .unwrap()
        else {
            panic!()
        };
        // Twelve hours of wall clock, eleven real hours.
        assert_eq!(
            due.duration_since(saturday.timestamp()),
            SignedDuration::from_hours(11)
        );
        let back = at(2026, 10, 31, 21, 0);
        assert_eq!(resolve(When::TomorrowMorning, &back), "2026-11-01 09:00");
        assert_eq!(resolve(When::In(1, Unit::Days), &back), "2026-11-01 21:00");
        // Later today at 12:30 AM as clocks go forward: three real hours on is 4:30 AM.
        let early = at(2026, 3, 8, 0, 30);
        assert_eq!(resolve(When::LaterToday, &early), "2026-03-08 04:30");
        // A time the change skips moves forward by the gap.
        let skipped = When::At("2026-03-08T02:30".parse().unwrap());
        assert_eq!(resolve(skipped, &saturday), "2026-03-08 03:30");
    }

    #[test]
    fn the_tool_reads_each_form_and_says_what_is_wrong() {
        let parsed = |args: Value| parse(&args);
        assert_eq!(
            parsed(
                json!({"text": "check the deploy", "when": "in", "amount": 20, "unit": "minutes"})
            ),
            Ok(("check the deploy".into(), When::In(20, Unit::Minutes)))
        );
        for (key, when) in [
            ("later_today", When::LaterToday),
            ("tomorrow_morning", When::TomorrowMorning),
            ("next_week", When::NextWeek),
            ("later", When::Later),
        ] {
            assert_eq!(parsed(json!({"text": "x", "when": key})).unwrap().1, when);
        }
        assert_eq!(
            parsed(json!({"text": "x", "when": "at", "at": "2026-11-20T15:00"}))
                .unwrap()
                .1,
            When::At(date(2026, 11, 20).at(15, 0, 0, 0))
        );
        for (args, problem) in [
            (json!({"text": "  ", "when": "later"}), "no text"),
            (
                json!({"text": "x".repeat(MAX_TEXT + 1), "when": "later"}),
                "201 characters",
            ),
            (json!({"text": "x", "when": "soon"}), "when must be one of"),
            (
                json!({"text": "x", "when": "in", "unit": "hours"}),
                "amount",
            ),
            (
                json!({"text": "x", "when": "in", "amount": 2, "unit": "weeks"}),
                "unit",
            ),
            (
                json!({"text": "x", "when": "at", "at": "tomorrow"}),
                "YYYY-MM-DDTHH:MM",
            ),
        ] {
            let got = parsed(args.clone()).unwrap_err();
            assert!(got.contains(problem), "{args}: {got}");
        }
        // The text is one line of plain text.
        assert_eq!(clean("call\n\tmum \u{202e}now").unwrap(), "call mum now");
    }

    fn clock_at(now: &Arc<Mutex<Zoned>>) -> Clock {
        let now = now.clone();
        Arc::new(move || now.lock().unwrap().clone())
    }

    fn scratch() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static RUN: AtomicU32 = AtomicU32::new(0);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/reminders")
            .join(format!(
                "{}-{}",
                std::process::id(),
                RUN.fetch_add(1, Ordering::Relaxed)
            ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn open(dir: &Path, now: &Arc<Mutex<Zoned>>) -> Reminders {
        let settings = Arc::new(RwLock::new(Settings::default()));
        Reminders::open(dir, settings, clock_at(now))
    }

    #[test]
    fn setting_answers_with_the_time_in_words_and_limits_what_it_keeps() {
        let now = Arc::new(Mutex::new(at(2026, 11, 13, 14, 30)));
        let reminders = Reminders::in_memory(clock_at(&now));
        let answer = |args: Value| reminders.answer(&args);
        assert_eq!(
            answer(
                json!({"text": "check the deploy", "when": "in", "amount": 20, "unit": "minutes"})
            ),
            Ok("Reminder set for Today, 2:50 PM: check the deploy".into())
        );
        assert_eq!(
            answer(json!({"text": "the standup", "when": "tomorrow_morning"})),
            Ok("Reminder set for Tomorrow, 9:00 AM: the standup".into())
        );
        assert_eq!(
            answer(json!({"text": "plan the week", "when": "next_week"})),
            Ok("Reminder set for Monday, Nov 16, 9:00 AM: plan the week".into())
        );
        assert_eq!(
            answer(json!({"text": "renew the domain", "when": "later"})),
            Ok(
                "Added to the captain's Later list: renew the domain. It comes back each day at \
                9:00 AM until they deal with it."
                    .into()
            )
        );
        assert_eq!(
            answer(json!({"text": "x", "when": "at", "at": "2026-11-13T09:00"})),
            Err("Today, 9:00 AM has already passed: it is now Today, 2:30 PM.".into())
        );
        assert!(answer(json!({"text": "x", "when": "in", "amount": 400, "unit": "days"})).is_err());
        assert!(answer(json!({"text": "x", "when": "in", "amount": 0, "unit": "days"})).is_err());
        let listed = reminders.describe();
        assert_eq!(
            listed,
            "It is now Today, 2:30 PM.\n\
             - Today, 2:50 PM: check the deploy (set Today, 2:30 PM)\n\
             - Tomorrow, 9:00 AM: the standup (set Today, 2:30 PM)\n\
             - Monday, Nov 16, 9:00 AM: plan the week (set Today, 2:30 PM)\n\
             - Later list: renew the domain (set Today, 2:30 PM)"
        );
        while reminders.list().len() < MAX_PENDING {
            reminders.set("more", When::Later).unwrap();
        }
        assert!(
            reminders
                .set("one too many", When::Later)
                .unwrap_err()
                .contains("already has 100")
        );
    }

    #[test]
    fn reminders_outlast_a_restart_and_come_due_late_as_overdue() {
        let dir = scratch();
        let now = Arc::new(Mutex::new(at(2026, 11, 13, 14, 30)));
        let reminders = open(&dir, &now);
        reminders
            .set("stretch", When::In(2, Unit::Minutes))
            .unwrap();
        reminders.set("renew the domain", When::Later).unwrap();
        assert_eq!(reminders.fire_due(), []);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join(FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let saved = reminders.list();
        drop(reminders);

        // The app was closed at the reminder's time and opens an hour later.
        *now.lock().unwrap() = at(2026, 11, 13, 15, 32);
        let reminders = open(&dir, &now);
        assert_eq!(reminders.list(), saved);
        let fired = reminders.fire_due();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].text, "stretch");
        assert!(fired[0].overdue());
        assert_eq!(reminders.fire_due(), [], "it fires once");
        // The new id follows the saved ones.
        reminders.set("third", When::Later).unwrap();
        assert_eq!(reminders.list().iter().map(|r| r.id).max(), Some(3));
        assert!(reminders.problem().is_none());
    }

    #[test]
    fn a_reminder_on_time_is_not_overdue_and_snooze_done_and_delete_work() {
        let now = Arc::new(Mutex::new(at(2026, 11, 13, 14, 30)));
        let reminders = Reminders::in_memory(clock_at(&now));
        reminders
            .set("stretch", When::In(2, Unit::Minutes))
            .unwrap();
        reminders
            .set("call back", When::In(1, Unit::Hours))
            .unwrap();
        assert_eq!(
            reminders.next_due().map(|t| local_time(Due::At(t))),
            Some("2026-11-13 14:32".into())
        );
        *now.lock().unwrap() = at(2026, 11, 13, 14, 32);
        let fired = reminders.fire_due();
        assert_eq!((fired.len(), fired[0].overdue()), (1, false));
        let id = fired[0].id;
        assert_eq!(
            reminders.snooze(id, When::In(10, Unit::Minutes)),
            Ok("Today, 2:42 PM".into())
        );
        assert_eq!(reminders.list()[0].fired, None);
        assert_eq!(
            reminders.snooze(id, When::TomorrowMorning),
            Ok("Tomorrow, 9:00 AM".into())
        );
        assert_eq!(
            reminders.next_due().map(|t| local_time(Due::At(t))),
            Some("2026-11-13 15:30".into())
        );
        reminders.done(id);
        reminders.delete(2);
        assert!(reminders.list().is_empty());
        assert_eq!(reminders.next_due(), None);
        assert!(reminders.snooze(id, When::Later).is_err());
    }

    #[test]
    fn the_later_list_comes_back_each_morning_until_dealt_with() {
        let dir = scratch();
        let now = Arc::new(Mutex::new(at(2026, 11, 13, 14, 30)));
        let reminders = open(&dir, &now);
        reminders.set("renew the domain", When::Later).unwrap();
        // Set after today's 9:00 AM: it first comes back tomorrow.
        assert_eq!(
            reminders.next_due().map(|t| local_time(Due::At(t))),
            Some("2026-11-14 09:00".into())
        );
        *now.lock().unwrap() = at(2026, 11, 14, 8, 59);
        assert_eq!(reminders.fire_due(), []);
        *now.lock().unwrap() = at(2026, 11, 14, 9, 0);
        let fired = reminders.fire_due();
        assert_eq!(fired.len(), 1);
        assert!(!fired[0].overdue());
        // Back on the Later list, it waits for the next morning.
        reminders.snooze(fired[0].id, When::Later).unwrap();
        *now.lock().unwrap() = at(2026, 11, 14, 17, 0);
        assert_eq!(reminders.fire_due(), []);
        drop(reminders);
        // The app was not running at 9:00 AM; it comes back at launch.
        *now.lock().unwrap() = at(2026, 11, 15, 11, 15);
        let reminders = open(&dir, &now);
        assert_eq!(reminders.fire_due().len(), 1);
        // Set before the day's time, it comes back that same morning.
        *now.lock().unwrap() = at(2026, 11, 16, 7, 0);
        reminders.set("water the plants", When::Later).unwrap();
        *now.lock().unwrap() = at(2026, 11, 16, 9, 0);
        assert_eq!(reminders.fire_due().len(), 1);
    }

    #[test]
    fn an_unreadable_file_is_kept_when_saved_over() {
        let dir = scratch();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(FILE), "{\"version\": 1, \"rem").unwrap();
        let now = Arc::new(Mutex::new(at(2026, 11, 13, 14, 30)));
        let reminders = open(&dir, &now);
        assert!(reminders.problem().unwrap().contains("not valid JSON"));
        assert!(reminders.list().is_empty());
        reminders.set("x", When::Later).unwrap();
        assert!(reminders.problem().is_none());
        assert_eq!(
            fs::read_to_string(dir.join(format!("{FILE}.bad"))).unwrap(),
            "{\"version\": 1, \"rem"
        );
    }

    #[test]
    fn settings_step_within_their_range_and_survive_the_file() {
        let mut settings = Settings::default();
        settings.step(Setting::WeekDay, false);
        assert_eq!(settings.week_day, Weekday::Sunday);
        settings.step(Setting::WeekDay, true);
        assert_eq!(settings.week_day, Weekday::Monday);
        for _ in 0..20 {
            settings.step(Setting::LaterTodayHours, true);
        }
        assert_eq!(settings.later_today_hours, 12);
        settings.step(Setting::Evening, true);
        assert_eq!(settings.shown(Setting::Evening), "6:30 PM");
        assert_eq!(settings.shown(Setting::Morning), "9:00 AM");
        assert_eq!(Settings::from_json(&settings.to_json()), Ok(settings));
        assert!(Settings::from_json(&json!({"version": 1})).is_err());
    }
}
