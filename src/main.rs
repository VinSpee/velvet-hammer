//! Velvet Hammer: one conversation with a chief of staff, the subagents it manages, and two
//! panels for what waits on the captain. `cargo run` opens the window;
//! `cargo run -- --snap chat.png` renders it headless (see the README for every scene).

mod agent;
mod approval;
mod bearings;
mod bridge;
mod claude;
mod find;
mod firstmate;
mod guard;
mod history;
mod links;
mod markdown;
mod menus;
mod model;
mod projects;
mod reminders;
mod store;
mod tools;
mod ui;
mod view;
mod visual;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rune::{AppSpec, Image, ImageCache, Key, KeyEvent, Player, TextEdit, Tx, Waker};

use agent::{ChiefOfStaff, Event, Stub};
use approval::{Activity, Mode, Store};
use bearings::Bearings;
use claude::Claude;
use firstmate::{Checkout, Distro, Firstmate, Work, lab::Lab};
use model::{Action, Author, Id, Message, Status, Workspace};
use projects::Project;
use reminders::{Reminder, Reminders, Unit, When};
use tools::{Handler, Picture};

/// All app state: the workspace the agent layer fills, plus what only the UI needs.
pub struct App {
    pub ws: Workspace,
    pub chief: Box<dyn ChiefOfStaff>,
    pub status: Status,
    pub firstmate: Firstmate,
    /// The captain's tool-approval settings, which the chief of staff's hook decides with.
    pub approvals: Store,
    /// The settings page's rule editor and example.
    pub form: approval::Form,
    /// Every tool use and what was decided.
    pub activity: Activity,
    /// Whether the chief of staff is steered towards views, and how firmly.
    pub visuals: store::Store<visual::Mode>,
    /// The reply typed on the visual-replies page to see what the check makes of it.
    pub visual_example: TextEdit,
    /// Which of firstmate's rules the hook enforces.
    pub rules: store::Store<guard::Settings>,
    /// Whether the conversation is kept across restarts.
    pub keeping: store::Store<history::Keeping>,
    /// The conversation as kept across restarts.
    pub history: history::History,
    /// The find bar over the conversation.
    pub find: find::Find,
    /// The captain's reminders, which the chief of staff's reminder tools set and list.
    pub reminders: Reminders,
    /// The times the reminder keywords stand for.
    pub reminder_settings: store::Store<reminders::Settings>,
    /// The reminders' change count when the window last showed them.
    reminders_seen: u64,
    /// The reminder whose snooze menu is open, and where: "banner", "panel" or "list".
    pub snoozing: Option<(&'static str, u64)>,
    /// The number typed into the open snooze menu.
    pub snooze_amount: TextEdit,
    /// Why the last snooze was not made.
    pub snooze_problem: Option<String>,
    /// Keeps `captain` current from firstmate's records, when the chief of staff is firstmate.
    watcher: Option<bearings::Watcher>,
    /// What firstmate's records say waits on the captain.
    pub captain: Bearings,
    /// Why firstmate's records could not be read the last time, in firstmate's words.
    pub captain_problem: Option<String>,
    /// The crewmate whose terminal is being read, while `showing` is [`Showing::Crew`].
    pub peek: Option<bearings::Peek>,
    /// Keeps `projects` current from firstmate's records, when the chief of staff is firstmate.
    project_watcher: Option<projects::Watcher>,
    /// The projects firstmate's records list, or why they could not be read. `None` until the
    /// first read, and always for a chief of staff that is not firstmate.
    pub projects: Option<Result<Vec<Project>, String>>,
    /// The captain started adding a project with the chief of staff, so the chat offers the
    /// small controls that help answer its questions.
    pub adding: bool,
    /// The folder dialog: [`projects::pick_folder`], or a stand-in in tests.
    pub picker: fn() -> Result<Option<PathBuf>, String>,
    /// The folder dialog that is open, while it is.
    picking: Option<Receiver<Result<Option<PathBuf>, String>>>,
    /// Why the folder dialog could not open.
    pub pick_problem: Option<String>,
    wake: Option<Waker>,
    /// The captain hold the next message answers, by its key.
    pub answering: Option<String>,
    pub draft: TextEdit,
    /// The chat message under the pointer, whose link button shows.
    pub hovered: Option<usize>,
    /// The element that has focus, as of the last frame.
    pub focused: Option<String>,
    /// The message whose link was just copied, by its id.
    pub copied: Option<String>,
    /// The message a link just went to, highlighted until the time given, or the id of one that
    /// is not in the chat.
    pub flash: Option<(Result<String, String>, f64)>,
    /// Seconds since the app started, as of the last frame.
    pub now: f64,
    /// An element to scroll to on the next frame, once the view shows it.
    pub scroll_to: Option<String>,
    /// Whose menu conventions the app follows.
    pub platform: menus::Platform,
    /// The window draws its own menu bar, as where Rune has no native one.
    pub in_window_menus: bool,
    /// The in-window menu that is open, by its title.
    pub open_menu: Option<&'static str>,
    /// Clear history was pressed and waits for the captain to confirm.
    pub confirm_clear: bool,
    pub showing: Showing,
    /// The chat message the chief of staff is still writing.
    pub replying: Option<usize>,
    /// How each finished tool use is drawn, by its id, with the image or video it shows.
    pub media: HashMap<String, Media>,
    /// The tool uses whose results the captain unfolded, by id.
    pub unfolded: HashSet<String>,
    /// The state of each view the chief of staff showed, by its tool use's id.
    pub ui: HashMap<String, ui::Live>,
    /// The images views show, by the path a view gave, or why one is not shown.
    pub ui_images: HashMap<String, Result<Image, String>>,
    /// Decodes the chat's images off the UI thread.
    images: ImageCache,
    /// The window's waker, once [`setup`] connects it.
    window_wake: Arc<OnceLock<Waker>>,
    /// Wakes the window when an image decodes or a video loads, once there is a window.
    media_wake: Waker,
}

/// What a finished tool use shows, chosen by [`tools::handler`] when its result arrives.
pub struct Media {
    pub handler: Handler,
    /// The decoding image of an image handler, or why there is none.
    pub image: Option<Result<Image, String>>,
    /// The player of a video handler.
    pub video: Option<Player>,
}

/// What the middle column shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Showing {
    /// The chief-of-staff chat, or the sign-in while it cannot run.
    Chat,
    /// A subagent's transcript.
    Peek(Id),
    /// A firstmate crewmate's terminal, by its task id.
    Crew(String),
    /// The managed firstmate clone: its state, and installing and updating it.
    Firstmate,
    /// The tool-approval settings.
    Approvals,
    /// The activity log of tool uses.
    Activity,
    /// The visual-replies setting.
    Visuals,
    /// Firstmate's rules the hook enforces, and what each did lately.
    Rules,
    /// Every setting, by section.
    Settings,
    /// The captain's reminders: due, coming up, and the Later list.
    Reminders,
    /// The times the reminder keywords stand for.
    ReminderSettings,
    /// One image from the chat, enlarged, by its tool use's id.
    Picture(String),
}

impl App {
    /// The app on `chief`, keeping its settings, activity log and reminders in `data`.
    pub fn new(chief: Box<dyn ChiefOfStaff>, firstmate: Firstmate, data: &Path) -> Self {
        App::with_clock(chief, firstmate, data, reminders::system_clock())
    }

    /// The app with reminders on `clock`, a fixed one in scenes and tests.
    pub fn with_clock(
        mut chief: Box<dyn ChiefOfStaff>,
        firstmate: Firstmate,
        data: &Path,
        clock: reminders::Clock,
    ) -> Self {
        let watcher = chief
            .workplace()
            .map(|clone| bearings::Watcher::start(clone, &firstmate.home_dir()));
        let project_watcher = chief
            .workplace()
            .map(|_| projects::Watcher::start(&firstmate.home_dir()));
        let approvals = Store::open(data);
        let visuals = store::Store::<visual::Mode>::open(data);
        let rules = store::Store::<guard::Settings>::open(data);
        let keeping = store::Store::<history::Keeping>::open(data);
        let mut history = history::History::new(data);
        let reminder_settings = store::Store::<reminders::Settings>::open(data);
        let reminders = Reminders::open(data, reminder_settings.shared(), clock);
        chief.approve_with(bridge::Policies {
            approval: approvals.shared(),
            visual: visuals.shared(),
            rules: rules.shared(),
            reminders: reminders.clone(),
        });
        let window_wake: Arc<OnceLock<Waker>> = Arc::new(OnceLock::new());
        let media_wake: Waker = Arc::new({
            let slot = window_wake.clone();
            move || {
                if let Some(wake) = slot.get() {
                    wake()
                }
            }
        });
        let mut ws = chief.open();
        if keeping.settings().0 {
            let earlier = history.load();
            ws.earlier = earlier.len();
            ws.chat.splice(0..0, earlier);
        }
        let mut app = App {
            ws,
            chief,
            status: Status::Checking,
            firstmate,
            approvals,
            form: approval::Form::default(),
            activity: Activity::open(data),
            visuals,
            visual_example: TextEdit::new(visual::EXAMPLE),
            rules,
            keeping,
            history,
            find: find::Find::default(),
            reminders,
            reminder_settings,
            reminders_seen: 0,
            snoozing: None,
            snooze_amount: TextEdit::default(),
            snooze_problem: None,
            watcher,
            captain: Bearings::default(),
            captain_problem: None,
            peek: None,
            project_watcher,
            projects: None,
            adding: false,
            picker: projects::pick_folder,
            picking: None,
            pick_problem: None,
            wake: None,
            answering: None,
            draft: TextEdit::default(),
            hovered: None,
            focused: None,
            copied: None,
            flash: None,
            now: 0.0,
            scroll_to: None,
            platform: menus::Platform::current(),
            in_window_menus: menus::Platform::current() != menus::Platform::Mac,
            open_menu: None,
            confirm_clear: false,
            showing: Showing::Chat,
            replying: None,
            media: HashMap::new(),
            unfolded: HashSet::new(),
            ui: HashMap::new(),
            ui_images: HashMap::new(),
            images: ImageCache::new(media_wake.clone()),
            window_wake,
            media_wake,
        };
        // A tool use kept from an earlier session shows as it did, but for its image bytes.
        for i in 0..app.ws.earlier {
            if let Some(tool) = app.ws.chat[i].tool.clone()
                && tool.result.is_some()
            {
                let media = app.media_for(&tools::handler(&tool));
                app.media.insert(tool.id, media);
            }
        }
        app.drain();
        app
    }

    /// Deletes the kept conversation and takes the earlier sessions off the chat. This
    /// session's messages stay until the app quits.
    pub fn clear_history(&mut self) {
        self.confirm_clear = false;
        self.history.clear();
        let earlier = std::mem::take(&mut self.ws.earlier);
        self.ws.chat.drain(..earlier);
        self.replying = self.replying.map(|i| i - earlier);
        self.find.current = 0;
        self.hovered = None;
    }

    /// Writes the finished chat message `i` to the history, if the captain keeps it.
    fn keep(&mut self, i: usize) {
        if self.keeping.settings().0
            && let Some(message) = self.ws.chat.get(i)
        {
            self.history.record(message);
        }
    }

    /// Applies what the agent layer reported; true when anything changed.
    pub fn drain(&mut self) -> bool {
        let events = self.chief.events();
        let mut changed = self.firstmate.poll() | !events.is_empty();
        changed |= self.follow_peek();
        if let Some(projects) = self.project_watcher.as_mut().and_then(|w| w.poll()) {
            self.projects = Some(projects);
            changed = true;
        }
        if let Some(picked) = self.picking.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.picking = None;
            match picked {
                // The captain reads the path and sends it, like any other answer.
                Ok(Some(folder)) => self.draft.text = folder.display().to_string(),
                Ok(None) => {}
                Err(problem) => self.pick_problem = Some(problem),
            }
            changed = true;
        }
        if let Some(snapshot) = self.watcher.as_mut().and_then(|w| w.poll()) {
            match snapshot {
                Ok(captain) => {
                    self.captain = captain;
                    self.captain_problem = None;
                }
                Err(problem) => self.captain_problem = Some(problem),
            }
            // An answered hold leaves firstmate's records; stop answering one that is gone.
            if let Some(key) = &self.answering
                && !self.captain.holds.iter().any(|h| &h.key == key)
            {
                self.answering = None;
            }
            changed = true;
        }
        for event in events {
            self.take(event);
        }
        changed | self.remind()
    }

    /// Shows the reminders that came due, posting each as a system notification where the app
    /// may; true when the reminders changed since the window last showed them.
    fn remind(&mut self) -> bool {
        for reminder in self.reminders.fire_due() {
            notify(&reminder);
        }
        let version = self.reminders.version();
        let changed = version != self.reminders_seen;
        self.reminders_seen = version;
        changed
    }

    /// The captain dealt with the reminder.
    pub fn reminder_done(&mut self, id: u64) {
        self.reminders.done(id);
        self.dealt_with(id);
    }

    pub fn delete_reminder(&mut self, id: u64) {
        self.reminders.delete(id);
        self.dealt_with(id);
    }

    /// Sets the reminder again for `when`, from now.
    pub fn snooze(&mut self, id: u64, when: When) {
        match self.reminders.snooze(id, when) {
            Ok(_) => self.dealt_with(id),
            Err(problem) => self.snooze_problem = Some(problem),
        }
    }

    /// Snoozes for the number typed in the snooze menu, in `unit`.
    pub fn snooze_for(&mut self, id: u64, unit: reminders::Unit) {
        match self.snooze_amount.text.trim().parse() {
            Ok(amount) => self.snooze(id, When::In(amount, unit)),
            Err(_) => self.snooze_problem = Some("Type a whole number first.".into()),
        }
    }

    /// Closes the snooze menu and takes the reminder's system notification back.
    fn dealt_with(&mut self, id: u64) {
        self.snoozing = None;
        self.snooze_amount.clear();
        self.snooze_problem = None;
        rune_notify::remove(&[&notification_id(id)]);
        self.remind();
    }

    /// Applies one thing the agent layer reported.
    pub fn take(&mut self, event: Event) {
        match event {
            Event::Status(status) => self.status = status,
            Event::Reply { text, done } => {
                match self.replying {
                    Some(i) => self.ws.chat[i].text = text,
                    None => {
                        self.ws.chat.push(Message::new(Author::ChiefOfStaff, text));
                        self.replying = Some(self.ws.chat.len() - 1);
                    }
                }
                if done && let Some(i) = self.replying.take() {
                    self.keep(i);
                }
            }
            Event::Problem(text) => {
                // A reply cut short is kept as far as it got.
                if let Some(i) = self.replying.take() {
                    self.keep(i);
                }
                self.ws.chat.push(Message::new(Author::Problem, text));
                self.keep(self.ws.chat.len() - 1);
            }
            Event::Item(item) => self.ws.items.push(item),
            Event::Withdrawn(id) => self.ws.items.retain(|i| i.id != id),
            Event::Activity(entry) => self.activity.record(entry),
            Event::Notice(text) => {
                self.ws.chat.push(Message::new(Author::App, text));
                self.keep(self.ws.chat.len() - 1);
            }
            // A view tool use arrives first as it starts streaming, then whole.
            Event::Tool(tool) => {
                let id = tool.id.clone();
                match self.tool_mut(&id) {
                    Some(shown) => shown.input = tool.input,
                    None => self.ws.chat.push(Message::tool(tool)),
                }
                self.load_view_images(&id);
            }
            Event::ToolInput { id, input } => {
                if let Some(tool) = self.tool_mut(&id) {
                    tool.input = input;
                    self.load_view_images(&id);
                }
            }
            Event::ToolResult { id, result } => {
                let Some(tool) = self.tool_mut(&id) else {
                    return;
                };
                tool.result = Some(result);
                let handler = tools::handler(tool);
                let media = self.media_for(&handler);
                self.media.insert(id.clone(), media);
                if let Some(i) = self
                    .ws
                    .chat
                    .iter()
                    .position(|m| m.tool.as_ref().is_some_and(|t| t.id == id))
                {
                    self.keep(i);
                }
            }
        }
    }

    /// The tool use in the chat with this id.
    pub fn tool(&self, id: &str) -> Option<&tools::ToolUse> {
        self.ws
            .chat
            .iter()
            .find_map(|m| m.tool.as_ref().filter(|t| t.id == id))
    }

    fn tool_mut(&mut self, id: &str) -> Option<&mut tools::ToolUse> {
        self.ws
            .chat
            .iter_mut()
            .find_map(|m| m.tool.as_mut().filter(|t| t.id == id))
    }

    /// The spec a view tool use carries, as far as it has arrived, or why it is not shown.
    pub fn view_spec(&self, id: &str) -> Result<ui::Spec, String> {
        let tool = self.tool(id).ok_or("The view is no longer in the chat.")?;
        ui::parse(&tool.input).map(|(spec, _)| spec)
    }

    /// Starts decoding each image a view shows that a tool use in this session already named.
    /// Any other path, and every URL, is refused without being read.
    fn load_view_images(&mut self, id: &str) {
        let Some(tool) = self.tool(id).filter(|t| tools::handler(t) == Handler::Ui) else {
            return;
        };
        let Ok((spec, _)) = ui::parse(&tool.input) else {
            return;
        };
        let state = self
            .ui
            .get(id)
            .map_or(spec.state.clone(), |l| l.state(&spec));
        let mut wanted = Vec::new();
        collect_images(ui::tree(&spec, &state).root.as_ref(), &mut wanted);
        for src in wanted {
            // A refusal is decided again, since a later tool use may name the file.
            if matches!(self.ui_images.get(&src), Some(Ok(_))) {
                continue;
            }
            let named = self
                .ws
                .chat
                .iter()
                .filter_map(|m| m.tool.as_ref())
                .filter(|t| tools::handler(t) != Handler::Ui)
                .flat_map(tools::named_files)
                .any(|p| p.as_os_str() == src.as_str());
            let image = if !named {
                Err("Only images a tool in this session already named are shown.".to_string())
            } else {
                match tools::handler_for_file(Path::new(&src)) {
                    Some(handler @ Handler::Image(_)) => self
                        .media_for(&handler)
                        .image
                        .unwrap_or_else(|| Err("There is no image to show.".into())),
                    _ => Err("It is not an image file that exists.".to_string()),
                }
            };
            self.ui_images.insert(src, image);
        }
    }

    /// Runs the actions a control in a view fires, then sends each `submit` to the chief of
    /// staff with the values of the view's inputs. `did` says what the captain did.
    pub fn fire(&mut self, id: &str, trigger: &ui::Trigger, did: String) {
        let Ok(spec) = self.view_spec(id) else {
            return;
        };
        let live = self.ui.entry(id.to_string()).or_default();
        let sent = live.fire(&spec, trigger);
        let state = live.state(&spec);
        let inputs = ui::values(&state, &ui::tree(&spec, &state).bound);
        self.load_view_images(id);
        for params in sent {
            if !self.chat_ready() {
                return;
            }
            self.act(Action::Ui(ui::UiAction {
                tool: id.to_string(),
                did: did.clone(),
                params,
                inputs: inputs.clone(),
            }));
        }
    }

    /// Checks or unchecks a view's checkbox, writing its bound path, then fires its actions.
    pub fn tick(
        &mut self,
        id: &str,
        path: Option<&str>,
        on: bool,
        change: Option<&ui::Trigger>,
        did: String,
    ) {
        let Ok(spec) = self.view_spec(id) else {
            return;
        };
        if let Some(path) = path {
            let live = self.ui.entry(id.to_string()).or_default();
            live.write(&spec, path, serde_json::Value::Bool(on));
        }
        if let Some(change) = change {
            self.fire(id, change, did);
        }
    }

    /// The text field a view binds to `path`.
    pub fn view_edit(&mut self, id: &str, path: &str, shown: &str) -> &mut TextEdit {
        let spec = self.view_spec(id).unwrap_or_default();
        self.ui
            .entry(id.to_string())
            .or_default()
            .edit(&spec, path, shown)
    }

    /// Starts decoding the image, or makes the player, that `handler` shows. Only files a tool
    /// use named reach here, and an image file larger than [`tools::MAX_MEDIA_BYTES`] is not
    /// read.
    fn media_for(&self, handler: &Handler) -> Media {
        let image = match handler {
            Handler::Image(Picture::Block { data, .. }) => {
                Some(data.clone().map(|bytes| self.images.load(bytes)))
            }
            Handler::Image(Picture::File(path)) => Some(match std::fs::metadata(path) {
                Ok(m) if m.len() > tools::MAX_MEDIA_BYTES => Err(format!(
                    "The image is larger than {} MB, so it is not shown.",
                    tools::MAX_MEDIA_BYTES / (1024 * 1024)
                )),
                Ok(_) => Ok(self.images.file(path, 0.0, 1.0)),
                Err(e) => Err(format!("The image could not be read: {e}.")),
            }),
            _ => None,
        };
        let video = match handler {
            Handler::Video(path) => {
                let player = Player::file(path);
                player.set_waker(self.media_wake.clone());
                Some(player)
            }
            _ => None,
        };
        Media {
            handler: handler.clone(),
            image,
            video,
        }
    }

    /// Opens a video file a tool use named in the system's default player. Nothing opens on
    /// its own; only the captain's press calls this.
    pub fn open_file(&self, path: &Path) {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let _ = std::process::Command::new(opener).arg(path).spawn();
    }

    pub fn act(&mut self, action: Action) {
        self.ws.apply(&action);
        if matches!(action, Action::Say(_) | Action::Ui(_)) {
            self.keep(self.ws.chat.len() - 1);
            // A reminder the chief of staff sets in this turn links back to this message.
            self.reminders
                .said_in(&self.ws.chat[self.ws.chat.len() - 1].id);
        }
        self.chief.act(&self.ws, &action);
        self.drain();
    }

    /// Starts, switches or stops the read of a crewmate's terminal to match what the middle
    /// column shows, and takes in what it read; true when the pane changed.
    fn follow_peek(&mut self) -> bool {
        let wanted = match (&self.showing, self.chief.workplace()) {
            (Showing::Crew(id), Some(clone)) => Some((id, clone)),
            _ => None,
        };
        match wanted {
            None => self.peek = None,
            Some((id, _)) if self.peek.as_ref().is_some_and(|p| &p.id == id) => {}
            Some((id, clone)) => {
                let home = self.firstmate.home_dir();
                self.peek = Some(bearings::Peek::start(id, clone, &home, self.wake.clone()));
            }
        }
        self.peek.as_mut().is_some_and(|p| p.poll())
    }

    /// Whether the chat can run: Claude Code is signed in, and a chief of staff that runs in
    /// the managed firstmate clone has that clone installed.
    pub fn chat_ready(&self) -> bool {
        matches!(self.status, Status::Ready(_))
            && (self.chief.workplace().is_none() || self.firstmate.installed())
    }

    pub fn send(&mut self) {
        let text = self.draft.text.trim().to_string();
        if text.is_empty() || !self.chat_ready() {
            return;
        }
        self.draft.clear();
        // An answer goes to the chief of staff in the captain's own words, naming the hold.
        // Firstmate records it and decides whether it closes a question or frees held work.
        let text = match self.answering.take() {
            Some(key) => format!("My answer to the captain hold {key}: {text}"),
            None => text,
        };
        self.act(Action::Say(text));
    }

    /// Whether firstmate's records list no project, including in a home that does not exist yet.
    pub fn has_no_projects(&self) -> bool {
        matches!(&self.projects, Some(Ok(list)) if list.is_empty())
    }

    /// Starts adding a project: asks the chief of staff, in the chat, to walk the captain
    /// through firstmate's own procedure. Firstmate clones and registers the project; the app
    /// only sees the result in firstmate's records.
    pub fn add_project(&mut self) {
        if !self.chat_ready() {
            return;
        }
        self.adding = true;
        self.answering = None;
        self.showing = Showing::Chat;
        self.act(Action::Say(ADD_PROJECT.into()));
    }

    /// Sends one of the add flow's quick replies, such as the captain's yes.
    pub fn reply(&mut self, words: &str) {
        if self.chat_ready() {
            self.act(Action::Say(words.into()));
        }
    }

    pub fn finish_adding(&mut self) {
        self.adding = false;
        self.pick_problem = None;
    }

    /// Opens the folder dialog on a background thread; the chosen path lands in the draft.
    pub fn pick_folder(&mut self) {
        if self.picking.is_some() {
            return;
        }
        self.pick_problem = None;
        let (tx, rx) = channel();
        let (picker, wake) = (self.picker, self.wake.clone());
        std::thread::spawn(move || {
            let _ = tx.send(picker());
            if let Some(wake) = wake {
                wake();
            }
        });
        self.picking = Some(rx);
    }

    pub fn picking(&self) -> bool {
        self.picking.is_some()
    }

    /// Tells the chief of staff, in the chat, that the captain approves merging `url`.
    pub fn approve_merge(&mut self, task: &str, url: &str) {
        if self.chat_ready() {
            self.act(Action::Say(format!(
                "I approve merging {url} (task {task})."
            )));
        }
    }

    pub fn sign_in(&mut self) {
        self.chief.sign_in();
        self.drain();
    }

    pub fn cancel_sign_in(&mut self) {
        self.chief.cancel_sign_in();
        self.drain();
    }

    pub fn recheck(&mut self) {
        self.chief.recheck();
        self.drain();
    }

    /// Opens the sign-in page Claude Code printed in the system browser. Claude Code opens it
    /// too; this is for when that did not work or the tab was closed.
    pub fn open_sign_in_page(&self) {
        if let Status::SigningIn { url: Some(url) } = &self.status {
            let opener = if cfg!(target_os = "macos") {
                "open"
            } else {
                "xdg-open"
            };
            let _ = std::process::Command::new(opener).arg(url).spawn();
        }
    }

    /// Drains events until `done` holds, for headless scenes that drive a real process.
    fn settle(&mut self, done: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(self) {
            assert!(Instant::now() < deadline, "the agent layer never settled");
            std::thread::sleep(Duration::from_millis(20));
            self.drain();
        }
    }
}

/// The image paths a drawn view shows.
fn collect_images(node: Option<&ui::Node>, out: &mut Vec<String>) {
    let Some(node) = node else { return };
    if let ui::What::Image { src, .. } = &node.what
        && !src.is_empty()
    {
        out.push(src.clone());
    }
    for child in &node.children {
        collect_images(Some(child), out);
    }
}

/// What the captain says to start adding a project. Firstmate's `project-management` skill owns
/// the procedure; this asks for it one question at a time and repeats the consent rules the
/// captain wants kept.
pub const ADD_PROJECT: &str = "I want to add a project. Follow your project-management \
    procedure and walk me through it one question at a time, waiting for my answer before the \
    next: first the repository (a git URL, or a folder on this computer), then the project's \
    name, then each delivery choice the procedure needs, with the default you recommend. Before \
    you clone, register or initialize anything, tell me exactly what you will clone or register \
    and where, and wait for my yes. Do not create, push to or publish anything remote unless I \
    say yes to that here. If something goes wrong or you refuse, tell me why in plain words and \
    leave everything as it is. When the project is added, tell me the result in plain words and \
    ask whether I want to add another.";

fn init() -> App {
    let mut firstmate = Firstmate::open(&firstmate::data_dir(), firstmate::UPSTREAM);
    firstmate.start();
    let claude = Claude::new("claude").in_firstmate(firstmate.clone_dir(), firstmate.home_dir());
    App::new(Box::new(claude), firstmate, &firstmate::data_dir())
}

fn stub() -> App {
    App::new(
        Box::new(Stub::default()),
        installed_firstmate(),
        &scratch("unused"),
    )
}

/// A firstmate clone that is installed and up to date, for scenes and tests about the rest of
/// the app. It runs no git.
fn installed_firstmate() -> Firstmate {
    Firstmate::showing(
        "/Users/captain/.velvet-hammer/firstmate".into(),
        firstmate::UPSTREAM,
        Distro::Installed(Checkout {
            head: "bb3b2d6".into(),
            upstream: Some("origin/main".into()),
            ..Checkout::default()
        }),
        None,
    )
}

/// The stub app showing the firstmate page for `firstmate`.
fn on_firstmate(firstmate: Firstmate) -> App {
    let mut app = App::new(Box::new(Stub::default()), firstmate, &scratch("unused"));
    app.showing = Showing::Firstmate;
    app
}

/// The firstmate page for a managed clone of a scratch upstream (see [`Lab`]): installed, then
/// changed on either side by `then`, then checked against upstream.
fn firstmate_lab(then: impl FnOnce(&Lab)) -> (App, Lab) {
    let lab = Lab::new();
    let mut app = on_firstmate(lab.firstmate());
    app.firstmate.install();
    app.settle(idle);
    then(&lab);
    app.firstmate.check();
    app.settle(idle);
    (app, lab)
}

fn idle(s: &App) -> bool {
    s.firstmate.work().is_none()
}

/// The fake `claude` in `scripts/`, which speaks the real one's protocol without an account.
const FAKE_CLAUDE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/fake-claude");

/// An app on the fake `claude`, starting in `state` (see `scripts/fake-claude`).
fn fake(state: &str, extra_env: &[(&str, &str)]) -> App {
    on_fake(FAKE_CLAUDE, &scratch(state), extra_env)
}

/// The fake `claude`, signed in and settled, with the tool-approval mode set to `mode`.
fn fake_in(mode: Mode) -> App {
    let mut app = fake("signed-in", &[]);
    app.approvals.change(|s| s.mode = mode);
    app.settle(settled);
    app
}

fn fake_program(program: &str, state: &str, extra_env: &[(&str, &str)]) -> App {
    on_fake(program, &scratch(state), extra_env)
}

/// A fresh scratch directory for the fake `claude`, so runs never share sign-in state. They live
/// under `target/`, so `cargo clean` removes them.
fn scratch(state: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static RUN: AtomicU32 = AtomicU32::new(0);
    let home = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/fake-claude")
        .join(format!(
            "{}-{}",
            std::process::id(),
            RUN.fetch_add(1, Ordering::Relaxed)
        ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("scratch directory");
    std::fs::write(home.join("state"), state).expect("fake claude state");
    home
}

fn on_fake(program: &str, home: &std::path::Path, extra_env: &[(&str, &str)]) -> App {
    let mut claude = Claude::new(program).env("FAKE_CLAUDE_HOME", home.to_string_lossy());
    for (k, v) in extra_env {
        claude = claude.env(k, *v);
    }
    App::new(Box::new(claude), installed_firstmate(), home)
}

/// An app on the fake `claude`, signed in and running as firstmate in `lab`'s managed clone,
/// which is installed first when `install` is set.
fn fake_in_firstmate(lab: &Lab, install: bool) -> App {
    let mut firstmate = lab.firstmate();
    let claude = Claude::new(FAKE_CLAUDE)
        .env("FAKE_CLAUDE_HOME", scratch("signed-in").to_string_lossy())
        .in_firstmate(firstmate.clone_dir(), firstmate.home_dir());
    if install {
        firstmate.install();
    }
    let mut app = App::new(Box::new(claude), firstmate, &lab.data);
    app.settle(|s| settled(s) && idle(s));
    app
}

fn settled(s: &App) -> bool {
    s.status != Status::Checking
}

fn replied(s: &App) -> bool {
    s.replying.is_none()
        && s.ws
            .chat
            .last()
            .is_some_and(|m| m.author != Author::Captain)
}

/// The longest the app waits before looking at the reminders again: a timer may not count the
/// time the computer slept.
const RECHECK: Duration = Duration::from_secs(30);

fn poll(s: &mut App, now: f64, tx: &mut Tx) -> bool {
    s.now = now;
    let mut changed = s.drain() | s.follow_find(tx);
    if let Some(key) = s.scroll_to.take() {
        tx.scroll_center(key);
    }
    if s.flash.as_ref().is_some_and(|(_, until)| *until <= now) {
        s.flash = None;
        changed = true;
    }
    let focused = tx.focused().map(str::to_string);
    if focused != s.focused {
        s.focused = focused;
        changed = true;
    }
    if let Some(due) = s.reminders.next_due() {
        let left = due.duration_since(s.reminders.now().timestamp());
        tx.wake_after(Duration::try_from(left).unwrap_or_default().min(RECHECK));
    }
    changed
}

/// How long a message a link went to stays highlighted, in seconds.
const FLASH: f64 = 2.5;

impl App {
    /// Goes to the message a message link names: shows the chat, scrolls to the message and
    /// highlights it briefly, or says the message is not in the chat. Any other link opens in the
    /// browser. `url` may come from outside the app (a link the system opened), so it is parsed
    /// and nothing else is done with it.
    pub fn open_link(&mut self, url: &str, tx: &mut Tx) {
        match links::parse(url.trim()) {
            Some(id) => {
                let id = id.to_string();
                self.go_to(&id);
                tx.wake_after(Duration::from_secs_f64(FLASH));
            }
            None => {
                let _ = tx.open_url(url);
            }
        }
    }

    pub fn go_to(&mut self, id: &str) {
        self.showing = Showing::Chat;
        let target = match self.ws.message(id) {
            Some((i, m)) => {
                self.scroll_to = Some(view::message_key(i, m));
                Ok(id.to_string())
            }
            None => Err(id.to_string()),
        };
        self.flash = Some((target, self.now + FLASH));
    }
}

/// The app's keyboard shortcuts, heard when the focused control does not take the key. Cmd is
/// Ctrl off macOS.
fn on_key(s: &mut App, key: &KeyEvent, tx: &mut Tx) -> bool {
    if key.is_command('f') {
        s.command(menus::Command::Find, tx);
    } else if key.is_command('g') && key.mods.shift {
        s.command(menus::Command::FindPrevious, tx);
    } else if key.is_command('g') {
        s.command(menus::Command::FindNext, tx);
    } else if key.is_command(',') {
        s.command(menus::Command::Settings, tx);
    } else if key.key == Key::Escape && s.open_menu.is_some() {
        s.open_menu = None;
    } else if key.key == Key::Escape && s.find.open {
        s.close_find(tx);
    } else {
        return false;
    }
    true
}

fn notification_id(id: u64) -> String {
    format!("reminder-{id}")
}

/// Posts a reminder that came due to the system's notifications, unless the window is in
/// front, where the banner shows it. Only a bundled app may post on macOS, which asks the
/// captain's permission the first time; otherwise, or if they refused, the window alone shows
/// it.
fn notify(reminder: &Reminder) {
    if rune_notify::available() && !rune_notify::app_focused() {
        rune_notify::post(
            &rune_notify::Notification::new(notification_id(reminder.id), "Reminder")
                .body(reminder.text.clone()),
        );
    }
}

fn setup(s: &mut App, wake: Waker) {
    s.firstmate.connect(wake.clone());
    if let Some(watcher) = &mut s.watcher {
        watcher.connect(wake.clone());
    }
    if let Some(watcher) = &mut s.project_watcher {
        watcher.connect(wake.clone());
    }
    s.wake = Some(wake.clone());
    let _ = s.window_wake.set(wake.clone());
    s.chief.connect(wake);
}

fn main() {
    let mut app = rune::App::new(AppSpec::<App> {
        scenes: &[
            ("chat", |s| *s = stub()),
            ("replied", |s| {
                *s = stub();
                s.draft.text = "Ship it when CI is green.".into();
                s.send();
            }),
            ("empty", |s| {
                *s = stub();
                s.ws.items.clear()
            }),
            ("peek", |s| {
                *s = stub();
                s.showing = Showing::Peek(s.ws.agents[0].id)
            }),
            // The rest drive the fake `claude` for real.
            ("signed-out", |s| {
                *s = fake("signed-out", &[]);
                s.settle(settled);
            }),
            ("signing-in", |s| {
                *s = fake("signed-out", &[]);
                s.settle(settled);
                s.sign_in();
                s.settle(|s| matches!(s.status, Status::SigningIn { url: Some(_) }));
            }),
            ("signed-in", |s| {
                *s = fake("signed-in", &[]);
                s.settle(settled);
                s.draft.text = "What needs me today?".into();
                s.send();
                s.settle(replied);
            }),
            ("expired", |s| {
                *s = fake("expired", &[]);
                s.settle(settled);
                s.draft.text = "What needs me today?".into();
                s.send();
                s.settle(|s| s.status == Status::Expired);
            }),
            ("cli-missing", |s| {
                *s = fake_program("/nonexistent/claude", "signed-out", &[]);
                s.settle(settled);
            }),
            // The fake calls back into the app through the bridge: the app_note tool, and the
            // PreToolUse hook waiting on the captain, then allowed.
            ("noted", |s| {
                *s = fake("signed-in", &[]);
                s.settle(settled);
                s.draft.text = "note: Renew the velvet-hammer.dev domain".into();
                s.send();
                s.settle(replied);
            }),
            ("approval", |s| {
                *s = fake_in(Mode::Ask);
                s.settle(settled);
                s.draft.text = "run: git push".into();
                s.send();
                s.settle(|s| s.ws.needs_input().next().is_some());
            }),
            ("approved", |s| {
                *s = fake_in(Mode::Ask);
                s.settle(settled);
                s.draft.text = "run: git push".into();
                s.send();
                s.settle(|s| s.ws.needs_input().next().is_some());
                let item = s.ws.needs_input().next().map(|i| i.id).unwrap();
                s.act(Action::Answer {
                    item,
                    choice: "Allow".into(),
                });
                s.settle(replied);
            }),
            ("approval-long", |s| {
                *s = fake_in(Mode::Ask);
                s.settle(settled);
                s.draft.text = format!("run: {}", long_command());
                s.send();
                s.settle(|s| s.ws.needs_input().next().is_some());
            }),
            // Tool approval: the settings page, an ask rule in Run without asking, the activity
            // log the fake's tool uses filled, and a settings file the app could not read.
            ("approval-settings", |s| {
                *s = stub();
                s.approvals.change(|x| x.rules = sample_rules());
                s.showing = Showing::Approvals;
            }),
            ("approval-by-rule", |s| {
                *s = fake_in(Mode::Run);
                s.approvals.change(|x| x.rules = sample_rules());
                s.draft.text = "run: git push origin main".into();
                s.send();
                s.settle(|s| s.ws.needs_input().next().is_some());
            }),
            ("approval-activity", |s| {
                *s = fake_in(Mode::Run);
                s.approvals.change(|x| x.rules = sample_rules());
                say(s, "run: cargo test");
                say(s, "run: git push --force");
                s.draft.text = "run: git push origin main".into();
                s.send();
                s.settle(|s| s.ws.needs_input().next().is_some());
                let item = s.ws.needs_input().next().map(|i| i.id).unwrap();
                s.act(Action::Answer {
                    item,
                    choice: "Allow".into(),
                });
                s.settle(replied);
                s.showing = Showing::Activity;
            }),
            ("approval-corrupt", |s| {
                let dir = scratch("unused");
                std::fs::write(
                    dir.join(approval::SETTINGS_FILE),
                    "{\"version\": 1, \"mode\"",
                )
                .unwrap();
                *s = App::new(Box::new(Stub::default()), installed_firstmate(), &dir);
                s.showing = Showing::Approvals;
            }),
            ("claude-error", |s| {
                *s = fake("signed-in", &[]);
                s.settle(settled);
                s.draft.text = "Please fail this one.".into();
                s.send();
                s.settle(replied);
            }),
            // The chat running as firstmate: the fake `claude` in a managed clone of a scratch
            // upstream, with its own operational home.
            ("firstmate-session", |s| {
                *s = fake_in_firstmate(&Lab::new(), true);
                s.draft.text = "whereami".into();
                s.send();
                s.settle(replied);
            }),
            // The panels fed from firstmate's records: two captain holds and a merge approval.
            ("firstmate-panels", |s| {
                let lab = Lab::new();
                lab.bearings(bearings::SNAPSHOT);
                *s = fake_in_firstmate(&lab, true);
                s.settle(|s| !s.captain.is_empty());
            }),
            ("firstmate-crew", |s| {
                let lab = Lab::new();
                lab.bearings(bearings::SNAPSHOT);
                lab.pane(
                    "lab-verify",
                    "$ scripts/verify.sh\nrendering chat.png\nrendering peek.png",
                );
                *s = fake_in_firstmate(&lab, true);
                s.settle(|s| !s.captain.crew.is_empty());
                s.showing = Showing::Crew("lab-verify".into());
                s.settle(|s| s.peek.as_ref().is_some_and(|p| p.pane.is_some()));
            }),
            ("firstmate-answering", |s| {
                let lab = Lab::new();
                lab.bearings(bearings::SNAPSHOT);
                *s = fake_in_firstmate(&lab, true);
                s.settle(|s| !s.captain.is_empty());
                s.answering = Some("lab-window-size".into());
            }),
            // Projects from firstmate's records: none yet in a fresh install, none in an
            // installed home, adding the first one with the chief of staff, and three listed.
            ("projects-fresh", |s| {
                *s = fake_in_firstmate(&Lab::new(), false);
                s.settle(|s| s.projects.is_some());
            }),
            ("projects-empty", |s| {
                *s = fake_in_firstmate(&Lab::new(), true);
                s.settle(|s| s.projects.is_some());
            }),
            ("projects-adding", |s| {
                *s = fake_in_firstmate(&Lab::new(), true);
                s.settle(|s| s.projects.is_some());
                s.add_project();
                s.settle(replied);
            }),
            ("projects-listed", |s| {
                let lab = Lab::new();
                lab.registry(projects::SAMPLE_REGISTRY);
                *s = fake_in_firstmate(&lab, true);
                s.settle(|s| {
                    s.projects.as_ref().is_some_and(|p| p.is_ok()) && !s.has_no_projects()
                });
            }),
            // The managed firstmate clone. All but the first two run git on a scratch upstream.
            ("firstmate-missing", |s| {
                *s = on_firstmate(Lab::new().firstmate());
            }),
            ("firstmate-installing", |s| {
                let lab = Lab::new();
                *s = on_firstmate(Firstmate::showing(
                    lab.clone_dir(),
                    &lab.url(),
                    Distro::Missing,
                    Some(Work::Installing),
                ));
            }),
            ("firstmate-up-to-date", |s| *s = firstmate_lab(|_| {}).0),
            ("firstmate-update-available", |s| {
                *s = firstmate_lab(|lab| lab.release("bin-fm.sh", "echo two\n")).0;
            }),
            ("firstmate-modified", |s| {
                *s = firstmate_lab(|lab| lab.edit("AGENTS.md", "You are my first mate.\n")).0;
            }),
            ("firstmate-blocked", |s| {
                *s = firstmate_lab(|lab| {
                    lab.edit("AGENTS.md", "You are my first mate.\n");
                    lab.release("AGENTS.md", "You are the bosun.\n");
                })
                .0;
            }),
            // The chief of staff's Markdown, formatted: the captain's screenshot of raw
            // asterisks, every construct, and a reply still arriving with its bold unclosed.
            ("formatted", |s| {
                *s = stub();
                s.ws.chat
                    .push(Message::new(Author::ChiefOfStaff, SCREENSHOT_REPLY));
            }),
            ("formatted-all", |s| {
                *s = stub();
                s.ws.items.clear();
                s.ws.chat = vec![Message::new(Author::ChiefOfStaff, EVERY_CONSTRUCT)];
            }),
            ("formatted-streaming", |s| {
                *s = stub();
                s.take(agent::Event::Reply {
                    text: "The fleet is **empty: nothing".into(),
                    done: false,
                });
            }),
            // Tool uses from the fake `claude`: an image Read returned, one that cannot be
            // decoded, a video file, a command's output unfolded, and an image enlarged.
            ("tool-image", |s| *s = shown(&fixture("chart.png"))),
            ("tool-image-broken", |s| *s = shown(&fixture("broken.png"))),
            ("tool-video", |s| *s = shown(&fixture("clip.mp4"))),
            ("tool-generic", |s| {
                *s = fake_in(Mode::Run);
                say(s, "shell: ls");
                s.unfolded.insert("toolu_fake_Bash".into());
            }),
            ("tool-picture", |s| {
                *s = shown(&fixture("chart.png"));
                s.showing = Showing::Picture("toolu_fake_Read".into());
            }),
            // Views the chief of staff showed with `app_ui` (json-render specs): every kind of
            // component, the same view half streamed, state bound to inputs and a repeat after
            // the captain ticked, typed and added, a submit and the reply to it, a component the
            // catalog lacks, and a spec over the limits.
            ("view-static", |s| {
                *s = viewing(spec("ui-release.json"), true)
            }),
            ("view-streaming", |s| {
                let whole = fixture_text("ui-release.json");
                *s = viewing(ui::partial(&whole[..whole.len() * 11 / 20]), false);
            }),
            ("view-state", |s| {
                *s = viewing(spec("ui-todos.json"), true);
                s.tick(VIEW, Some("/todos/1/done"), true, None, String::new());
                s.view_edit(VIEW, "/draft", "").text = "Update the README".into();
                press(s, "add");
                s.view_edit(VIEW, "/draft", "").text = "Announce it".into();
            }),
            ("view-action", |s| {
                *s = viewing(spec("ui-todos.json"), true);
                press(s, "send");
            }),
            ("view-unknown", |s| {
                *s = viewing(spec("ui-unknown.json"), true)
            }),
            ("view-too-large", |s| *s = viewing(too_large(), true)),
            // Visual replies: the settings page, a list the fake sent as text that was sent back
            // once and shown as a view, and a reply that ignored the feedback, sent back once.
            ("visuals-settings", |s| {
                *s = stub();
                s.showing = Showing::Visuals;
            }),
            ("visuals-enforced", |s| {
                *s = fake_in(Mode::Run);
                say_until(s, LIST, "Shown above.");
            }),
            ("visuals-capped", |s| {
                *s = fake_in(Mode::Run);
                say_until(
                    s,
                    "stubborn: Merge the docs PR, Renew the domain, Answer the question",
                    "Left as text.",
                );
            }),
            // Firstmate's rules, on the fake `claude` running as firstmate: the page after a
            // forced push was denied and a merge was asked about and allowed, the merge question,
            // and a write into a project.
            ("rules-settings", |s| {
                *s = rules_lab("rules-settings");
                say(s, "run: git push --force origin main");
                s.draft.text = format!("run: {MERGE}");
                s.send();
                s.settle(|s| s.ws.needs_input().next().is_some());
                let item = s.ws.needs_input().next().map(|i| i.id).unwrap();
                s.act(Action::Answer {
                    item,
                    choice: "Allow".into(),
                });
                s.settle(replied);
                s.showing = Showing::Rules;
            }),
            ("rules-merge-ask", |s| {
                *s = rules_lab("rules-merge-ask");
                s.draft.text = format!("run: {MERGE}");
                s.send();
                s.settle(|s| s.ws.needs_input().next().is_some());
            }),
            // Find in the conversation: a long chat for scripts/find.json, matches with the first
            // current, the third current after stepping, a query with none, and the history kept
            // from an earlier session.
            ("long-chat", |s| *s = long_chat(40)),
            ("find", |s| *s = finding("release", 0)),
            ("find-stepped", |s| *s = finding("release", 2)),
            ("find-none", |s| *s = finding("zebra", 0)),
            ("history", |s| {
                *s = reopened("history");
                s.find.open = true;
                s.find.query.text = "zeppelin".into();
            }),
            // Message links: a link copied, one opened scrolling to its message, a link in the
            // captain's message quoted to the chief of staff, and one to a missing message.
            ("link-copied", |s| {
                *s = stub();
                s.hovered = Some(1);
                s.copied = Some(s.ws.chat[1].id.clone());
            }),
            ("link-opened", |s| {
                *s = long_chat(40);
                s.ws.items.clear();
                s.go_to("000000000005");
            }),
            ("link-context", |s| {
                *s = fake_in(Mode::Run);
                say(s, "Plan: merge the docs PR first, then tag the release.");
                let answer = links::url(&s.ws.chat[s.ws.chat.len() - 1].id);
                say(s, &format!("do what this answer says: {answer}"));
            }),
            ("link-missing", |s| {
                *s = fake_in(Mode::Run);
                say(s, &format!("what did {} say?", links::url("0000deadbeef")));
            }),
            // Settings: every section, the confirmation before clearing the kept conversation,
            // and the menus the window draws where Rune has no native menu bar (Windows' File
            // menu, Linux's Edit menu).
            ("settings", |s| {
                *s = App::new(
                    Box::new(Stub::default()),
                    installed_firstmate(),
                    &shown_dir("settings"),
                );
                s.showing = Showing::Settings;
            }),
            ("settings-clear", |s| {
                *s = reopened("settings-clear");
                s.showing = Showing::Settings;
                s.confirm_clear = true;
            }),
            ("menu-windows", |s| {
                *s = stub();
                s.in_window_menus = true;
                s.platform = menus::Platform::Windows;
                s.open_menu = Some("File");
            }),
            ("menu-linux", |s| {
                *s = stub();
                s.in_window_menus = true;
                s.platform = menus::Platform::Linux;
                s.open_menu = Some("Edit");
            }),
            ("rules-project-ask", |s| {
                *s = rules_lab("rules-project-ask");
                let file = s.firstmate.home_dir().join("projects/site/README.md");
                s.draft.text = format!("write: {} Hello", file.display());
                s.send();
                s.settle(|s| s.ws.needs_input().next().is_some());
            }),
            // Reminders on a fixed clock, Friday 2:30 PM: the Reminders page, two that came due
            // (one overdue) with the banner and For you to do, the snooze menu, and the settings.
            ("reminders-list", |s| *s = reminders_set().0),
            ("reminder-fired", |s| *s = reminders_due().0),
            ("reminder-snooze", |s| {
                let (app, stretch) = reminders_due();
                *s = app;
                s.snoozing = Some(("banner", stretch));
            }),
            ("reminder-settings", |s| {
                *s = on_clock(&scratch("unused"), friday(14, 30)).0;
                s.showing = Showing::ReminderSettings;
            }),
            // A due reminder's "Set in this message", pressed: the chat at the captain's message,
            // highlighted.
            ("reminder-link-opened", |s| {
                *s = reminders_due().0;
                let message = s.ws.chat[0].id.clone();
                s.go_to(&message);
            }),
        ],
        min_size: Some((900.0, 560.0)),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(on_key),
        ..AppSpec::new("Velvet Hammer", (1100.0, 720.0), init, view::view)
    })
    // A message link the system opened with the app; macOS needs the app bundle to declare the
    // scheme first (see docs/links.md).
    .on_open_url(|s: &mut App, url: &str, tx: &mut Tx| s.open_link(url, tx))
    // The Mac's Edit and Window menus, with Settings, Find and Help added. Rune has no native
    // menu bar elsewhere, where the window draws its own (see menus.rs); headless runs keep the
    // native items so scripts can pick them.
    .standard_menus();
    if cfg!(target_os = "macos") || rune::is_headless() {
        for item in menus::native() {
            app = app.menu_item(item);
        }
    }
    app.run();
}

/// Friday, 2026-11-13, at `hour:minute` in US Eastern time, for the reminder scenes and tests.
fn friday(hour: i8, minute: i8) -> jiff::Zoned {
    let eastern = jiff::tz::TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").expect("a POSIX zone");
    jiff::civil::date(2026, 11, 13)
        .at(hour, minute, 0, 0)
        .to_zoned(eastern)
        .expect("a time in the zone")
}

/// The hands of a clock the reminder scenes and tests move themselves.
type Hands = Arc<Mutex<jiff::Zoned>>;

fn clock(hands: &Hands) -> reminders::Clock {
    let hands = hands.clone();
    Arc::new(move || hands.lock().unwrap_or_else(|e| e.into_inner()).clone())
}

/// The stub app keeping its data in `data`, its reminders on a clock set to `now`.
fn on_clock(data: &Path, now: jiff::Zoned) -> (App, Hands) {
    let hands = Arc::new(Mutex::new(now));
    let app = App::with_clock(
        Box::new(Stub::default()),
        installed_firstmate(),
        data,
        clock(&hands),
    );
    (app, hands)
}

/// Reminders set at 2:30 PM on Friday for each form, shown on the Reminders page.
fn reminders_set() -> (App, Hands) {
    let (mut app, hands) = on_clock(&scratch("unused"), friday(14, 30));
    for (text, when) in [
        ("Check the deploy", When::In(20, Unit::Minutes)),
        ("Reply to the design review", When::LaterToday),
        ("The standup", When::TomorrowMorning),
        ("Send the invoice", When::In(3, Unit::Days)),
        ("Plan the week", When::NextWeek),
        ("Renew the velvet-hammer.dev domain", When::Later),
        ("Read the Rune changelog", When::Later),
    ] {
        app.reminders.set(text, when).expect("a reminder");
    }
    app.showing = Showing::Reminders;
    app.drain();
    (app, hands)
}

/// Two reminders that came due at 2:32 PM: one set at 9:00 AM for 10:00 AM, which the app
/// shows as overdue because it fired late, and one set at 2:30 PM for two minutes on, with the
/// id of the second. Two more wait.
fn reminders_due() -> (App, u64) {
    let (mut app, hands) = on_clock(&scratch("unused"), friday(9, 0));
    // Set in reply to the stub's first message, which the reminders link back to.
    app.reminders.said_in(&app.ws.chat[0].id);
    app.reminders
        .set("Call the bank about the wire", When::In(1, Unit::Hours))
        .expect("a reminder");
    *hands.lock().unwrap() = friday(14, 30);
    for (text, when) in [
        ("Stretch", When::In(2, Unit::Minutes)),
        ("Check the deploy", When::LaterToday),
        ("Renew the velvet-hammer.dev domain", When::Later),
    ] {
        app.reminders.set(text, when).expect("a reminder");
    }
    *hands.lock().unwrap() = friday(14, 32);
    app.drain();
    let stretch = app
        .reminders
        .list()
        .iter()
        .find(|r| r.text == "Stretch")
        .map(|r| r.id)
        .expect("set above");
    (app, stretch)
}

/// The list the visual-replies scenes and tests have the fake send as text.
const LIST: &str = "list: Merge the docs PR, Renew the domain, Answer the window-size question";

/// A merge through firstmate's script, which its rules ask the captain about.
const MERGE: &str = "bin/fm-pr-merge.sh site-docs https://github.com/captain/site/pull/7";

/// The fake `claude` running as firstmate in a lab under `/tmp/velvet-hammer/NAME`, so the
/// screenshots show a short path that names no one's home directory, with three projects
/// registered.
fn rules_lab(name: &str) -> App {
    let lab = Lab::at(&Path::new("/tmp/velvet-hammer").join(name));
    lab.registry(projects::SAMPLE_REGISTRY);
    let mut app = fake_in_firstmate(&lab, true);
    app.settle(|s| s.projects.as_ref().is_some_and(|p| p.is_ok()) && !s.has_no_projects());
    app
}

/// Rules for the tool-approval scenes: ask before any push, never force one, and allow edits
/// under `src`.
fn sample_rules() -> Vec<approval::Rule> {
    use approval::{Effect, Rule};
    let rule = |effect, tool: &str, pattern: Option<&str>| Rule {
        effect,
        tool: tool.into(),
        pattern: pattern.map(str::to_string),
    };
    vec![
        rule(Effect::Ask, "Bash", Some("git push")),
        rule(Effect::Deny, "Bash", Some("git push --force")),
        rule(Effect::Allow, "Edit", Some("/Users/captain/project/src/*")),
    ]
}

/// The reply in the captain's screenshot, which showed its asterisks before replies were
/// formatted.
const SCREENSHOT_REPLY: &str = "Captain, the fleet is empty: nothing is running, queued, or \
recently landed.

**Captain's Call**
No decision is recorded, captain. Contribution coverage is unverified (0 of 0 checked, and one \
contribution record couldn't be read), so I can't say it's clear.

**Recently Landed**
No recent completions are in the current baseline.

**Underway**
Nothing is underway.

**Charted Next**
Nothing is queued.

Since the fleet is empty, I can change shared firstmate material directly if you want that. I \
can also look into the unreadable contribution record.";

/// Every Markdown construct the chat formats, and raw HTML, which shows as text.
const EVERY_CONSTRUCT: &str = "# Heading one
## Heading two
### Heading three

Plain, **bold**, *italic*, ~~struck~~, `inline code` and a [link](https://example.com).
A second line, kept as written.

- First bullet
- Second bullet
  - Nested bullet

1. First step
2. Second step

> A quoted line.

```rust
fn main() {
    println!(\"hello\");
}
```

<script>alert(1)</script> stays text, as does <b>this</b>.";

/// A copy of a file in `scripts/fixtures` at `/tmp/velvet-hammer/NAME`, so the scenes and their
/// screenshots show a short path that names no one's home directory. `clip.mp4` is Rune's own
/// test clip (`crates/rune/tests/fixtures/rgbw.mp4`, MIT licensed).
fn fixture(name: &str) -> String {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scripts/fixtures")
        .join(name);
    let dir = Path::new("/tmp/velvet-hammer");
    std::fs::create_dir_all(dir).expect("fixture directory");
    let path = dir.join(name);
    // Tests run at once and may copy the same fixture; a rename never shows a half-written file.
    let partial = dir.join(format!("{name}.{}.part", std::process::id()));
    std::fs::copy(&source, &partial).expect("fixture copy");
    std::fs::rename(&partial, &path).expect("fixture rename");
    path.display().to_string()
}

/// The fake `claude` after it used Read on `path`, with any image decoded.
fn shown(path: &str) -> App {
    let mut app = fake_in(Mode::Run);
    say(&mut app, &format!("show: {path}"));
    for media in app.media.values() {
        if let Some(Ok(image)) = &media.image {
            let _ = image.wait();
        }
    }
    app
}

/// A file in `scripts/fixtures`, as written.
fn fixture_text(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scripts/fixtures")
        .join(name);
    std::fs::read_to_string(path).expect("fixture")
}

/// A json-render spec in `scripts/fixtures`.
fn spec(name: &str) -> serde_json::Value {
    serde_json::from_str(&fixture_text(name)).expect("fixture JSON")
}

/// A spec with more elements than the limit.
fn too_large() -> serde_json::Value {
    let mut elements = serde_json::Map::new();
    let ids: Vec<String> = (0..=ui::MAX_ELEMENTS)
        .map(|i| format!("line-{i}"))
        .collect();
    for id in &ids {
        elements.insert(
            id.clone(),
            serde_json::json!({"type": "Text", "props": {"text": id}, "children": []}),
        );
    }
    elements.insert(
        "main".into(),
        serde_json::json!({"type": "Stack", "props": {}, "children": ids}),
    );
    serde_json::json!({"root": "main", "elements": elements})
}

/// The id of the view tool use in the view scenes and tests.
const VIEW: &str = "toolu_view";

/// The stub app after the captain asked and the chief of staff started showing a view with
/// `input`, the tool's answer arrived when `done`.
fn viewing(input: serde_json::Value, done: bool) -> App {
    let mut app = stub();
    app.ws.chat = vec![Message::new(
        Author::Captain,
        "Where does the release stand?",
    )];
    app.take(Event::Tool(tools::ToolUse {
        id: VIEW.into(),
        name: ui::tool_name(),
        input: serde_json::json!({}),
        result: None,
    }));
    app.take(Event::ToolInput {
        id: VIEW.into(),
        input: input.clone(),
    });
    if done {
        let (is_error, text) = match ui::answer(&input) {
            Ok(text) => (false, text),
            Err(problem) => (true, problem),
        };
        app.take(Event::ToolResult {
            id: VIEW.into(),
            result: tools::ToolResult {
                is_error,
                content: vec![tools::Content::Text(text)],
            },
        });
    }
    app
}

/// Presses the control drawn for the element `id` in the scenes' view, as the captain would.
fn press(s: &mut App, id: &str) {
    fn find(node: &ui::Node, id: &str) -> Option<(ui::Trigger, String)> {
        match &node.what {
            ui::What::Button {
                label,
                press: Some(press),
                ..
            } if node.key == id => {
                Some((press.clone(), format!("Pressed \u{201c}{label}\u{201d}")))
            }
            _ => node.children.iter().find_map(|c| find(c, id)),
        }
    }
    let spec = s.view_spec(VIEW).expect("a view");
    let state =
        s.ui.get(VIEW)
            .map_or(spec.state.clone(), |l| l.state(&spec));
    let tree = ui::tree(&spec, &state);
    let (trigger, did) = tree
        .root
        .as_ref()
        .and_then(|r| find(r, id))
        .expect("a button");
    s.fire(VIEW, &trigger, did);
}

/// The stub's chat with `n` more messages from the chief of staff, every tenth naming the
/// release, so the matches are far apart.
fn long_chat(n: usize) -> App {
    let mut app = stub();
    for i in 0..n {
        let words = if i % 10 == 0 {
            format!("Reply {i}: the **release** is on track.")
        } else {
            format!("Reply {i}: nothing new.")
        };
        let mut reply = Message::new(Author::ChiefOfStaff, words);
        // Fixed ids, so scripts can link to them (scripts/open-link.json).
        reply.id = format!("{i:012x}");
        app.ws.chat.push(reply);
    }
    app
}

/// The stub's chat with the find bar open on `query`, at match `current`.
fn finding(query: &str, current: usize) -> App {
    let mut app = long_chat(40);
    app.ws.items.clear();
    app.find.open = true;
    app.find.query.text = query.into();
    app.find.searched = query.into();
    app.find.current = current;
    app.find.reveal = true;
    app
}

/// The stub on a data directory where an earlier session asked about the zeppelin plan.
fn reopened(name: &str) -> App {
    let data = shown_dir(name);
    let mut first = App::new(Box::new(Stub::default()), installed_firstmate(), &data);
    first.draft.text = "Remember the zeppelin plan for the launch.".into();
    first.send();
    drop(first);
    App::new(Box::new(Stub::default()), installed_firstmate(), &data)
}

/// An empty data directory at `/tmp/velvet-hammer/NAME`, for scenes whose screenshots show where
/// a file is kept, so they name no one's home directory.
fn shown_dir(name: &str) -> PathBuf {
    let dir = Path::new("/tmp/velvet-hammer").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scene directory");
    dir
}

/// Sends `text` and waits for the reply, for scenes.
fn say(s: &mut App, text: &str) {
    s.draft.text = text.into();
    s.send();
    s.settle(replied);
}

/// Sends `text` and waits until the chief of staff's last words are `end`, for turns the stop
/// hook continues past a first reply.
fn say_until(s: &mut App, text: &str, end: &str) {
    s.draft.text = text.into();
    s.send();
    s.settle(|s| s.replying.is_none() && s.ws.chat.last().is_some_and(|m| m.text == end));
}

/// A command whose dangerous part sits far past where a short preview would have cut it off.
fn long_command() -> String {
    format!("echo {}; rm -rf ~", "a harmless-looking prefix ".repeat(20))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Item;
    use rune::{Env, Runtime};

    fn runtime() -> Runtime<App> {
        runtime_with(stub())
    }

    fn runtime_with(app: App) -> Runtime<App> {
        let env = Env {
            system_dark: true,
            scale: 2.0,
        };
        let mut rt = Runtime::new(app, view::view, (1100.0, 720.0), env);
        rt.poll = Some(poll);
        rt.on_key = Some(on_key);
        rt.frame(0.0);
        rt
    }

    fn tap(rt: &mut Runtime<App>, key: &str) {
        // Lets any transition finish first, so the element is where the pointer goes.
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        let (x, y) = rt
            .find(key)
            .unwrap_or_else(|| panic!("no element keyed {key}"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
    }

    #[test]
    fn sending_a_message_adds_it_and_a_reply() {
        let mut rt = runtime();
        let before = rt.state.ws.chat.len();
        rt.state.draft.text = "hello".into();
        rt.frame(1.0 / 60.0);
        tap(&mut rt, "send");
        let chat = &rt.state.ws.chat;
        assert_eq!(chat.len(), before + 2);
        assert_eq!(chat[before].text, "hello");
        assert_eq!(chat[before + 1].author, model::Author::ChiefOfStaff);
        assert!(rt.state.draft.text.is_empty());
    }

    #[test]
    fn an_empty_draft_sends_nothing() {
        let mut rt = runtime();
        let before = rt.state.ws.chat.len();
        rt.state.draft.text = "   ".into();
        tap(&mut rt, "send");
        assert_eq!(rt.state.ws.chat.len(), before);
    }

    #[test]
    fn answering_an_item_removes_it_from_its_panel() {
        let mut rt = runtime();
        assert!(rt.find("input-1").is_some());
        tap(&mut rt, "answer-1-0");
        assert!(rt.state.ws.item(1).is_none());
        assert!(rt.find("input-1").is_none());
        assert!(rt.state.ws.chat.last().unwrap().text.contains("1100×720"));
    }

    #[test]
    fn marking_a_task_done_removes_it_and_shows_the_empty_state() {
        let mut rt = runtime();
        assert!(rt.find("for-you-empty").is_none());
        tap(&mut rt, "done-3");
        assert!(rt.state.ws.item(3).is_none());
        assert!(rt.find("for-you-empty").is_some());
    }

    #[test]
    fn peeking_at_an_agent_replaces_the_chat_until_closed() {
        let mut rt = runtime();
        tap(&mut rt, "agent-2");
        assert_eq!(rt.state.showing, Showing::Peek(2));
        assert!(rt.find("peek").is_some());
        assert!(rt.find("chat").is_none());
        tap(&mut rt, "peek-close");
        assert_eq!(rt.state.showing, Showing::Chat);
        assert!(rt.find("chat").is_some());
    }

    // The chat running as firstmate, on the fake `claude` and a scratch upstream.

    #[test]
    fn the_chat_runs_in_the_managed_clone_with_its_own_home() {
        let lab = Lab::new();
        let mut rt = runtime_with(fake_in_firstmate(&lab, true));
        let home = lab.data.join("home");
        assert!(!home.exists(), "the home is made when the session starts");
        assert!(rt.find("workplace").is_some());
        rt.state.draft.text = "whereami".into();
        tap(&mut rt, "send");
        wait(&mut rt, replied);
        let clone = std::fs::canonicalize(lab.clone_dir()).unwrap();
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            format!(
                "You said: cwd={} FM_HOME={}",
                clone.display(),
                home.display()
            )
        );
        for part in ["state", "data", "config", "projects"] {
            assert!(home.join(part).is_dir(), "{part} is missing from the home");
        }
    }

    #[test]
    fn the_panel_shows_what_firstmates_records_hold_for_the_captain() {
        let lab = Lab::new();
        let mut rt = runtime_with(fake_in_firstmate(&lab, true));
        wait(&mut rt, |s| s.captain_problem.is_none());
        assert!(rt.find("needs-input-empty").is_some());
        lab.bearings(bearings::SNAPSHOT);
        wait(&mut rt, |s| !s.captain.is_empty());
        assert!(rt.find("hold-lab-window-size").is_some());
        assert!(rt.find("hold-lab-gh-login").is_some());
        assert!(rt.find("merge-lab-panels").is_some());
        assert!(rt.find("needs-input-empty").is_none());
        // Firstmate closed one hold: it leaves the panel when the records change.
        lab.bearings(r#"{"schema":"fm-bearings.v1","decisions_open":[{"key":"lab-gh-login","question":"Sign in"}]}"#);
        wait(&mut rt, |s| s.captain.holds.len() == 1);
        assert!(rt.find("hold-lab-window-size").is_none());
        assert!(rt.find("merge-lab-panels").is_none());
    }

    #[test]
    fn answering_a_hold_sends_the_captains_words_to_the_chief_of_staff() {
        let lab = Lab::new();
        lab.bearings(bearings::SNAPSHOT);
        let mut rt = runtime_with(fake_in_firstmate(&lab, true));
        wait(&mut rt, |s| !s.captain.is_empty());
        tap(&mut rt, "answer-lab-window-size");
        assert_eq!(rt.state.answering.as_deref(), Some("lab-window-size"));
        assert!(rt.find("answering").is_some());
        tap(&mut rt, "answer-cancel");
        assert_eq!(rt.state.answering, None);
        tap(&mut rt, "answer-lab-window-size");
        rt.state.draft.text = "1280x800".into();
        tap(&mut rt, "send");
        wait(&mut rt, replied);
        let chat = &rt.state.ws.chat;
        assert_eq!(
            chat[0].text,
            "My answer to the captain hold lab-window-size: 1280x800"
        );
        assert_eq!(chat[1].text, format!("You said: {}", chat[0].text));
        assert_eq!(rt.state.answering, None);
        // The hold stays in the panel until firstmate's records say it is closed.
        assert!(rt.find("hold-lab-window-size").is_some());
    }

    #[test]
    fn approving_a_merge_tells_the_chief_of_staff_in_the_chat() {
        let lab = Lab::new();
        // Only the merge, so its button is not scrolled out of the panel.
        lab.bearings(
            r#"{"schema":"fm-bearings.v1","contributions":{"captain":[{"task":"lab-panels",
            "url":"https://github.com/example/app/pull/7","reason":"checks green","hold":null}]}}"#,
        );
        let mut rt = runtime_with(fake_in_firstmate(&lab, true));
        wait(&mut rt, |s| !s.captain.is_empty());
        tap(&mut rt, "approve-lab-panels");
        wait(&mut rt, replied);
        assert_eq!(
            rt.state.ws.chat[0].text,
            "I approve merging https://github.com/example/app/pull/7 (task lab-panels)."
        );
    }

    #[test]
    fn a_snapshot_firstmate_could_not_produce_says_why_and_keeps_the_last_one() {
        let lab = Lab::new();
        lab.bearings(bearings::SNAPSHOT);
        let mut rt = runtime_with(fake_in_firstmate(&lab, true));
        wait(&mut rt, |s| !s.captain.is_empty());
        lab.bearings(r#"{"schema":"fm-bearings.v9"}"#);
        wait(&mut rt, |s| s.captain_problem.is_some());
        assert!(rt.find("captain-problem").is_some());
        assert!(rt.find("hold-lab-window-size").is_some());
    }

    #[test]
    #[ignore = "clones firstmate from GitHub and needs its tasks-axi and jq on PATH"]
    fn real_firstmate_reports_a_captain_hold() {
        let data = scratch("unused").join("data");
        let mut firstmate = Firstmate::open(&data, firstmate::UPSTREAM);
        firstmate.install();
        let deadline = Instant::now() + Duration::from_secs(120);
        while !firstmate.poll() {
            assert!(Instant::now() < deadline, "the clone never finished");
            std::thread::sleep(Duration::from_millis(50));
        }
        let (clone, home) = (firstmate.clone_dir(), firstmate.home_dir());
        firstmate::prepare_home(&home).unwrap();
        // Firstmate's own command for holding a task for the captain.
        let held = std::process::Command::new(clone.join("bin/fm-captain-hold.sh"))
            .args(["hold", "vh-window-size", "--title", "Default window size"])
            .args(["--reason", "Pick 1100x720 or 1280x800."])
            .current_dir(clone)
            .env("FM_HOME", &home)
            .output()
            .unwrap();
        assert!(
            held.status.success(),
            "{}",
            String::from_utf8_lossy(&held.stderr)
        );
        let mut watcher = bearings::Watcher::start(clone, &home);
        let captain = loop {
            assert!(Instant::now() < deadline, "no snapshot arrived");
            match watcher.poll() {
                Some(snapshot) => break snapshot.unwrap(),
                None => std::thread::sleep(Duration::from_millis(50)),
            }
        };
        assert_eq!(captain.holds.len(), 1);
        assert_eq!(captain.holds[0].key, "vh-window-size");
        assert_eq!(
            captain.holds[0].question,
            "Default window size: Pick 1100x720 or 1280x800."
        );
    }

    #[test]
    fn the_agents_list_shows_firstmates_tasks_in_flight() {
        let lab = Lab::new();
        let mut rt = runtime_with(fake_in_firstmate(&lab, true));
        wait(&mut rt, |s| s.captain_problem.is_none());
        assert!(rt.find("crew-lab-verify").is_none());
        lab.bearings(bearings::SNAPSHOT);
        wait(&mut rt, |s| s.captain.crew.len() == 2);
        assert!(rt.find("crew-lab-verify").is_some());
        assert!(rt.find("crew-lab-layout").is_some());
    }

    #[test]
    fn peeking_at_a_crewmate_reads_its_terminal_until_closed() {
        let lab = Lab::new();
        lab.bearings(bearings::SNAPSHOT);
        lab.pane("lab-verify", "rendering chat.png\n\n\n");
        let mut rt = runtime_with(fake_in_firstmate(&lab, true));
        wait(&mut rt, |s| !s.captain.crew.is_empty());
        tap(&mut rt, "crew-lab-verify");
        assert_eq!(rt.state.showing, Showing::Crew("lab-verify".into()));
        wait(&mut rt, |s| {
            s.peek.as_ref().is_some_and(|p| p.pane.is_some())
        });
        let pane = |rt: &Runtime<App>| rt.state.peek.as_ref().unwrap().pane.clone().unwrap();
        assert_eq!(pane(&rt), Ok("rendering chat.png".into()));
        assert!(rt.find("crew-pane").is_some());
        assert!(rt.find("chat").is_none());
        // The terminal moves on, and the peek follows.
        lab.pane("lab-verify", "rendering chat.png\nrendering peek.png\n");
        wait(&mut rt, |s| {
            s.peek.as_ref().unwrap().pane
                == Some(Ok("rendering chat.png\nrendering peek.png".into()))
        });
        tap(&mut rt, "peek-close");
        assert!(
            rt.state.peek.is_none(),
            "the read stops when the peek closes"
        );
        assert!(rt.find("chat").is_some());
    }

    #[test]
    fn a_crewmate_firstmate_cannot_read_says_why() {
        let lab = Lab::new();
        lab.bearings(bearings::SNAPSHOT);
        let mut rt = runtime_with(fake_in_firstmate(&lab, true));
        wait(&mut rt, |s| !s.captain.crew.is_empty());
        tap(&mut rt, "crew-lab-layout");
        wait(&mut rt, |s| {
            s.peek.as_ref().is_some_and(|p| p.pane.is_some())
        });
        // Firstmate's reason, not its supervision banner.
        assert_eq!(
            rt.state.peek.as_ref().unwrap().pane,
            Some(Err("error: no endpoint for lab-layout".into()))
        );
        assert!(rt.find("crew-peek-problem").is_some());
        assert!(rt.find("crew-pane").is_none());
    }

    #[test]
    fn a_turn_nobody_asked_for_shows_in_the_chat() {
        let mut rt = runtime_with(fake_in_firstmate(&Lab::new(), true));
        rt.state.draft.text = "wake: A crewmate finished its task.".into();
        tap(&mut rt, "send");
        wait(&mut rt, |s| {
            s.replying.is_none()
                && s.ws
                    .chat
                    .last()
                    .is_some_and(|m| m.text == "A crewmate finished its task.")
        });
        let chat = &rt.state.ws.chat;
        assert_eq!(chat.len(), 3);
        assert_eq!(
            chat[1].text,
            "You said: wake: A crewmate finished its task."
        );
        assert_eq!(chat[2].author, Author::ChiefOfStaff);
    }

    #[test]
    fn the_chat_waits_for_firstmate_to_be_installed() {
        let lab = Lab::new();
        let mut rt = runtime_with(fake_in_firstmate(&lab, false));
        assert!(matches!(rt.state.status, Status::Ready(_)));
        assert!(rt.find("chat-input").is_none());
        assert!(rt.find("firstmate-page").is_some());
        rt.state.draft.text = "hello".into();
        rt.state.send();
        assert!(rt.state.ws.chat.is_empty());
        tap(&mut rt, "firstmate-install");
        wait(&mut rt, idle);
        assert!(rt.find("chat-input").is_some());
        assert!(rt.find("firstmate-page").is_none());
    }

    // Projects, from firstmate's records, and adding one with the chief of staff.

    fn projects_read(s: &App) -> bool {
        s.projects.is_some()
    }

    #[test]
    fn a_fresh_install_has_no_projects_and_adds_them_once_it_can_chat() {
        let lab = Lab::new();
        let mut rt = runtime_with(fake_in_firstmate(&lab, false));
        wait(&mut rt, projects_read);
        assert!(!lab.data.join("home").exists(), "reading made no home");
        assert!(rt.state.has_no_projects());
        assert!(rt.find("projects-none").is_some());
        assert!(rt.find("add-project-later").is_some());
        assert!(rt.find("add-project").is_none());
        rt.state.add_project();
        assert!(!rt.state.adding);
        assert!(rt.state.ws.chat.is_empty());
        tap(&mut rt, "firstmate-install");
        wait(&mut rt, idle);
        assert!(rt.find("projects-empty").is_some());
        assert!(rt.find("add-first-project").is_some());
        assert!(rt.find("add-project").is_some());
    }

    #[test]
    fn adding_a_project_is_a_conversation_and_the_project_appears_when_firstmate_registers_it() {
        let lab = Lab::new();
        let mut rt = runtime_with(fake_in_firstmate(&lab, true));
        wait(&mut rt, projects_read);
        assert!(rt.find("adding").is_none());
        tap(&mut rt, "add-first-project");
        assert!(rt.state.adding);
        wait(&mut rt, replied);
        assert_eq!(rt.state.ws.chat[0].text, ADD_PROJECT);
        assert_eq!(rt.state.ws.chat[1].text, format!("You said: {ADD_PROJECT}"));
        assert!(rt.find("projects-empty").is_none());
        assert!(rt.find("adding").is_some());
        // The chief of staff asked; the captain's yes goes to it in the chat.
        tap(&mut rt, "add-yes");
        assert_eq!(rt.state.ws.chat[2].text, "Yes.");
        wait(&mut rt, replied);
        assert!(rt.find("add-yes").is_some());
        // Firstmate's procedure registered the project: it appears without a restart.
        lab.registry("- notes [local-only] - Meeting notes (added 2026-11-13)\n");
        wait(&mut rt, |s| !s.has_no_projects());
        assert!(rt.find("project-notes").is_some());
        assert!(rt.find("projects-none").is_none());
        // The captain can add another, or stop.
        assert!(rt.find("add-project").is_some());
        tap(&mut rt, "add-finish");
        assert!(!rt.state.adding);
        assert!(rt.find("adding").is_none());
        assert!(rt.find("projects-empty").is_none());
    }

    #[test]
    fn choosing_a_folder_puts_its_path_in_the_draft_to_send() {
        let mut rt = runtime_with(fake_in_firstmate(&Lab::new(), true));
        wait(&mut rt, projects_read);
        tap(&mut rt, "add-project");
        wait(&mut rt, replied);
        let sent = rt.state.ws.chat.len();
        rt.state.picker = || Ok(Some(PathBuf::from("/Users/captain/code/notes")));
        tap(&mut rt, "add-pick-folder");
        wait(&mut rt, |s| !s.picking());
        assert_eq!(rt.state.draft.text, "/Users/captain/code/notes");
        assert_eq!(
            rt.state.ws.chat.len(),
            sent,
            "nothing is sent until the captain sends it"
        );
        // Cancelling the dialog changes nothing; a dialog that cannot open says why.
        rt.state.draft.clear();
        rt.state.picker = || Ok(None);
        tap(&mut rt, "add-pick-folder");
        wait(&mut rt, |s| !s.picking());
        assert!(rt.state.draft.text.is_empty());
        rt.state.picker = || Err("Could not open a folder dialog.".into());
        tap(&mut rt, "add-pick-folder");
        wait(&mut rt, |s| !s.picking());
        assert!(rt.find("add-pick-problem").is_some());
    }

    #[test]
    fn a_registry_that_cannot_be_read_says_why_instead_of_prompting() {
        let lab = Lab::new();
        // A directory where the registry file belongs.
        std::fs::create_dir_all(lab.data.join("home/data/projects.md")).unwrap();
        let mut rt = runtime_with(fake_in_firstmate(&lab, true));
        wait(&mut rt, projects_read);
        assert!(!rt.state.has_no_projects());
        assert!(rt.find("projects-problem").is_some());
        assert!(rt.find("projects-empty").is_none());
    }

    #[test]
    #[ignore = "uses the real Claude account and clones firstmate from GitHub"]
    fn real_firstmate_adds_a_local_folder_as_a_project() {
        let data = scratch("unused").join("data");
        let folder = data.with_file_name("notes-repo");
        std::fs::create_dir_all(&folder).unwrap();
        let git = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .args(args)
                .current_dir(&folder)
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        };
        git(&["init", "--quiet", "-b", "main"]);
        std::fs::write(folder.join("README.md"), "Meeting notes\n").unwrap();
        git(&["add", "-A"]);
        git(&[
            "-c",
            "user.name=Lab",
            "-c",
            "user.email=lab@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "first",
        ]);
        let mut firstmate = Firstmate::open(&data, firstmate::UPSTREAM);
        let claude = Claude::new("claude")
            .env("ANTHROPIC_MODEL", "sonnet")
            .in_firstmate(firstmate.clone_dir(), firstmate.home_dir());
        firstmate.install();
        let mut rt = runtime_with(App::new(Box::new(claude), firstmate, &data));
        wait_for(&mut rt, Duration::from_secs(120), |s| {
            settled(s) && idle(s) && s.projects.is_some()
        });
        assert!(rt.state.has_no_projects());
        tap(&mut rt, "add-first-project");
        wait_for(&mut rt, Duration::from_secs(300), replied);
        let answers = [
            folder.display().to_string(),
            "Name it notes.".to_string(),
            "local-only, with no remote and yolo off.".to_string(),
        ];
        for answer in answers.iter().map(String::as_str).chain(["Yes."; 4]) {
            if !rt.state.has_no_projects() {
                break;
            }
            rt.state.draft.text = answer.into();
            tap(&mut rt, "send");
            wait_for(&mut rt, Duration::from_secs(300), replied);
        }
        wait_for(&mut rt, Duration::from_secs(10), |s| !s.has_no_projects());
        for m in &rt.state.ws.chat {
            eprintln!("--- {:?}\n{}", m.author, m.text);
        }
        let projects = rt.state.projects.clone().unwrap().unwrap();
        eprintln!("{projects:?}");
        eprintln!(
            "{}",
            std::fs::read_to_string(data.join("home/data/projects.md")).unwrap_or_default()
        );
        assert!(projects.iter().any(|p| p.name == "notes"), "{projects:?}");
        assert!(rt.find("project-notes").is_some());
    }

    #[test]
    fn the_stub_shows_no_projects_section() {
        let rt = runtime();
        assert!(rt.find("projects").is_none());
        assert!(rt.find("projects-empty").is_none());
    }

    #[test]
    #[ignore = "uses the real Claude account and clones firstmate from GitHub"]
    fn real_claude_starts_as_firstmate() {
        let data = scratch("unused").join("data");
        let mut firstmate = Firstmate::open(&data, firstmate::UPSTREAM);
        let claude = Claude::new("claude")
            .env("ANTHROPIC_MODEL", "haiku")
            .in_firstmate(firstmate.clone_dir(), firstmate.home_dir());
        firstmate.install();
        let mut rt = runtime_with(App::new(Box::new(claude), firstmate, &data));
        wait_for(&mut rt, Duration::from_secs(120), |s| settled(s) && idle(s));
        assert!(rt.state.chat_ready(), "{:?}", rt.state.firstmate.problem());
        rt.state.draft.text = "Use no tools. Quote the line of your session-start context that \
            begins with SESSION START, then name the file your instructions come from."
            .into();
        tap(&mut rt, "send");
        wait_for(&mut rt, Duration::from_secs(180), replied);
        let reply = &rt.state.ws.chat.last().unwrap().text;
        assert!(reply.contains("SESSION START"), "{reply}");
        assert!(
            reply.contains(&data.join("home").display().to_string()),
            "{reply}"
        );
        assert!(reply.contains("AGENTS.md"), "{reply}");
        assert!(data.join("home/state/.lock").exists());
    }

    // The managed firstmate clone, on a scratch upstream (see `firstmate::lab`).

    fn standing(rt: &Runtime<App>) -> firstmate::Standing {
        match rt.state.firstmate.distro() {
            Distro::Installed(checkout) => checkout.standing(),
            Distro::Missing => panic!("firstmate is not installed"),
        }
    }

    #[test]
    fn the_firstmate_card_opens_its_page_until_closed() {
        let mut rt = runtime();
        assert!(rt.find("firstmate-page").is_none());
        open_setting(&mut rt, "firstmate");
        assert_eq!(rt.state.showing, Showing::Firstmate);
        assert!(rt.find("firstmate-page").is_some());
        assert!(rt.find("firstmate-check").is_some());
        assert!(rt.find("firstmate-install").is_none());
        assert!(rt.find("chat").is_none());
        tap(&mut rt, "firstmate-close");
        assert!(rt.find("chat").is_some());
    }

    #[test]
    fn installing_firstmate_from_its_page_clones_it() {
        let lab = Lab::new();
        let mut rt = runtime_with(on_firstmate(lab.firstmate()));
        assert!(rt.find("firstmate-check").is_none());
        // One frame rather than `tap`'s thirty: a local clone can finish within those.
        let (x, y) = rt.find("firstmate-install").expect("install button");
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        assert_eq!(rt.state.firstmate.work(), Some(Work::Installing));
        wait(&mut rt, idle);
        assert_eq!(standing(&rt), firstmate::Standing::UpToDate);
        assert!(lab.clone_dir().join("AGENTS.md").exists());
        assert!(rt.find("firstmate-check").is_some());
        assert!(rt.find("firstmate-update").is_none());
    }

    #[test]
    fn updating_firstmate_from_its_page_keeps_an_edit() {
        let (app, lab) = firstmate_lab(|lab| lab.edit("AGENTS.md", "You are my first mate.\n"));
        let mut rt = runtime_with(app);
        assert!(rt.find("firstmate-edited").is_some());
        assert!(rt.find("firstmate-update").is_none());
        lab.release("bin-fm.sh", "echo two\n");
        tap(&mut rt, "firstmate-check");
        wait(&mut rt, idle);
        tap(&mut rt, "firstmate-update");
        wait(&mut rt, idle);
        assert_eq!(rt.state.firstmate.problem(), None);
        assert_eq!(standing(&rt), firstmate::Standing::Modified);
        assert!(lab.clone_dir().join("bin-fm.sh").exists());
        assert_eq!(
            std::fs::read_to_string(lab.clone_dir().join("AGENTS.md")).unwrap(),
            "You are my first mate.\n"
        );
    }

    #[test]
    fn a_blocked_update_names_the_files_and_offers_no_update() {
        let (app, _lab) = firstmate_lab(|lab| {
            lab.edit("AGENTS.md", "You are my first mate.\n");
            lab.release("AGENTS.md", "You are the bosun.\n");
        });
        let rt = runtime_with(app);
        assert_eq!(standing(&rt), firstmate::Standing::Blocked);
        assert!(rt.find("firstmate-conflicts").is_some());
        assert!(rt.find("firstmate-update").is_none());
        assert!(rt.find("firstmate-check").is_some());
    }

    // The rest run the app on `scripts/fake-claude`, a stand-in for Claude Code's `claude`.

    /// Frames until `done` holds, letting the fake's real processes run.
    fn wait(rt: &mut Runtime<App>, done: impl Fn(&App) -> bool) {
        wait_for(rt, Duration::from_secs(20), done);
    }

    fn wait_for(rt: &mut Runtime<App>, patience: Duration, done: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + patience;
        while !done(&rt.state) {
            assert!(
                Instant::now() < deadline,
                "timed out; status is {:?}",
                rt.state.status
            );
            std::thread::sleep(Duration::from_millis(20));
            rt.frame(1.0 / 60.0);
        }
        rt.frame(1.0 / 60.0);
    }

    #[test]
    fn signed_out_shows_the_sign_in_screen_instead_of_the_chat() {
        let mut rt = runtime_with(fake("signed-out", &[]));
        wait(&mut rt, settled);
        assert_eq!(rt.state.status, Status::SignedOut { problem: None });
        assert!(rt.find("sign-in-button").is_some());
        assert!(rt.find("chat-input").is_none());
    }

    #[test]
    fn signing_in_runs_claude_and_returns_to_the_chat() {
        let home = scratch("signed-out");
        let mut rt = runtime_with(on_fake(FAKE_CLAUDE, &home, &[]));
        wait(&mut rt, settled);
        tap(&mut rt, "sign-in-button");
        wait(&mut rt, |s| {
            matches!(s.status, Status::SigningIn { url: Some(_) })
        });
        assert!(rt.find("sign-in-url").is_some());
        assert!(rt.find("cancel-sign-in").is_some());
        // The captain approves in the browser.
        std::fs::write(home.join("browser-done"), "").unwrap();
        wait(&mut rt, |s| matches!(s.status, Status::Ready(_)));
        assert_eq!(
            rt.state.status,
            Status::Ready(model::Plan::Subscription("max".into()))
        );
        assert!(rt.find("chat-input").is_some());
    }

    #[test]
    fn a_failed_sign_in_says_why_in_claudes_words() {
        let mut rt = runtime_with(fake("signed-out", &[("FAKE_CLAUDE_LOGIN", "fail")]));
        wait(&mut rt, settled);
        tap(&mut rt, "sign-in-button");
        wait(&mut rt, |s| {
            matches!(s.status, Status::SignedOut { problem: Some(_) })
        });
        let Status::SignedOut {
            problem: Some(problem),
        } = &rt.state.status
        else {
            unreachable!()
        };
        assert!(
            problem.contains("the fake sign-in was refused"),
            "{problem}"
        );
        assert!(rt.find("sign-in-problem").is_some());
    }

    #[test]
    fn cancelling_a_sign_in_goes_back_to_signed_out() {
        let mut rt = runtime_with(fake("signed-out", &[]));
        wait(&mut rt, settled);
        tap(&mut rt, "sign-in-button");
        wait(&mut rt, |s| {
            matches!(s.status, Status::SigningIn { url: Some(_) })
        });
        tap(&mut rt, "cancel-sign-in");
        assert_eq!(rt.state.status, Status::SignedOut { problem: None });
    }

    #[test]
    fn a_message_gets_claudes_streamed_reply() {
        let mut rt = runtime_with(fake("signed-in", &[]));
        wait(&mut rt, settled);
        assert!(rt.find("health").is_some());
        rt.state.draft.text = "hello \"there\"".into();
        tap(&mut rt, "send");
        wait(&mut rt, replied);
        let last = rt.state.ws.chat.last().unwrap();
        assert_eq!(last.author, model::Author::ChiefOfStaff);
        assert_eq!(last.text, "You said: hello \"there\"");
    }

    #[test]
    fn a_claude_failure_shows_in_the_chat() {
        let mut rt = runtime_with(fake("signed-in", &[]));
        wait(&mut rt, settled);
        rt.state.draft.text = "please fail".into();
        tap(&mut rt, "send");
        wait(&mut rt, replied);
        let last = rt.state.ws.chat.last().unwrap();
        assert_eq!(last.author, model::Author::Problem);
        assert!(last.text.contains("could not finish this turn"));
    }

    #[test]
    fn a_rejected_sign_in_shows_the_expired_screen() {
        let mut rt = runtime_with(fake("expired", &[]));
        wait(&mut rt, settled);
        assert!(matches!(rt.state.status, Status::Ready(_)));
        rt.state.draft.text = "hello".into();
        tap(&mut rt, "send");
        wait(&mut rt, |s| s.status == Status::Expired);
        assert!(rt.find("sign-in-button").is_some());
        assert!(rt.find("chat-input").is_none());
    }

    #[test]
    fn signing_in_again_after_expiry_returns_to_the_chat() {
        let home = scratch("expired");
        let mut rt = runtime_with(on_fake(FAKE_CLAUDE, &home, &[]));
        wait(&mut rt, settled);
        rt.state.draft.text = "hello".into();
        tap(&mut rt, "send");
        wait(&mut rt, |s| s.status == Status::Expired);
        tap(&mut rt, "sign-in-button");
        wait(&mut rt, |s| {
            matches!(s.status, Status::SigningIn { url: Some(_) })
        });
        std::fs::write(home.join("browser-done"), "").unwrap();
        wait(&mut rt, |s| matches!(s.status, Status::Ready(_)));
        rt.state.draft.text = "hello again".into();
        tap(&mut rt, "send");
        wait(&mut rt, replied);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            "You said: hello again"
        );
    }

    #[test]
    fn an_api_key_sign_in_is_blocked_and_offers_the_subscription_sign_in() {
        let mut rt = runtime_with(fake("console", &[]));
        wait(&mut rt, settled);
        assert_eq!(
            rt.state.status,
            Status::NotSubscription {
                method: "api_key".into()
            }
        );
        assert!(rt.find("sign-in-button").is_some());
        assert!(rt.find("chat-input").is_none());
    }

    #[test]
    fn api_billing_credentials_never_reach_claude() {
        let mut rt = runtime_with(fake(
            "signed-in",
            &[
                ("ANTHROPIC_API_KEY", "sk-test"),
                ("ANTHROPIC_AUTH_TOKEN", "token-test"),
            ],
        ));
        wait(&mut rt, settled);
        rt.state.draft.text = "envcheck".into();
        tap(&mut rt, "send");
        wait(&mut rt, replied);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            "You said: api-key: token:"
        );
    }

    #[test]
    fn a_note_from_claudes_tool_lands_in_for_you_to_do() {
        let mut rt = runtime_with(fake("signed-in", &[]));
        wait(&mut rt, settled);
        rt.state.draft.text = "note: Renew the domain".into();
        tap(&mut rt, "send");
        wait(&mut rt, replied);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            "You said: I noted it for you: Renew the domain"
        );
        let note = rt.state.ws.for_you().next().expect("a note").clone();
        assert_eq!(note.title, "Renew the domain");
        assert!(rt.find(&format!("task-{}", note.id)).is_some());
        // Marking it done tells the chief of staff which note it was.
        let chat = rt.state.ws.chat.len();
        tap(&mut rt, &format!("done-{}", note.id));
        wait(&mut rt, |s| replied(s) && s.ws.chat.len() > chat);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            "You said: I have done this: Renew the domain"
        );
    }

    /// Sends "run: git push", which makes the fake ask the hook, and answers the question.
    fn answer_tool_use(choice: usize) -> Runtime<App> {
        let mut rt = runtime_with(fake_in(Mode::Ask));
        wait(&mut rt, settled);
        rt.state.draft.text = "run: git push".into();
        tap(&mut rt, "send");
        wait(&mut rt, |s| s.ws.needs_input().next().is_some());
        let item = rt.state.ws.needs_input().next().unwrap().clone();
        assert_eq!(item.title, "Allow Bash?");
        assert_eq!(item.detail, "command: git push");
        let chat = rt.state.ws.chat.len();
        tap(&mut rt, &format!("answer-{}-{choice}", item.id));
        wait(&mut rt, replied);
        // The answer went to the hook, not into the chat.
        assert_eq!(rt.state.ws.chat.len(), chat + 1);
        assert!(rt.state.ws.needs_input().next().is_none());
        rt
    }

    #[test]
    fn allowing_a_tool_use_answers_claudes_hook() {
        let rt = answer_tool_use(0);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            "You said: Bash was allowed to run git push"
        );
    }

    /// Sends `message`, which makes the fake ask the hook, and returns the question shown.
    fn ask_about(message: &str) -> (Runtime<App>, Item) {
        let mut rt = runtime_with(fake_in(Mode::Ask));
        wait(&mut rt, settled);
        rt.state.draft.text = message.into();
        tap(&mut rt, "send");
        wait(&mut rt, |s| s.ws.needs_input().next().is_some());
        let item = rt.state.ws.needs_input().next().unwrap().clone();
        (rt, item)
    }

    #[test]
    fn the_question_shows_the_whole_command_not_a_preview() {
        let command = long_command();
        assert!(command.len() > 300);
        let (mut rt, item) = ask_about(&format!("run: {command}"));
        assert_eq!(item.detail, format!("command: {command}"));
        assert!(item.detail.ends_with("; rm -rf ~"));
        assert!(rt.find(&format!("input-{}-detail", item.id)).is_some());
        tap(&mut rt, &format!("answer-{}-1", item.id));
        wait(&mut rt, replied);
    }

    #[test]
    fn dropping_the_app_stops_a_session_that_is_mid_turn() {
        let home = scratch("signed-in");
        let mut rt = runtime_with(on_fake(FAKE_CLAUDE, &home, &[]));
        wait(&mut rt, settled);
        rt.state.approvals.change(|s| s.mode = Mode::Ask);
        rt.state.draft.text = "run: git push".into();
        tap(&mut rt, "send");
        wait(&mut rt, |s| s.ws.needs_input().next().is_some());
        let pid: libc::pid_t = std::fs::read_to_string(home.join("pid"))
            .expect("the fake wrote its pid")
            .trim()
            .parse()
            .expect("a pid");
        let alive = || unsafe { libc::kill(pid, 0) == 0 };
        assert!(alive());
        drop(rt);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while alive() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(!alive(), "the session outlived the app");
    }

    #[test]
    fn dropping_the_app_signals_a_session_that_never_finishes_its_turn() {
        let home = scratch("signed-in");
        let mut rt = runtime_with(on_fake(FAKE_CLAUDE, &home, &[]));
        wait(&mut rt, settled);
        rt.state.draft.text = "stuck: forever".into();
        tap(&mut rt, "send");
        let pid_file = home.join("pid");
        let started = std::time::Instant::now();
        while !pid_file.exists() && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(25));
        }
        let pid: libc::pid_t = std::fs::read_to_string(&pid_file)
            .expect("the fake wrote its pid")
            .trim()
            .parse()
            .expect("a pid");
        let alive = || unsafe { libc::kill(pid, 0) == 0 };
        assert!(alive());
        drop(rt);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while alive() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(!alive(), "the session outlived the app");
    }

    #[test]
    fn the_question_for_a_write_shows_the_path_and_the_content() {
        let (mut rt, item) = ask_about("write: /tmp/notes.txt hello there");
        assert_eq!(item.title, "Allow Write?");
        assert!(item.detail.contains("file_path: /tmp/notes.txt"));
        assert!(item.detail.contains("content: hello there"));
        tap(&mut rt, &format!("answer-{}-0", item.id));
        wait(&mut rt, replied);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            "You said: Write was allowed to run /tmp/notes.txt"
        );
    }

    #[test]
    fn an_input_too_large_to_show_is_denied_without_asking() {
        let mut rt = runtime_with(fake_in(Mode::Ask));
        wait(&mut rt, settled);
        rt.state.draft.text = "big:".into();
        tap(&mut rt, "send");
        wait(&mut rt, replied);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            "You said: Bash was denied: too large to review"
        );
        assert!(rt.state.ws.needs_input().next().is_none());
    }

    #[test]
    fn a_question_claude_code_stopped_waiting_on_leaves_the_panel() {
        let (mut rt, item) = ask_about("abandon: ls");
        assert_eq!(item.title, "Allow Bash?");
        wait(&mut rt, |s| s.ws.needs_input().next().is_none());
        wait(&mut rt, replied);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            "You said: the hook did not answer, so Bash was blocked"
        );
        // Nothing is left to click, and no answer went anywhere.
        assert!(rt.find(&format!("answer-{}-0", item.id)).is_none());
    }

    #[test]
    fn denying_a_tool_use_answers_claudes_hook() {
        let rt = answer_tool_use(1);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            "You said: Bash was denied: git push"
        );
    }

    // Tool approval, on the fake `claude` and its real calls to the hook.

    /// Sends `message` and waits for the fake's reply, answering nothing.
    fn send_and_wait(rt: &mut Runtime<App>, message: &str) -> String {
        let chat = rt.state.ws.chat.len();
        rt.state.draft.text = message.into();
        tap(rt, "send");
        wait(rt, |s| replied(s) && s.ws.chat.len() > chat);
        rt.state.ws.chat.last().unwrap().text.clone()
    }

    fn last_entry(rt: &Runtime<App>) -> approval::Entry {
        rt.state
            .activity
            .entries()
            .last()
            .expect("a log entry")
            .clone()
    }

    #[test]
    fn a_fresh_install_runs_tools_without_asking_and_logs_them() {
        let mut rt = runtime_with(fake("signed-in", &[]));
        wait(&mut rt, settled);
        assert_eq!(rt.state.approvals.settings().mode, Mode::Run);
        let reply = send_and_wait(&mut rt, "run: git push");
        assert_eq!(reply, "You said: Bash was allowed to run git push");
        assert!(rt.state.ws.needs_input().next().is_none());
        let entry = last_entry(&rt);
        assert_eq!(
            (entry.tool.as_str(), entry.subject.as_str(), entry.allowed),
            ("Bash", "git push", true)
        );
        assert_eq!(entry.how, "allowed by the mode Run without asking");
    }

    #[test]
    fn auto_allow_read_only_runs_a_read_and_asks_about_a_command() {
        let mut rt = runtime_with(fake_in(Mode::ReadOnly));
        wait(&mut rt, settled);
        let reply = send_and_wait(&mut rt, "read: /tmp/notes.txt");
        assert_eq!(reply, "You said: Read was allowed to run /tmp/notes.txt");
        rt.state.draft.text = "run: ls".into();
        tap(&mut rt, "send");
        wait(&mut rt, |s| s.ws.needs_input().next().is_some());
        let item = rt.state.ws.needs_input().next().unwrap().clone();
        assert_eq!(
            (item.title.as_str(), item.detail.as_str()),
            ("Allow Bash?", "command: ls")
        );
        tap(&mut rt, &format!("answer-{}-1", item.id));
        wait(&mut rt, replied);
        assert_eq!(
            last_entry(&rt).how,
            "asked by the mode Auto-allow read-only tools; you denied it"
        );
    }

    #[test]
    fn ask_every_time_asks_even_about_a_read() {
        let (mut rt, item) = {
            let mut rt = runtime_with(fake_in(Mode::Ask));
            wait(&mut rt, settled);
            rt.state.draft.text = "read: /tmp/notes.txt".into();
            tap(&mut rt, "send");
            wait(&mut rt, |s| s.ws.needs_input().next().is_some());
            let item = rt.state.ws.needs_input().next().unwrap().clone();
            (rt, item)
        };
        assert_eq!(item.title, "Allow Read?");
        tap(&mut rt, &format!("answer-{}-0", item.id));
        wait(&mut rt, replied);
        assert_eq!(
            last_entry(&rt).how,
            "asked by the mode Ask every time; you allowed it"
        );
    }

    #[test]
    fn a_deny_rule_wins_over_an_allow_rule() {
        let mut rt = runtime_with(fake_in(Mode::Ask));
        wait(&mut rt, settled);
        rt.state.approvals.change(|s| s.rules = sample_rules());
        rt.state.approvals.change(|s| {
            s.rules.insert(
                0,
                approval::Rule {
                    effect: approval::Effect::Allow,
                    tool: "Bash".into(),
                    pattern: Some("git".into()),
                },
            )
        });
        let reply = send_and_wait(&mut rt, "run: git push --force origin");
        assert_eq!(reply, "You said: Bash was denied: git push --force origin");
        assert!(
            rt.state.ws.needs_input().next().is_none(),
            "nothing was asked"
        );
        assert_eq!(
            last_entry(&rt).how,
            "denied by rule 3 (Deny Bash matching git push --force)"
        );
        let reply = send_and_wait(&mut rt, "run: git status");
        assert_eq!(reply, "You said: Bash was allowed to run git status");
    }

    #[test]
    fn an_ask_rule_shows_the_whole_input_while_running_without_asking() {
        let mut rt = runtime_with(fake_in(Mode::Run));
        wait(&mut rt, settled);
        rt.state.approvals.change(|s| {
            s.rules.push(approval::Rule {
                effect: approval::Effect::Ask,
                tool: "Bash".into(),
                pattern: Some("echo".into()),
            })
        });
        let command = long_command();
        rt.state.draft.text = format!("run: {command}");
        tap(&mut rt, "send");
        wait(&mut rt, |s| s.ws.needs_input().next().is_some());
        let item = rt.state.ws.needs_input().next().unwrap().clone();
        assert_eq!(item.detail, format!("command: {command}"));
        assert!(item.detail.ends_with("; rm -rf ~"));
        tap(&mut rt, &format!("answer-{}-1", item.id));
        wait(&mut rt, replied);
        let entry = last_entry(&rt);
        assert!(!entry.allowed);
        assert_eq!(
            entry.how,
            "asked by rule 1 (Ask Bash matching echo); you denied it"
        );
        // The rest still runs without asking.
        let reply = send_and_wait(&mut rt, "run: ls");
        assert_eq!(reply, "You said: Bash was allowed to run ls");
    }

    #[test]
    fn the_settings_page_changes_the_running_session_and_survives_a_restart() {
        let home = scratch("signed-in");
        let mut rt = runtime_with(on_fake(FAKE_CLAUDE, &home, &[]));
        wait(&mut rt, settled);
        open_setting(&mut rt, "approvals");
        assert_eq!(rt.state.showing, Showing::Approvals);
        assert!(rt.find("mode-run-without-asking").is_some());
        assert!(rt.find("rules-empty").is_some());
        tap_scrolled(&mut rt, "mode-ask-every-time");
        assert_eq!(rt.state.approvals.settings().mode, Mode::Ask);
        tap_scrolled(&mut rt, "rule-effect-deny");
        rt.state.form.tool.text = "Bash".into();
        rt.state.form.pattern.text = "git push".into();
        tap_scrolled(&mut rt, "rule-save");
        assert!(rt.find("rule-0").is_some());
        assert_eq!(
            rt.state.approvals.settings().rules[0].describe(),
            "Deny Bash matching git push"
        );
        // The example follows the settings.
        assert_eq!(
            rt.state.form.example(rt.state.approvals.settings()),
            "Denied, by rule 1 (Deny Bash matching git push)."
        );
        assert!(rt.find("example-result").is_some());
        tap(&mut rt, "approvals-close");
        let reply = send_and_wait(&mut rt, "run: git push");
        assert_eq!(reply, "You said: Bash was denied: git push");
        let saved = rt.state.approvals.settings().clone();
        let log = rt.state.activity.entries().to_vec();
        drop(rt);

        let rt = runtime_with(on_fake(FAKE_CLAUDE, &home, &[]));
        assert_eq!(rt.state.approvals.settings(), &saved);
        assert_eq!(rt.state.activity.entries(), log.as_slice());
    }

    #[test]
    fn reset_asks_first_then_restores_the_recommended_default() {
        let mut rt = runtime_with(fake_in(Mode::Ask));
        rt.state.approvals.change(|s| s.rules = sample_rules());
        open_setting(&mut rt, "approvals");
        assert!(rt.find("rule-2").is_some());
        tap(&mut rt, "rule-2-up");
        assert_eq!(rt.state.approvals.settings().rules[1].tool, "Edit");
        tap(&mut rt, "rule-0-delete");
        assert_eq!(rt.state.approvals.settings().rules.len(), 2);
        tap(&mut rt, "approvals-reset");
        assert_eq!(rt.state.approvals.settings().mode, Mode::Ask, "not yet");
        tap(&mut rt, "approvals-reset-cancel");
        assert_eq!(rt.state.approvals.settings().rules.len(), 2);
        tap(&mut rt, "approvals-reset");
        tap(&mut rt, "approvals-reset-confirm");
        assert_eq!(
            rt.state.approvals.settings(),
            &approval::Settings::default()
        );
        assert!(rt.find("rules-empty").is_some());
    }

    #[test]
    fn a_corrupt_settings_file_asks_every_time_and_says_so() {
        let home = scratch("signed-in");
        std::fs::write(home.join(approval::SETTINGS_FILE), "{\"version\": 1,").unwrap();
        let mut rt = runtime_with(on_fake(FAKE_CLAUDE, &home, &[]));
        wait(&mut rt, settled);
        assert_eq!(rt.state.approvals.settings().mode, Mode::Ask);
        open_setting(&mut rt, "approvals");
        assert!(rt.find("approvals-problem").is_some());
        tap(&mut rt, "approvals-close");
        rt.state.draft.text = "run: ls".into();
        tap(&mut rt, "send");
        wait(&mut rt, |s| s.ws.needs_input().next().is_some());
        let item = rt.state.ws.needs_input().next().unwrap().id;
        tap(&mut rt, &format!("answer-{item}-1"));
        wait(&mut rt, replied);
    }

    #[test]
    fn the_activity_page_lists_tool_uses_newest_first() {
        let mut rt = runtime_with(fake_in(Mode::Run));
        wait(&mut rt, settled);
        open_setting(&mut rt, "approvals");
        tap(&mut rt, "approvals-activity");
        assert!(rt.find("activity-empty").is_some());
        tap(&mut rt, "activity-close");
        send_and_wait(&mut rt, "run: cargo test");
        send_and_wait(&mut rt, "write: /tmp/notes.txt hello");
        open_setting(&mut rt, "approvals");
        tap(&mut rt, "approvals-activity");
        assert!(rt.find("activity-0").is_some());
        assert!(rt.find("activity-1").is_some());
        assert!(rt.find("activity-empty").is_none());
        assert_eq!(rt.state.activity.entries()[1].tool, "Write");
    }

    /// One tiny message on the captain's real Claude Code and the cheapest model, to check that
    /// the installed `claude` accepts the bridge's flags and calls the tool and the hook. It uses
    /// the subscription, so it only runs on request: `cargo test real_claude -- --ignored`.
    #[test]
    #[ignore = "uses the real Claude account"]
    fn real_claude_calls_the_tool_through_the_hook() {
        let claude = Claude::new("claude").env("ANTHROPIC_MODEL", "haiku");
        let mut rt = runtime_with(App::new(
            Box::new(claude),
            installed_firstmate(),
            &scratch("unused"),
        ));
        rt.state.approvals.change(|s| s.mode = Mode::Ask);
        wait(&mut rt, settled);
        assert!(matches!(rt.state.status, Status::Ready(_)), "not signed in");
        rt.state.draft.text = "Call the app_note tool with the text \"bridge check\", then reply \
                               with only the word done."
            .into();
        tap(&mut rt, "send");
        let deadline = Instant::now() + Duration::from_secs(120);
        while !replied(&rt.state) {
            assert!(Instant::now() < deadline, "no reply");
            // Loading the deferred MCP tool and calling it are the only tool uses this needs.
            let question = rt.state.ws.needs_input().next().cloned();
            if let Some(item) = question {
                eprintln!("hook asked: {} {}", item.title, item.detail);
                let wanted = ["Allow ToolSearch?", "Allow mcp__velvet-hammer__app_note?"];
                let choice = if wanted.contains(&item.title.as_str()) {
                    0
                } else {
                    1
                };
                tap(&mut rt, &format!("answer-{}-{choice}", item.id));
            }
            std::thread::sleep(Duration::from_millis(50));
            rt.frame(1.0 / 60.0);
        }
        eprintln!("chat: {:?}", rt.state.ws.chat);
        let note = rt.state.ws.for_you().next().expect("app_note added a note");
        assert_eq!(note.title, "bridge check");
    }

    /// The real `claude` on the Haiku model, with visual replies enforced: a list asked for as
    /// plain text is shown as a view, either at once because of the standing instruction or
    /// after the stop hook sends it back. Run it with `cargo test real_claude -- --ignored`.
    #[test]
    #[ignore = "uses the real Claude account"]
    fn real_claude_shows_a_list_as_a_view() {
        let claude = Claude::new("claude").env("ANTHROPIC_MODEL", "haiku");
        let mut rt = runtime_with(App::new(
            Box::new(claude),
            installed_firstmate(),
            &scratch("unused"),
        ));
        wait(&mut rt, settled);
        assert!(matches!(rt.state.status, Status::Ready(_)), "not signed in");
        assert_eq!(*rt.state.visuals.settings(), visual::Mode::Enforce);
        rt.state.draft.text = "Name three fruits I could buy today, as a short list.".into();
        tap(&mut rt, "send");
        let viewed = |s: &App| {
            s.ws.chat.iter().any(|m| {
                m.tool
                    .as_ref()
                    .is_some_and(|t| t.name == ui::tool_name() && t.result.is_some())
            })
        };
        let deadline = Instant::now() + Duration::from_secs(180);
        while !(viewed(&rt.state) && replied(&rt.state)) {
            assert!(Instant::now() < deadline, "no view: {:?}", rt.state.ws.chat);
            std::thread::sleep(Duration::from_millis(50));
            rt.frame(1.0 / 60.0);
        }
        eprintln!("notices: {:?}", notices(&rt));
        eprintln!("chat: {:?}", rt.state.ws.chat);
    }

    /// The real `claude` on the Haiku model, told "remind me in 2 minutes to stretch": it calls
    /// `app_remind` with the "in" form, and the reminder fires two minutes later, on a clock
    /// moved forward rather than waited for. Run it with `cargo test real_claude -- --ignored`.
    #[test]
    #[ignore = "uses the real Claude account"]
    fn real_claude_sets_a_reminder_that_fires() {
        let ahead = Arc::new(Mutex::new(jiff::SignedDuration::ZERO));
        let clock: reminders::Clock = {
            let ahead = ahead.clone();
            Arc::new(move || {
                let now = jiff::Zoned::now();
                now.checked_add(*ahead.lock().unwrap()).unwrap_or(now)
            })
        };
        let claude = Claude::new("claude").env("ANTHROPIC_MODEL", "haiku");
        let mut rt = runtime_with(App::with_clock(
            Box::new(claude),
            installed_firstmate(),
            &scratch("unused"),
            clock,
        ));
        wait(&mut rt, settled);
        assert!(matches!(rt.state.status, Status::Ready(_)), "not signed in");
        rt.state.draft.text = "remind me in 2 minutes to stretch".into();
        tap(&mut rt, "send");
        // A turn may load the deferred tool and say something first; it ends with words.
        wait_for(&mut rt, Duration::from_secs(120), |s| {
            !s.reminders.list().is_empty()
                && replied(s)
                && s.ws
                    .chat
                    .last()
                    .is_some_and(|m| m.author == Author::ChiefOfStaff)
        });
        eprintln!("chat: {:?}", rt.state.ws.chat);
        let call = rt
            .state
            .ws
            .chat
            .iter()
            .filter_map(|m| m.tool.as_ref())
            .find(|t| t.name == "mcp__velvet-hammer__app_remind")
            .expect("app_remind was called")
            .input
            .clone();
        assert_eq!(
            (&call["when"], &call["amount"], &call["unit"]),
            (
                &serde_json::json!("in"),
                &serde_json::json!(2),
                &serde_json::json!("minutes")
            )
        );
        assert!(
            call["text"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("stretch")
        );
        let reminder = rt.state.reminders.list().pop().expect("a reminder");
        assert!(reminder.fired.is_none());
        *ahead.lock().unwrap() = jiff::SignedDuration::from_secs(121);
        rt.frame(1.0 / 60.0);
        assert!(rt.find("reminder-banner").is_some(), "the reminder fired");
    }

    #[test]
    fn a_missing_claude_says_so() {
        let mut rt = runtime_with(fake_program("/nonexistent/claude", "signed-out", &[]));
        wait(&mut rt, settled);
        assert_eq!(rt.state.status, Status::CliMissing);
        assert!(rt.find("recheck").is_some());
        assert!(rt.find("sign-in-button").is_none());
    }

    #[test]
    fn a_broken_claude_shows_its_own_error() {
        let mut rt = runtime_with(fake("broken", &[]));
        wait(&mut rt, settled);
        assert_eq!(
            rt.state.status,
            Status::Failed("fake claude: the settings file is corrupt".into())
        );
        assert!(rt.find("status-detail").is_some());
    }

    /// Every text on screen, without the invisible direction marks and code padding Rune's
    /// Markdown view adds around words.
    fn texts(rt: &Runtime<App>) -> Vec<String> {
        rt.elements()
            .into_iter()
            .filter_map(|e| e.text)
            .map(|t| {
                t.chars()
                    .filter(|c| !('\u{2066}'..='\u{2069}').contains(c) && *c != '\u{202f}')
                    .collect()
            })
            .collect()
    }

    /// The texts inside the element keyed `key`.
    fn texts_under(rt: &Runtime<App>, key: &str) -> Vec<String> {
        let all = rt.elements();
        let root = all.iter().position(|e| e.key.as_deref() == Some(key));
        let inside = |mut i: usize| loop {
            if Some(i) == root {
                return true;
            }
            match all[i].parent {
                Some(p) => i = p,
                None => return false,
            }
        };
        (0..all.len())
            .filter(|&i| inside(i))
            .filter_map(|i| all[i].text.clone())
            .collect()
    }

    fn chief_says(text: &str) -> Runtime<App> {
        let mut app = stub();
        app.ws
            .chat
            .push(Message::new(Author::ChiefOfStaff, text.to_string()));
        runtime_with(app)
    }

    #[test]
    fn the_screenshot_reply_shows_its_headings_bold_not_in_asterisks() {
        let rt = chief_says(SCREENSHOT_REPLY);
        let texts = texts(&rt);
        assert!(!texts.iter().any(|t| t.contains("**")), "{texts:?}");
        // Each heading keeps its own line above what it introduces.
        assert!(
            texts
                .iter()
                .any(|t| t.starts_with("Captain's Call\nNo decision is recorded")),
            "{texts:?}"
        );
        assert!(rt.find("msg-2-text").is_some());
    }

    #[test]
    fn every_construct_is_formatted_and_html_shows_as_text() {
        let rt = chief_says(EVERY_CONSTRUCT);
        let texts = texts(&rt);
        for shown in [
            "Heading one",
            "First bullet",
            "Nested bullet",
            "Second step",
            "A quoted line.",
            "<script>alert(1)</script> stays text, as does <b>this</b>.",
        ] {
            assert!(texts.iter().any(|t| t == shown), "{shown} in {texts:?}");
        }
        assert!(texts.iter().any(|t| t.contains("println!(\"hello\")")));
        assert!(
            !texts
                .iter()
                .any(|t| t.contains("**") || t.contains("~~") || t.contains("# "))
        );
    }

    #[test]
    fn a_reply_still_arriving_never_shows_its_markers() {
        let whole = "The fleet is **empty**, *quiet* and ~~busy~~ `idle` now.";
        let mut rt = runtime();
        let mut end = 0;
        while end < whole.len() {
            end += 1;
            let text = whole[..end].to_string();
            rt.update(|s, _| s.take(Event::Reply { text, done: false }));
            rt.frame(1.0 / 60.0);
            let i = rt.state.replying.expect("still replying");
            let shown = texts_under(&rt, &format!("msg-{i}-text"));
            assert!(!shown.is_empty());
            assert!(
                !shown
                    .iter()
                    .any(|t| t.contains('*') || t.contains("~~") || t.contains('`')),
                "{:?} showed {shown:?}",
                &whole[..end]
            );
        }
    }

    #[test]
    fn a_link_opens_in_the_browser_only_when_pressed_and_only_on_the_web() {
        let mut rt =
            chief_says("Read [the docs](https://example.com/docs) or [this](file:///etc/hosts).");
        for _ in 0..10 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.take_url_requests().is_empty(), "nothing opens by itself");
        tap(&mut rt, "https://example.com/docs");
        assert_eq!(rt.take_url_requests(), ["https://example.com/docs"]);
        tap(&mut rt, "file:///etc/hosts");
        assert!(rt.take_url_requests().is_empty());
    }

    #[test]
    fn an_image_a_tool_returned_shows_inline_and_enlarges() {
        let mut rt = runtime_with(shown(&fixture("chart.png")));
        assert!(rt.find("tool-2-placeholder").is_none());
        tap(&mut rt, "tool-2-image");
        assert_eq!(rt.state.showing, Showing::Picture("toolu_fake_Read".into()));
        assert!(rt.find("picture").is_some());
        tap(&mut rt, "picture-close");
        assert_eq!(rt.state.showing, Showing::Chat);
    }

    #[test]
    fn an_image_that_cannot_be_decoded_says_so_with_its_path() {
        let path = fixture("broken.png");
        let rt = runtime_with(shown(&path));
        assert!(rt.find("tool-2-image").is_none());
        let placeholder = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("tool-2-placeholder"))
            .and_then(|e| e.text)
            .unwrap();
        assert!(placeholder.contains(&path) && placeholder.contains("could not be decoded"));
    }

    #[test]
    fn an_image_file_too_large_to_read_is_not_read() {
        let dir = scratch("unused");
        let big = dir.join("big.png");
        let file = std::fs::File::create(&big).unwrap();
        file.set_len(tools::MAX_MEDIA_BYTES + 1).unwrap();
        let mut app = stub();
        app.take(Event::Tool(tools::ToolUse {
            id: "t1".into(),
            name: "Write".into(),
            input: serde_json::json!({ "file_path": big }),
            result: None,
        }));
        app.take(Event::ToolResult {
            id: "t1".into(),
            result: tools::ToolResult {
                is_error: false,
                content: vec![tools::Content::Text("written".into())],
            },
        });
        let image = app.media["t1"].image.as_ref().unwrap();
        assert!(image.as_ref().unwrap_err().contains("larger than 32 MB"));
    }

    #[test]
    fn a_video_file_shows_a_player_and_an_open_action() {
        let mut rt = runtime_with(shown(&fixture("clip.mp4")));
        for key in ["tool-2-video", "tool-2-play", "tool-2-open"] {
            assert!(rt.find(key).is_some(), "no {key}");
        }
        tap(&mut rt, "tool-2-play");
        let player = rt.state.media["toolu_fake_Read"].video.clone().unwrap();
        if cfg!(any(target_os = "macos", target_os = "windows")) {
            assert!(player.is_playing());
        } else {
            // Rune has no player on this platform; the system's player is one press away.
            assert!(rt.find("tool-2-problem").is_some());
        }
    }

    #[test]
    fn any_other_tool_shows_compactly_with_its_result_folded() {
        let mut app = fake_in(Mode::Run);
        say(&mut app, "shell: ls");
        let mut rt = runtime_with(app);
        assert!(texts(&rt).iter().any(|t| t == "Bash"));
        assert!(rt.find("tool-2-result").is_none());
        tap(&mut rt, "tool-2-toggle");
        assert!(texts(&rt).iter().any(|t| t == "one\ntwo\nthree"));
        tap(&mut rt, "tool-2-toggle");
        assert!(rt.find("tool-2-result").is_none());
    }

    fn view_runtime(input: serde_json::Value) -> Runtime<App> {
        let mut rt = runtime_with(viewing(input, true));
        for _ in 0..5 {
            rt.frame(1.0 / 60.0);
        }
        rt
    }

    #[test]
    fn a_static_view_draws_each_component_in_the_chat() {
        let rt = view_runtime(spec("ui-release.json"));
        for key in [
            "tool-1-ui-review",
            "tool-1-ui-blocker",
            "tool-1-ui-progress",
            "tool-1-ui-tasks",
            "tool-1-ui-alert",
            "tool-1-ui-tag",
        ] {
            assert!(rt.find(key).is_some(), "no {key}");
        }
        let texts = texts(&rt);
        for shown in [
            "Release 0.4",
            "Chief of staff's view",
            "in review",
            "Tag it anyway",
        ] {
            assert!(texts.iter().any(|t| t == shown), "{shown} in {texts:?}");
        }
    }

    #[test]
    fn a_streamed_view_fills_in_as_its_input_arrives() {
        let whole = fixture_text("ui-release.json");
        let mut rt = runtime_with(viewing(serde_json::json!({}), false));
        rt.frame(1.0 / 60.0);
        assert!(rt.find("tool-1-waiting").is_some());
        let mut seen = 0;
        for (end, _) in whole.char_indices().step_by(7) {
            let input = ui::partial(&whole[..end]);
            rt.update(|s, _| {
                s.take(Event::ToolInput {
                    id: VIEW.into(),
                    input,
                })
            });
            rt.frame(1.0 / 60.0);
            let shown = rt
                .elements()
                .iter()
                .filter(|e| {
                    e.key
                        .as_deref()
                        .is_some_and(|k| k.starts_with("tool-1-ui-"))
                })
                .count();
            assert!(shown >= seen.min(1), "the view vanished at {end}");
            seen = seen.max(shown);
        }
        assert!(seen > 0);
        rt.update(|s, _| {
            s.take(Event::Tool(tools::ToolUse {
                id: VIEW.into(),
                name: ui::tool_name(),
                input: spec("ui-release.json"),
                result: None,
            }))
        });
        rt.frame(1.0 / 60.0);
        assert!(rt.find("tool-1-ui-tag").is_some());
        assert_eq!(
            rt.state.ws.chat.len(),
            2,
            "the whole input replaced the streamed one"
        );
    }

    #[test]
    fn inputs_and_checkboxes_bind_state_and_a_repeat_follows_it() {
        let mut rt = view_runtime(spec("ui-todos.json"));
        assert!(rt.find("tool-1-ui-done.1").is_none());
        tap(&mut rt, "tool-1-ui-check.1");
        assert!(
            rt.find("tool-1-ui-done.1").is_some(),
            "ticking showed its badge"
        );
        rt.update(|s, _| s.view_edit(VIEW, "/draft", "").text = "Update the README".into());
        rt.frame(1.0 / 60.0);
        tap(&mut rt, "tool-1-ui-add");
        assert!(texts(&rt).iter().any(|t| t == "Update the README"));
        assert!(rt.find("tool-1-ui-check.3").is_some());
        tap(&mut rt, "tool-1-ui-remove.0");
        assert!(rt.find("tool-1-ui-check.3").is_none());
        assert!(!texts(&rt).iter().any(|t| t == "Rebase onto main"));
        // Nothing went to the chief of staff: these actions stay in the view.
        assert_eq!(rt.state.ws.chat.len(), 2);
    }

    #[test]
    fn a_submit_goes_to_the_chief_of_staff_and_shows_what_the_captain_did() {
        let mut rt = view_runtime(spec("ui-todos.json"));
        tap(&mut rt, "tool-1-ui-check.2");
        rt.update(|s, _| s.view_edit(VIEW, "/draft", "").text = "half typed".into());
        tap(&mut rt, "tool-1-ui-send");
        let chat = &rt.state.ws.chat;
        assert_eq!(chat[2].author, Author::Captain);
        assert_eq!(chat[2].text, "Pressed \u{201c}Send the list\u{201d}.");
        assert!(
            chat[3]
                .text
                .starts_with("Thanks. I got your submit with {\"list\":")
        );
        assert!(chat[3].text.contains("\"title\":\"Tag the release\""));
    }

    #[test]
    fn a_submit_reaches_claude_codes_session_with_the_input_values() {
        let mut app = fake_in(Mode::Run);
        app.take(Event::Tool(tools::ToolUse {
            id: VIEW.into(),
            name: ui::tool_name(),
            input: spec("ui-todos.json"),
            result: None,
        }));
        let mut rt = runtime_with(app);
        let i = rt.state.ws.chat.len() - 1;
        rt.update(|s, _| s.view_edit(VIEW, "/draft", "").text = "Ship".into());
        tap(&mut rt, &format!("tool-{i}-ui-send"));
        wait(&mut rt, replied);
        let reply = &rt.state.ws.chat.last().unwrap().text;
        assert!(
            reply.contains("You said: In the view you showed with app_ui (tool use toolu_view)")
        );
        assert!(reply.contains("pressed “Send the list”"), "{reply}");
        assert!(
            reply.contains(r#"input values: {"/draft":"Ship","/todos/0/done":true"#),
            "{reply}"
        );
    }

    #[test]
    fn an_unknown_component_shows_its_type_and_the_rest_still_draws() {
        let rt = view_runtime(spec("ui-unknown.json"));
        let texts = texts(&rt);
        assert!(
            texts
                .iter()
                .any(|t| t == "Unknown component \u{201c}BarChart\u{201d}")
        );
        assert!(texts.iter().any(|t| t == "Monday was the slowest."));
    }

    #[test]
    fn a_view_over_the_limits_is_refused_with_why() {
        let rt = view_runtime(too_large());
        assert!(rt.find("tool-1-ui-main").is_none());
        let texts = texts(&rt);
        assert!(
            texts
                .iter()
                .any(|t| t.contains("more than the limit of 200")),
            "{texts:?}"
        );
    }

    #[test]
    fn a_view_shows_only_images_the_session_already_named() {
        let chart = fixture("chart.png");
        let input = |src: &str| {
            serde_json::json!({"root": "i", "elements": {"i": {"type": "Image",
                "props": {"src": src, "alt": "Build times"}}}})
        };
        let mut app = viewing(input(&chart), true);
        assert!(
            app.ui_images[&chart]
                .as_ref()
                .unwrap_err()
                .contains("already named")
        );
        // A view cannot name its own file: `path` on the spec is not an approval.
        let mut selfnamed = input(&chart);
        selfnamed["path"] = serde_json::json!(chart);
        let selfnamed = viewing(selfnamed, true);
        assert!(selfnamed.ui_images[&chart].is_err());
        let url = viewing(input("https://example.com/x.png"), true);
        assert!(url.ui_images["https://example.com/x.png"].is_err());
        // Once a tool use names the file, a view can show it.
        app.take(Event::Tool(tools::ToolUse {
            id: "t-read".into(),
            name: "Read".into(),
            input: serde_json::json!({"file_path": chart}),
            result: None,
        }));
        app.take(Event::ToolInput {
            id: VIEW.into(),
            input: input(&chart),
        });
        let image = app.ui_images[&chart].as_ref().unwrap();
        let _ = image.wait();
        assert!(image.error().is_none());
    }

    // Visual replies, on the fake `claude` and its real calls to the prompt and stop hooks.

    /// Sends `message` and waits until the chief of staff's last words are `end`.
    fn send_until(rt: &mut Runtime<App>, message: &str, end: &str) {
        rt.state.draft.text = message.into();
        tap(rt, "send");
        wait(rt, |s| {
            s.replying.is_none() && s.ws.chat.last().is_some_and(|m| m.text == end)
        });
    }

    /// Scrolls the element keyed `key` to the middle of whatever scrolls it, then taps it.
    fn tap_scrolled(rt: &mut Runtime<App>, key: &str) {
        let centered = key.to_string();
        rt.update(move |_, tx| tx.scroll_center(centered));
        tap(rt, key);
    }

    fn notices(rt: &Runtime<App>) -> Vec<String> {
        rt.state
            .ws
            .chat
            .iter()
            .filter(|m| m.author == Author::App)
            .map(|m| m.text.clone())
            .collect()
    }

    #[test]
    fn a_list_sent_as_text_is_sent_back_once_and_shown_as_a_view() {
        let mut rt = runtime_with(fake_in(Mode::Run));
        assert_eq!(*rt.state.visuals.settings(), visual::Mode::Enforce);
        send_until(&mut rt, LIST, "Shown above.");
        assert_eq!(
            notices(&rt),
            [
                "Velvet Hammer asked the chief of staff to show this as a view: the reply lists 3 \
              items."
            ]
        );
        let chat = &rt.state.ws.chat;
        let at = |want: &dyn Fn(&Message) -> bool| chat.iter().position(want).unwrap();
        let text = at(&|m| m.text.starts_with("Open items:"));
        let notice = at(&|m| m.author == Author::App);
        let view = at(&|m| m.tool.as_ref().is_some_and(|t| t.name == ui::tool_name()));
        assert!(text < notice && notice < view, "{text} {notice} {view}");
        assert!(texts(&rt).iter().any(|t| t == "Renew the domain"));
        assert!(rt.find(&format!("msg-{notice}")).is_some());
    }

    #[test]
    fn a_reply_that_ignores_the_feedback_is_not_sent_back_again() {
        let mut rt = runtime_with(fake_in(Mode::Run));
        send_until(&mut rt, "stubborn: a, b, c", "Left as text.");
        assert_eq!(notices(&rt).len(), 1);
        // The next reply is checked again.
        send_until(&mut rt, "stubborn: d, e, f", "Left as text.");
        assert_eq!(notices(&rt).len(), 2);
    }

    #[test]
    fn the_instruction_and_the_check_follow_the_visuals_page() {
        let mut rt = runtime_with(fake_in(Mode::Run));
        assert_eq!(
            send_and_wait(&mut rt, "context?"),
            "You said: context: the app asked for views"
        );
        open_setting(&mut rt, "visuals");
        assert_eq!(rt.state.showing, Showing::Visuals);
        tap(&mut rt, "visual-mode-off");
        assert_eq!(*rt.state.visuals.settings(), visual::Mode::Off);
        tap(&mut rt, "visuals-close");
        assert_eq!(
            send_and_wait(&mut rt, "context?"),
            "You said: context: none"
        );
        let reply = send_and_wait(&mut rt, LIST);
        assert!(reply.starts_with("Open items:"), "{reply}");
        assert!(notices(&rt).is_empty());
        // Nudge only adds the instruction and checks nothing.
        rt.state.visuals.change(|m| *m = visual::Mode::Nudge);
        assert_eq!(
            send_and_wait(&mut rt, "context?"),
            "You said: context: the app asked for views"
        );
        send_and_wait(&mut rt, LIST);
        assert!(notices(&rt).is_empty());
    }

    #[test]
    fn the_visuals_page_tries_a_reply_and_keeps_its_mode() {
        let mut rt = runtime();
        open_setting(&mut rt, "visuals");
        assert!(
            texts(&rt)
                .iter()
                .any(|t| t == "\u{25cf} Enforce (Recommended)")
        );
        assert!(rt.find("visual-instruction").is_some());
        assert!(rt.find("visual-shape-4").is_some());
        assert_eq!(
            texts_under(&rt, "visual-example-result"),
            ["Sent back once to be shown as a view: the reply lists 3 items."]
        );
        rt.state.visual_example.text = "Done. The build is green.".into();
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        assert_eq!(
            texts_under(&rt, "visual-example-result"),
            ["Stays plain text."]
        );
        tap(&mut rt, "visual-mode-nudge");
        let dir = rt.state.visuals.path().parent().unwrap().to_path_buf();
        assert_eq!(
            *store::Store::<visual::Mode>::open(&dir).settings(),
            visual::Mode::Nudge
        );
        tap(&mut rt, "visuals-all-settings");
        assert_eq!(texts_under(&rt, "visuals-summary"), ["Nudge only"]);
    }

    // Firstmate's rules, on the fake `claude` running as firstmate.

    #[test]
    fn firstmate_rules_deny_a_force_push_and_ask_about_a_merge_by_name() {
        let lab = Lab::new();
        let mut rt = runtime_with(fake_in_firstmate(&lab, true));
        assert_eq!(rt.state.approvals.settings().mode, Mode::Run);
        assert_eq!(
            send_and_wait(&mut rt, "run: git push --force origin main"),
            "You said: Bash was denied: git push --force origin main"
        );
        let entry = last_entry(&rt);
        assert_eq!(entry.rule.as_deref(), Some("no-force-push"));
        assert_eq!(
            entry.how,
            "denied by firstmate rule \u{201c}No force pushes\u{201d}"
        );
        // A merge asks even while tools run without asking.
        rt.state.draft.text = format!("run: {MERGE}");
        tap(&mut rt, "send");
        wait(&mut rt, |s| s.ws.needs_input().next().is_some());
        let item = rt.state.ws.needs_input().next().unwrap().clone();
        assert_eq!(item.title, "Merge https://github.com/captain/site/pull/7?");
        assert!(item.detail.starts_with("Firstmate's hard rule 2"));
        assert!(item.detail.ends_with(&format!("command: {MERGE}")));
        tap(&mut rt, &format!("answer-{}-0", item.id));
        wait(&mut rt, replied);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            format!("You said: Bash was allowed to run {MERGE}")
        );
        // The page shows what each rule did.
        open_setting(&mut rt, "rules");
        assert_eq!(rt.state.showing, Showing::Rules);
        assert!(rt.find("guard-no-force-push-recent-0").is_some());
        assert!(rt.find("guard-merge-asks-captain-recent-0").is_some());
        assert!(rt.find("guard-publish-asks-captain-quiet").is_some());
        // Turned off, the forced push is no longer denied, and still asks as a push.
        tap_scrolled(&mut rt, "guard-no-force-push-toggle");
        assert!(!rt.state.rules.settings().on(guard::Rule::NoForcePush));
        assert!(
            !store::Store::<guard::Settings>::open(&lab.data)
                .settings()
                .on(guard::Rule::NoForcePush)
        );
        tap(&mut rt, "rules-close");
        rt.state.draft.text = "run: git push --force origin main".into();
        tap(&mut rt, "send");
        wait(&mut rt, |s| s.ws.needs_input().next().is_some());
        let item = rt.state.ws.needs_input().next().unwrap().clone();
        assert_eq!(item.title, "Push to origin from /?");
        tap(&mut rt, &format!("answer-{}-1", item.id));
        wait(&mut rt, replied);
        assert_eq!(
            last_entry(&rt).how,
            "asked by firstmate rule \u{201c}Publishing waits for your word\u{201d}; you denied it"
        );
    }

    #[test]
    fn a_write_into_a_project_asks_and_names_the_file() {
        let lab = Lab::new();
        let mut rt = runtime_with(fake_in_firstmate(&lab, true));
        let home = rt.state.firstmate.home_dir();
        let file = home.join("projects/site/README.md");
        rt.state.draft.text = format!("write: {} Hello", file.display());
        tap(&mut rt, "send");

        wait(&mut rt, |s| s.ws.needs_input().next().is_some());
        let item = rt.state.ws.needs_input().next().unwrap().clone();
        assert_eq!(
            item.title,
            format!("Let the chief of staff edit {}?", file.display())
        );
        tap_scrolled(&mut rt, &format!("answer-{}-1", item.id));
        wait(&mut rt, replied);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            format!("You said: Write was denied: {}", file.display())
        );
        // Firstmate's own records are not a project.
        let notes = home.join("data/notes.md");
        assert_eq!(
            send_and_wait(&mut rt, &format!("write: {} Hello", notes.display())),
            format!("You said: Write was allowed to run {}", notes.display())
        );
    }

    #[test]
    fn a_chief_of_staff_that_is_not_firstmate_has_no_firstmate_rules() {
        let mut rt = runtime_with(fake_in(Mode::Run));
        assert_eq!(
            send_and_wait(&mut rt, "run: git push --force"),
            "You said: Bash was allowed to run git push --force"
        );
        assert!(last_entry(&rt).rule.is_none());
    }

    /// The fake `claude`, signed in, with the app's reminders on a clock set to `now`.
    fn fake_on_clock(now: jiff::Zoned) -> (Runtime<App>, Hands) {
        let home = scratch("signed-in");
        let hands = Arc::new(Mutex::new(now));
        let claude = Claude::new(FAKE_CLAUDE).env("FAKE_CLAUDE_HOME", home.to_string_lossy());
        let app = App::with_clock(
            Box::new(claude),
            installed_firstmate(),
            &home,
            clock(&hands),
        );
        let mut rt = runtime_with(app);
        wait(&mut rt, settled);
        (rt, hands)
    }

    #[test]
    fn a_reminder_set_through_claudes_tool_comes_due_with_a_banner() {
        let (mut rt, hands) = fake_on_clock(friday(14, 30));
        let reply = send_and_wait(&mut rt, "remind: in 2 minutes | Stretch");
        assert_eq!(reply, "Reminder set for Today, 2:32 PM: Stretch");
        assert_eq!(texts_under(&rt, "reminders-summary"), ["1 coming up"]);
        assert!(rt.find("reminder-banner").is_none());
        // It links back to the captain's message it was set in.
        let asked = rt
            .state
            .ws
            .chat
            .iter()
            .find(|m| m.text == "remind: in 2 minutes | Stretch")
            .expect("the captain's message")
            .id
            .clone();
        assert_eq!(rt.state.reminders.list()[0].message.as_ref(), Some(&asked));
        let reply = send_and_wait(&mut rt, "reminders?");
        assert_eq!(
            reply,
            format!(
                "It is now Today, 2:30 PM.\n- Today, 2:32 PM: Stretch (set Today, 2:30 PM in {})",
                links::url(&asked)
            )
        );
        *hands.lock().unwrap() = friday(14, 32);
        rt.frame(1.0 / 60.0);
        assert_eq!(texts_under(&rt, "reminder-banner-text"), ["Stretch"]);
        assert_eq!(
            texts_under(&rt, "reminder-banner-when"),
            ["Set Today, 2:30 PM \u{b7} due Today, 2:32 PM"]
        );
        assert!(rt.find("reminder-1").is_some(), "in For you to do");
        // From the Reminders page, the link goes back to the message and highlights it.
        tap(&mut rt, "reminders");
        tap(&mut rt, "list-1-message");
        assert_eq!(rt.state.showing, Showing::Chat);
        assert!(matches!(&rt.state.flash, Some((Ok(id), _)) if *id == asked));
        tap(&mut rt, "banner-1-done");
        assert!(rt.find("reminder-banner").is_none());
        assert!(rt.find("reminder-1").is_none());
        assert!(rt.state.reminders.list().is_empty());
    }

    #[test]
    fn the_reminder_settings_are_a_section_of_settings() {
        let (app, _) = on_clock(&scratch("unused"), friday(14, 30));
        let mut rt = runtime_with(app);
        chord(&mut rt, ',', false);
        assert_eq!(rt.state.showing, Showing::Settings);
        assert_eq!(
            texts_under(&rt, "reminder-settings-summary"),
            ["later today by 6:00 PM \u{b7} mornings 9:00 AM"]
        );
        tap(&mut rt, "reminder-settings");
        assert_eq!(rt.state.showing, Showing::ReminderSettings);
        tap(&mut rt, "reminder-setting-morning-up");
        tap(&mut rt, "reminder-settings-all-settings");
        assert_eq!(rt.state.showing, Showing::Settings);
        assert_eq!(
            texts_under(&rt, "reminder-settings-summary"),
            ["later today by 6:00 PM \u{b7} mornings 9:30 AM"]
        );
    }

    #[test]
    fn the_tool_refuses_what_it_cannot_set_and_says_why() {
        let (mut rt, _) = fake_on_clock(friday(14, 30));
        let reply = send_and_wait(&mut rt, "remind: soon | Stretch");
        assert!(reply.starts_with("when must be one of"), "{reply}");
        let reply = send_and_wait(&mut rt, "remind: in 0 minutes | Stretch");
        assert!(reply.contains("1 or more"), "{reply}");
        assert!(rt.state.reminders.list().is_empty());
    }

    #[test]
    fn a_reminder_missed_while_closed_shows_as_overdue_at_launch() {
        let data = scratch("unused");
        let (app, _) = on_clock(&data, friday(9, 0));
        app.reminders
            .set("Call the bank", When::In(1, Unit::Hours))
            .unwrap();
        drop(app);
        let (app, _) = on_clock(&data, friday(14, 30));
        let rt = runtime_with(app);
        assert_eq!(texts_under(&rt, "reminder-banner-text"), ["Call the bank"]);
        assert!(texts(&rt).contains(&"Reminder \u{b7} overdue".to_string()));
        assert!(
            texts_under(&rt, "reminder-1-when")[0].starts_with("Reminder \u{b7} overdue \u{b7} ")
        );
    }

    #[test]
    fn snoozing_from_the_banner_sets_the_reminder_again() {
        let (app, stretch) = reminders_due();
        let mut rt = runtime_with(app);
        tap(&mut rt, &format!("banner-{stretch}-snooze"));
        assert!(rt.find(&format!("banner-{stretch}-snooze-menu")).is_some());
        tap(&mut rt, &format!("banner-{stretch}-snooze-10m"));
        assert_eq!(rt.state.snoozing, None);
        let r = rt.state.reminders.list();
        let r = r.iter().find(|r| r.id == stretch).unwrap();
        assert_eq!(r.fired, None);
        assert_eq!(r.due_words(&friday(14, 32)), "Today, 2:42 PM");
        // The overdue one is still due, so the banner shows it.
        assert_eq!(
            texts_under(&rt, "reminder-banner-text"),
            ["Call the bank about the wire"]
        );
        // A typed number of hours, and a typed word that is not a number.
        let bank = rt.state.reminders.list()[0].id;
        tap(&mut rt, &format!("panel-{bank}-snooze"));
        rt.state.snooze_amount.text = "soon".into();
        tap(&mut rt, &format!("panel-{bank}-snooze-hours"));
        assert!(rt.find("snooze-problem").is_some());
        rt.state.snooze_amount.text = "2".into();
        tap(&mut rt, &format!("panel-{bank}-snooze-hours"));
        let r = rt.state.reminders.list();
        let r = r.iter().find(|r| r.id == bank).unwrap();
        assert_eq!(r.due_words(&friday(14, 32)), "Today, 4:32 PM");
        assert!(rt.find("reminder-banner").is_none());
    }

    #[test]
    fn the_reminders_page_groups_them_and_deletes_one() {
        let (app, _) = reminders_set();
        let mut rt = runtime_with(app);
        tap(&mut rt, "reminders-close");
        tap(&mut rt, "for-you-reminders");
        assert_eq!(rt.state.showing, Showing::Reminders);
        assert!(rt.find("reminders-coming").is_some());
        assert!(rt.find("reminders-later").is_some());
        assert!(rt.find("reminders-due").is_none());
        assert_eq!(texts_under(&rt, "reminders-3-due"), ["Tomorrow, 9:00 AM"]);
        tap(&mut rt, "list-1-delete");
        assert!(rt.find("reminders-1").is_none());
        assert_eq!(rt.state.reminders.list().len(), 6);
        assert_eq!(
            texts_under(&rt, "reminders-summary"),
            ["4 coming up \u{b7} 2 later"]
        );
    }

    #[test]
    fn the_settings_page_changes_what_later_today_means_and_survives_a_restart() {
        let data = scratch("unused");
        let (app, _) = on_clock(&data, friday(14, 30));
        let mut rt = runtime_with(app);
        tap(&mut rt, "reminders");
        tap(&mut rt, "reminders-settings");
        assert_eq!(
            texts_under(&rt, "reminder-example-later-today"),
            ["\u{201c}Remind me later today\u{201d} \u{2192} Today, 5:30 PM"]
        );
        tap(&mut rt, "reminder-setting-later-today-hours-down");
        assert_eq!(
            texts_under(&rt, "reminder-setting-later-today-hours-value"),
            ["2 hours"]
        );
        assert_eq!(
            texts_under(&rt, "reminder-example-later-today"),
            ["\u{201c}Remind me later today\u{201d} \u{2192} Today, 4:30 PM"]
        );
        rt.state
            .reminders
            .set("Reply to the review", When::LaterToday)
            .unwrap();
        drop(rt);
        let (app, _) = on_clock(&data, friday(14, 30));
        assert_eq!(app.reminder_settings.settings().later_today_hours, 2);
        assert_eq!(
            app.reminders.list()[0].due_words(&friday(14, 30)),
            "Today, 4:30 PM"
        );
    }

    #[test]
    fn the_rules_page_lists_every_rule_on_by_default() {
        let mut rt = runtime();
        chord(&mut rt, ',', false);
        assert_eq!(texts_under(&rt, "rules-summary"), ["7 of 7 on"]);
        tap(&mut rt, "rules");
        for rule in guard::Rule::ALL {
            assert!(
                rt.find(&format!("guard-{}", rule.key())).is_some(),
                "{rule:?}"
            );
            assert!(rt.find(&format!("guard-{}-toggle", rule.key())).is_some());
        }
        tap_scrolled(&mut rt, "guard-publish-asks-captain-toggle");
        tap(&mut rt, "rules-all-settings");
        assert_eq!(texts_under(&rt, "rules-summary"), ["6 of 7 on"]);
        tap(&mut rt, "rules");
        tap(&mut rt, "rules-activity");
        assert_eq!(rt.state.showing, Showing::Activity);
    }

    /// Opens Settings with Cmd+, (Ctrl+, off macOS), then the section keyed `key`.
    fn open_setting(rt: &mut Runtime<App>, key: &str) {
        chord(rt, ',', false);
        assert_eq!(rt.state.showing, Showing::Settings);
        tap(rt, key);
    }

    /// Presses `key` with Cmd (Ctrl off macOS) held, and Shift when `shift`.
    fn chord(rt: &mut Runtime<App>, c: char, shift: bool) {
        let mac = cfg!(target_os = "macos");
        let mods = rune::Mods {
            cmd: true,
            shift,
            ctrl: !mac,
            logo: mac,
            ..rune::Mods::default()
        };
        rt.key(KeyEvent {
            key: Key::Command(c),
            mods,
        });
        settle(rt);
    }

    fn press_key(rt: &mut Runtime<App>, key: Key, shift: bool) {
        let mods = rune::Mods {
            shift,
            ..rune::Mods::default()
        };
        rt.key(KeyEvent { key, mods });
        settle(rt);
    }

    fn type_text(rt: &mut Runtime<App>, words: &str) {
        for c in words.chars() {
            rt.key(KeyEvent {
                key: Key::Text(c.to_string()),
                mods: rune::Mods::default(),
            });
        }
        settle(rt);
    }

    fn settle(rt: &mut Runtime<App>) {
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
    }

    fn find_count(rt: &Runtime<App>) -> Vec<String> {
        texts_under(rt, "find-count")
    }

    /// Whether the element keyed `key` is drawn inside the window.
    fn on_screen(rt: &Runtime<App>, key: &str) -> bool {
        rt.find(key).is_some_and(|(_, y)| (0.0..720.0).contains(&y))
    }

    #[test]
    fn cmd_f_opens_find_over_the_conversation_and_counts_matches() {
        let mut rt = runtime_with(long_chat(40));
        tap(&mut rt, "chat-input");
        assert!(rt.find("find-bar").is_none());
        chord(&mut rt, 'f', false);
        assert!(rt.find("find-bar").is_some());
        // The cursor is in the find field, not the message field.
        type_text(&mut rt, "RELEASE");
        assert_eq!(rt.state.find.query.text, "RELEASE");
        assert!(rt.state.draft.text.is_empty());
        assert_eq!(find_count(&rt), ["1 of 4"]);
        assert!(rt.find(&find::match_key(0)).is_some());
        assert!(on_screen(&rt, &find::match_key(0)));
    }

    #[test]
    fn a_query_with_no_matches_says_so() {
        let mut rt = runtime_with(long_chat(5));
        chord(&mut rt, 'f', false);
        type_text(&mut rt, "zebra");
        assert_eq!(find_count(&rt), ["No matches"]);
        assert!(rt.find(&find::match_key(0)).is_none());
        // Stepping with nothing to step to changes nothing.
        press_key(&mut rt, Key::Enter, false);
        assert_eq!(rt.state.find.current, 0);
    }

    #[test]
    fn enter_and_cmd_g_step_through_the_matches_and_scroll_to_each() {
        let mut rt = runtime_with(long_chat(40));
        chord(&mut rt, 'f', false);
        type_text(&mut rt, "release");
        assert_eq!(find_count(&rt), ["1 of 4"]);
        press_key(&mut rt, Key::Enter, false);
        assert_eq!(find_count(&rt), ["2 of 4"]);
        assert!(on_screen(&rt, &find::match_key(1)));
        chord(&mut rt, 'g', false);
        chord(&mut rt, 'g', false);
        assert_eq!(find_count(&rt), ["4 of 4"]);
        assert!(on_screen(&rt, &find::match_key(3)));
        assert!(!on_screen(&rt, &find::match_key(0)));
        // Past the last it wraps to the first, and back.
        press_key(&mut rt, Key::Enter, false);
        assert_eq!(find_count(&rt), ["1 of 4"]);
        assert!(on_screen(&rt, &find::match_key(0)));
        press_key(&mut rt, Key::Enter, true);
        assert_eq!(find_count(&rt), ["4 of 4"]);
        chord(&mut rt, 'g', true);
        assert_eq!(find_count(&rt), ["3 of 4"]);
        tap(&mut rt, "find-previous");
        assert_eq!(find_count(&rt), ["2 of 4"]);
        tap(&mut rt, "find-next");
        assert_eq!(find_count(&rt), ["3 of 4"]);
    }

    #[test]
    fn escape_closes_find_and_returns_to_the_message_field() {
        let mut rt = runtime_with(long_chat(5));
        chord(&mut rt, 'f', false);
        type_text(&mut rt, "release");
        press_key(&mut rt, Key::Escape, false);
        assert!(rt.find("find-bar").is_none());
        assert!(rt.find(&find::match_key(0)).is_none());
        type_text(&mut rt, "hi");
        assert_eq!(rt.state.draft.text, "hi");
        // Opening it again keeps the query.
        chord(&mut rt, 'f', false);
        assert_eq!(find_count(&rt), ["1 of 1"]);
    }

    #[test]
    fn find_is_case_insensitive_and_searches_captain_messages() {
        let mut rt = runtime();
        chord(&mut rt, 'f', false);
        type_text(&mut rt, "desktop SKELETON");
        assert_eq!(find_count(&rt), ["1 of 1"]);
        assert_eq!(
            find::search(&rt.state, "desktop SKELETON")[0].message,
            0,
            "the captain's first message"
        );
    }

    #[test]
    fn find_searches_tool_uses_and_the_words_in_views() {
        let mut rt = runtime_with(viewing(spec("ui-release.json"), true));
        chord(&mut rt, 'f', false);
        let hits = find::search(&rt.state, "release");
        assert!(hits.iter().any(|h| h.card), "the view's card");
        type_text(&mut rt, "release");
        assert!(rt.find("find-count").is_some());
        assert_ne!(find_count(&rt), ["No matches"]);

        let mut rt = runtime_with(fake_in(Mode::Run));
        send_and_wait(&mut rt, "shell: ls");
        let hits = find::search(&rt.state, "three");
        assert_eq!(hits.len(), 1, "the command's output");
        assert!(hits[0].card);
    }

    #[test]
    fn the_conversation_is_kept_across_a_restart_and_find_searches_it() {
        let data = scratch("unused");
        let mut first = App::new(Box::new(Stub::default()), installed_firstmate(), &data);
        first.draft.text = "Remember the zeppelin plan.".into();
        first.send();
        let said = first.ws.chat[first.ws.chat.len() - 2].clone();
        let replied = first.ws.chat.last().unwrap().clone();
        drop(first);

        let mut rt = runtime_with(App::new(
            Box::new(Stub::default()),
            installed_firstmate(),
            &data,
        ));
        assert_eq!(rt.state.ws.earlier, 2);
        assert_eq!(rt.state.ws.chat[0], said);
        assert_eq!(rt.state.ws.chat[1], replied);
        assert!(rt.find("earlier-divider").is_some());
        chord(&mut rt, 'f', false);
        type_text(&mut rt, "zeppelin");
        assert_eq!(find_count(&rt), ["1 of 2"]);
        assert!(on_screen(&rt, &find::match_key(0)));
    }

    #[test]
    fn a_messages_link_button_copies_its_link() {
        let mut rt = runtime();
        let board = rune::clipboard::memory();
        rt.use_clipboard(&board);
        let id = rt.state.ws.chat[1].id.clone();
        assert_eq!(rt.state.hovered, None);
        // The button shows once the pointer is over its message.
        let (x, y) = rt.find("msg-1-link").unwrap();
        rt.pointer_move(x, y);
        settle(&mut rt);
        assert_eq!(rt.state.hovered, Some(1));
        tap(&mut rt, "msg-1-link");
        assert_eq!(rt.state.copied.as_deref(), Some(id.as_str()), "pressed");
        assert_eq!(
            board.text().as_deref().map(|t| t.as_str()),
            Some(links::url(&id).as_str())
        );
        assert_eq!(texts_under(&rt, "msg-1-link"), ["Link copied"]);
        assert_eq!(rt.state.hovered, Some(1));
    }

    #[test]
    fn opening_a_message_link_scrolls_to_the_message_and_highlights_it_briefly() {
        let mut rt = runtime_with(long_chat(40));
        settle(&mut rt);
        assert!(!on_screen(&rt, "msg-5"));
        let url = links::url(&rt.state.ws.chat[5].id);
        rt.update(|s, tx| s.open_link(&url, tx));
        settle(&mut rt);
        assert!(on_screen(&rt, "msg-5"));
        assert!(matches!(&rt.state.flash, Some((Ok(_), _))));
        for _ in 0..200 {
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(rt.state.flash, None, "the highlight fades");
        // From another page, the link comes back to the chat.
        rt.state.showing = Showing::Approvals;
        let url = links::url(&rt.state.ws.chat[0].id);
        rt.update(|s, tx| s.open_link(&url, tx));
        settle(&mut rt);
        assert_eq!(rt.state.showing, Showing::Chat);
        assert!(on_screen(&rt, "msg-0"));
    }

    #[test]
    fn a_link_pasted_into_find_goes_to_its_message() {
        let mut rt = runtime_with(long_chat(40));
        let url = links::url(&rt.state.ws.chat[3].id);
        chord(&mut rt, 'f', false);
        type_text(&mut rt, &url);
        assert!(on_screen(&rt, "msg-3"));
        assert_eq!(find_count(&rt), ["Went to the linked message"]);
        rt.state.find.query.text = links::url("0000deadbeef");
        settle(&mut rt);
        assert_eq!(find_count(&rt), ["Message not found"]);
    }

    #[test]
    fn links_in_messages_show_as_chips_naming_their_message() {
        let mut rt = runtime();
        let answer = rt.state.ws.chat[1].id.clone();
        let words = format!(
            "Do what this says: {} and {}",
            links::url(&answer),
            links::url("0000deadbeef")
        );
        rt.update(|s, _| {
            let first = links::url(&s.ws.chat[0].id);
            s.ws.chat.push(Message::new(Author::Captain, words));
            s.ws.chat.push(Message::new(
                Author::ChiefOfStaff,
                format!("As you asked in {first}."),
            ));
        });
        settle(&mut rt);
        let all = texts(&rt).join("\n");
        assert!(
            all.contains(
                "Do what this says: \u{21aa} Chief of staff: On it. I've split the work between \
                 two crewmates. Two questi… and \u{26a0} Message not found"
            ),
            "{all}"
        );
        assert!(all.contains("\u{21aa} You: Get the desktop skeleton building and check it"));
        assert!(
            !all.contains("velvet-hammer://"),
            "links show as chips: {all}"
        );
    }

    #[test]
    fn a_link_in_the_captains_message_reaches_the_chief_of_staff_quoted() {
        let mut rt = runtime_with(fake_in(Mode::Run));
        assert_eq!(send_and_wait(&mut rt, "links?"), "You said: links: told");
        assert_eq!(
            send_and_wait(&mut rt, "links?"),
            "You said: links: not told",
            "told once per session"
        );
        send_and_wait(&mut rt, "Plan: merge the docs PR first.");
        let answer = rt.state.ws.chat.last().unwrap().id.clone();
        let words = format!(
            "do what this answer says: {} {}",
            links::url(&answer),
            links::url("0000deadbeef")
        );
        let reply = send_and_wait(&mut rt, &words);
        assert!(reply.starts_with(&format!("You said: {words}")), "{reply}");
        assert!(
            reply.contains(&format!(
                "<<< {}, written by you, the chief of staff:\nYou said: Plan: merge the docs PR \
                 first.\n>>>",
                links::url(&answer)
            )),
            "{reply}"
        );
        assert!(reply.contains(&format!(
            "<<< {}: message not found.",
            links::url("0000deadbeef")
        )));
        // The captain's own message keeps their words, with the links as chips.
        let said = &rt.state.ws.chat[rt.state.ws.chat.len() - 2];
        assert_eq!(said.text, words);
    }

    #[test]
    fn nothing_kept_from_an_earlier_session_reaches_the_chief_of_staff_unless_linked() {
        let home = scratch("signed-in");
        let mut first = runtime_with(on_fake(FAKE_CLAUDE, &home, &[]));
        wait(&mut first, settled);
        send_and_wait(&mut first, "The zeppelin code is 42.");
        let kept = first.state.ws.chat[first.state.ws.chat.len() - 2]
            .id
            .clone();
        drop(first);

        let mut rt = runtime_with(on_fake(FAKE_CLAUDE, &home, &[]));
        wait(&mut rt, settled);
        assert_eq!(rt.state.ws.earlier, 2);
        assert_eq!(send_and_wait(&mut rt, "hello"), "You said: hello");
        let reply = send_and_wait(&mut rt, &format!("what was {}", links::url(&kept)));
        assert!(
            reply.contains("written by the captain:\nThe zeppelin code is 42."),
            "{reply}"
        );
    }

    #[test]
    fn the_main_window_keeps_its_panels_and_settings_hold_every_setting() {
        let mut rt = runtime();
        for key in ["agents", "chat", "needs-input", "for-you"] {
            assert!(rt.find(key).is_some(), "{key}");
        }
        for key in [
            "approvals",
            "visuals",
            "rules",
            "firstmate",
            "history-section",
        ] {
            assert!(
                rt.find(key).is_none(),
                "{key} is in Settings, not the main window"
            );
        }
        chord(&mut rt, ',', false);
        assert_eq!(rt.state.showing, Showing::Settings);
        for key in [
            "approvals",
            "visuals",
            "rules",
            "firstmate",
            "history-section",
        ] {
            assert!(rt.find(key).is_some(), "{key}");
        }
        assert_eq!(
            texts_under(&rt, "approvals-summary"),
            ["Run without asking"]
        );
        // Each section opens its page, which comes back to Settings and to the chat.
        for (card, page) in [
            ("approvals", Showing::Approvals),
            ("visuals", Showing::Visuals),
            ("rules", Showing::Rules),
            ("firstmate", Showing::Firstmate),
        ] {
            tap(&mut rt, card);
            assert_eq!(rt.state.showing, page);
            tap(&mut rt, &format!("{card}-all-settings"));
            assert_eq!(rt.state.showing, Showing::Settings);
        }
        tap(&mut rt, "settings-close");
        assert_eq!(rt.state.showing, Showing::Chat);
    }

    #[test]
    fn turning_history_off_keeps_nothing_new_and_on_again_keeps_it() {
        let data = scratch("unused");
        let open = || App::new(Box::new(Stub::default()), installed_firstmate(), &data);
        let mut rt = runtime_with(open());
        assert_eq!(*rt.state.keeping.settings(), history::Keeping(true));
        chord(&mut rt, ',', false);
        assert!(
            texts(&rt)
                .iter()
                .any(|t| t == "\u{25cf} Keep the conversation (Recommended)")
        );
        tap_scrolled(&mut rt, "history-off");
        assert_eq!(*rt.state.keeping.settings(), history::Keeping(false));
        rt.state.draft.text = "Not kept.".into();
        rt.update(|s, _| s.send());
        drop(rt);
        let mut rt = runtime_with(open());
        assert_eq!(
            rt.state.ws.earlier, 0,
            "nothing was kept, and nothing is shown"
        );
        chord(&mut rt, ',', false);
        tap_scrolled(&mut rt, "history-keep");
        rt.update(|s, _| {
            s.draft.text = "Kept.".into();
            s.send();
        });
        drop(rt);
        let rt = runtime_with(open());
        assert_eq!(rt.state.ws.earlier, 2);
        assert_eq!(rt.state.ws.chat[0].text, "Kept.");
    }

    #[test]
    fn clearing_history_asks_first_then_deletes_it_and_the_earlier_sessions() {
        let mut rt = runtime_with(reopened(&format!("clear-{}", std::process::id())));
        let path = rt.state.history.path().to_path_buf();
        assert!(path.exists());
        assert_eq!(rt.state.ws.earlier, 2);
        chord(&mut rt, ',', false);
        tap_scrolled(&mut rt, "history-clear");
        assert!(
            path.exists(),
            "nothing is deleted before the captain confirms"
        );
        tap_scrolled(&mut rt, "history-clear-cancel");
        assert!(rt.find("history-clear-confirm").is_none());
        tap_scrolled(&mut rt, "history-clear");
        tap_scrolled(&mut rt, "history-clear-confirm");
        assert!(!path.exists());
        assert_eq!(rt.state.ws.earlier, 0);
        assert!(
            rt.state
                .ws
                .chat
                .iter()
                .all(|m| !m.text.contains("zeppelin")),
            "the earlier session left the chat"
        );
        assert_eq!(rt.state.ws.chat.len(), 2, "this session's messages stay");
    }

    #[test]
    fn the_in_window_menus_reach_settings_and_find() {
        let mut app = long_chat(5);
        app.in_window_menus = true;
        app.platform = menus::Platform::Windows;
        let mut rt = runtime_with(app);
        assert!(rt.find("menu-bar").is_some());
        tap(&mut rt, "menu-file");
        assert!(rt.find("menu-item-settings").is_some());
        assert_eq!(
            texts_under(&rt, "menu-item-settings"),
            ["Settings", "Ctrl+,"]
        );
        tap(&mut rt, "menu-item-settings");
        assert_eq!(rt.state.showing, Showing::Settings);
        assert_eq!(rt.state.open_menu, None);
        tap(&mut rt, "menu-edit");
        tap(&mut rt, "menu-item-find");
        assert_eq!(rt.state.showing, Showing::Chat);
        type_text(&mut rt, "release");
        assert_eq!(find_count(&rt), ["1 of 1"]);
        // Linux calls it Preferences, in the Edit menu.
        rt.update(|s, _| s.platform = menus::Platform::Linux);
        tap(&mut rt, "menu-edit");
        assert_eq!(
            texts_under(&rt, "menu-item-settings"),
            ["Preferences", "Ctrl+,"]
        );
        press_key(&mut rt, Key::Escape, false);
        assert_eq!(rt.state.open_menu, None);
    }
}
