//! Velvet Hammer: one conversation with a chief of staff, the subagents it manages, and two
//! panels for what waits on the captain. `cargo run` opens the window;
//! `cargo run -- --snap chat.png` renders it headless (see the README for every scene).

mod agent;
mod approval;
mod bearings;
mod bridge;
mod claude;
mod crew;
mod firstmate;
mod guard;
mod markdown;
mod model;
mod pi;
mod projects;
mod store;
mod tools;
mod ui;
mod view;
mod visual;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use rune::{AppSpec, Image, ImageCache, Player, TextEdit, Tx, Waker};

use agent::{ChiefOfStaff, Event, Stub};
use approval::{Activity, Mode, Store};
use bearings::Bearings;
use claude::Claude;
use firstmate::{Checkout, Distro, Firstmate, Work, lab::Lab};
use model::{Action, Author, Id, Message, Status, Workspace};
use projects::Project;
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
    /// The captain's worker nicknames, in the firstmate home's `config/`.
    pub nicknames: store::Store<crew::Nicknames>,
    /// The Workers page's nickname editor.
    pub crew_form: crew::Form,
    /// The pi runtime the app installs for workers on other providers.
    pub runtime: pi::Runtime,
    /// Whether the opencode Go key is in the keychain, as last checked; None before the
    /// Workers page first opens.
    pub key: Option<pi::Key>,
    /// How the key is checked: [`pi::key_state`], or a stand-in in tests.
    pub key_check: fn() -> pi::Key,
    /// Why firstmate's pi account setting could not be written.
    pub pin_problem: Option<String>,
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
    /// Workers on other providers: the pi runtime, the key, and the worker nicknames.
    Workers,
    /// One image from the chat, enlarged, by its tool use's id.
    Picture(String),
}

impl App {
    /// The app on `chief`, keeping its tool-approval settings and activity log in `data`.
    pub fn new(mut chief: Box<dyn ChiefOfStaff>, firstmate: Firstmate, data: &Path) -> Self {
        let watcher = chief
            .workplace()
            .map(|clone| bearings::Watcher::start(clone, &firstmate.home_dir()));
        let project_watcher = chief
            .workplace()
            .map(|_| projects::Watcher::start(&firstmate.home_dir()));
        let approvals = Store::open(data);
        let visuals = store::Store::<visual::Mode>::open(data);
        let rules = store::Store::<guard::Settings>::open(data);
        let home = firstmate.home_dir();
        let nicknames = store::Store::<crew::Nicknames>::open(&home.join("config"));
        let mut runtime = pi::Runtime::open(data, pi::Release::official());
        runtime.list_models();
        chief.approve_with(bridge::Policies {
            approval: approvals.shared(),
            visual: visuals.shared(),
            rules: rules.shared(),
            nicknames: nicknames.shared(),
            models: runtime.models(),
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
        let mut app = App {
            ws: chief.open(),
            chief,
            status: Status::Checking,
            firstmate,
            approvals,
            form: approval::Form::default(),
            activity: Activity::open(data),
            visuals,
            visual_example: TextEdit::new(visual::EXAMPLE),
            rules,
            nicknames,
            crew_form: crew::Form::default(),
            runtime,
            key: None,
            key_check: pi::key_state,
            pin_problem: None,
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
        app.drain();
        app
    }

    /// Applies what the agent layer reported; true when anything changed.
    pub fn drain(&mut self) -> bool {
        let events = self.chief.events();
        let mut changed = self.firstmate.poll() | !events.is_empty();
        changed |= self.runtime.poll();
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
        changed
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
                if done {
                    self.replying = None;
                }
            }
            Event::Problem(text) => {
                self.replying = None;
                self.ws.chat.push(Message::new(Author::Problem, text));
            }
            Event::Item(item) => self.ws.items.push(item),
            Event::Withdrawn(id) => self.ws.items.retain(|i| i.id != id),
            Event::Activity(entry) => self.activity.record(entry),
            Event::Notice(text) => self.ws.chat.push(Message::new(Author::App, text)),
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
                self.media.insert(id, media);
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

    /// Opens the Workers page, checking the keychain for the provider key on the way.
    pub fn open_workers(&mut self) {
        self.showing = Showing::Workers;
        self.check_key();
    }

    pub fn check_key(&mut self) {
        self.key = Some((self.key_check)());
    }

    /// Points firstmate's pi workers at the app's pi directory (see [`pi::connect`]), on the
    /// captain's press of the Workers page's button, then lists the models pi can use.
    pub fn connect_workers(&mut self) {
        self.pin_problem = pi::connect(&self.runtime, &self.firstmate.home_dir()).err();
        self.runtime.list_models();
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
    // The pi runtime goes first on the chief of staff's PATH, where firstmate's spawn finds it
    // for pi workers. Claude Code itself is found and signed in as before.
    let path = pi::Runtime::open(&firstmate::data_dir(), None).path_env(std::env::var_os("PATH"));
    let claude = Claude::new("claude")
        .env("PATH", path.to_string_lossy())
        .in_firstmate(firstmate.clone_dir(), firstmate.home_dir());
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

fn poll(s: &mut App, _: f64, _: &mut Tx) -> bool {
    s.drain()
}

fn setup(s: &mut App, wake: Waker) {
    s.firstmate.connect(wake.clone());
    if let Some(watcher) = &mut s.watcher {
        watcher.connect(wake.clone());
    }
    if let Some(watcher) = &mut s.project_watcher {
        watcher.connect(wake.clone());
    }
    s.runtime.connect(wake.clone());
    s.wake = Some(wake.clone());
    let _ = s.window_wake.set(wake.clone());
    s.chief.connect(wake);
}

fn main() {
    rune::run(AppSpec::<App> {
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
            // Workers on other providers: the page before anything is set up, the page with pi
            // installed from a stand-in release, the key found and firstmate pointed at it, a
            // Claude model refused for pi, and the agents list showing what each worker runs on.
            ("workers-fresh", |s| {
                *s = workers_lab("workers-fresh", false);
                s.key_check = || pi::Key::Missing;
                s.open_workers();
            }),
            ("workers-settings", |s| {
                *s = workers_lab("workers-settings", true);
                s.open_workers();
            }),
            ("workers-claude-refused", |s| {
                *s = workers_lab("workers-claude-refused", true);
                s.open_workers();
                s.crew_form.nickname.text = "poet".into();
                s.crew_form.model.text = "opencode-go/claude-haiku-5-5".into();
                s.crew_form.save(&mut s.nicknames);
            }),
            ("workers-crew", |s| {
                let lab = Lab::at(Path::new("/tmp/velvet-hammer/workers-crew"));
                lab.bearings(bearings::SNAPSHOT);
                lab.meta(
                    "lab-verify",
                    "harness=pi\nmodel=opencode-go/deepseek-v4.1-flash\neffort=default\n",
                );
                lab.meta("lab-layout", "harness=claude\nmodel=claude-sonnet-5-5\n");
                *s = fake_in_firstmate(&lab, true);
                s.settle(|s| s.captain.crew.iter().any(|c| c.runs.is_some()));
            }),
            ("rules-project-ask", |s| {
                *s = rules_lab("rules-project-ask");
                let file = s.firstmate.home_dir().join("projects/site/README.md");
                s.draft.text = format!("write: {} Hello", file.display());
                s.send();
                s.settle(|s| s.ws.needs_input().next().is_some());
            }),
        ],
        min_size: Some((900.0, 560.0)),
        setup: Some(setup),
        poll: Some(poll),
        ..AppSpec::new("Velvet Hammer", (1100.0, 720.0), init, view::view)
    });
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

/// The fake `claude` running as firstmate in a lab under `/tmp/velvet-hammer/NAME`, with the pi
/// runtime installable from a stand-in release. When `ready`, pi is installed, firstmate is
/// pointed at it, the keychain stand-in has the key, and a second nickname is added.
fn workers_lab(name: &str, ready: bool) -> App {
    let root = Path::new("/tmp/velvet-hammer").join(name);
    let lab = Lab::at(&root);
    let mut app = fake_in_firstmate(&lab, true);
    app.runtime
        .set_release(pi::lab::release(&root.join("pi-release"), false));
    app.key_check = || pi::Key::Present;
    if ready {
        app.runtime.install();
        app.runtime.settle();
        app.connect_workers();
        app.runtime.settle();
        app.crew_form.nickname.text = "night-owl".into();
        app.crew_form.harness = crew::Harness::Claude;
        app.crew_form.model.text = "opus".into();
        app.crew_form.effort.text = "high".into();
        app.crew_form.save(&mut app.nicknames);
    }
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
    fn opening_the_app_writes_nothing_into_the_firstmate_home() {
        let root = scratch("quiet-home");
        let firstmate = Firstmate::showing(
            root.join("clone"),
            firstmate::UPSTREAM,
            Distro::Installed(Checkout::default()),
            None,
        );
        let home = firstmate.home_dir();
        std::fs::create_dir_all(&home).unwrap();
        let claude = Claude::new(FAKE_CLAUDE)
            .env("FAKE_CLAUDE_HOME", scratch("signed-in").to_string_lossy())
            .in_firstmate(firstmate.clone_dir(), home.clone());
        let app = App::new(Box::new(claude), firstmate, &root.join("data"));
        assert_eq!(std::fs::read_dir(&home).unwrap().count(), 0);
        assert_eq!(app.nicknames.settings().presets.len(), 1);
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
        tap(&mut rt, "firstmate");
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
        tap(&mut rt, "approvals");
        assert_eq!(rt.state.showing, Showing::Approvals);
        assert!(rt.find("mode-run-without-asking").is_some());
        assert!(rt.find("rules-empty").is_some());
        tap(&mut rt, "mode-ask-every-time");
        assert_eq!(rt.state.approvals.settings().mode, Mode::Ask);
        tap(&mut rt, "rule-effect-deny");
        rt.state.form.tool.text = "Bash".into();
        rt.state.form.pattern.text = "git push".into();
        tap(&mut rt, "rule-save");
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
        tap(&mut rt, "approvals");
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
        tap(&mut rt, "approvals");
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
        tap(&mut rt, "approvals");
        tap(&mut rt, "approvals-activity");
        assert!(rt.find("activity-empty").is_some());
        tap(&mut rt, "activity-close");
        send_and_wait(&mut rt, "run: cargo test");
        send_and_wait(&mut rt, "write: /tmp/notes.txt hello");
        tap(&mut rt, "approvals");
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
        tap(&mut rt, "visuals");
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
        tap(&mut rt, "visuals");
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
        assert!(texts(&rt).iter().any(|t| t == "Nudge only"));
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
        tap(&mut rt, "rules");
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

    #[test]
    fn the_rules_page_lists_every_rule_on_by_default() {
        let mut rt = runtime();
        assert_eq!(texts_under(&rt, "rules-summary"), ["8 of 8 on"]);
        tap(&mut rt, "rules");
        for rule in guard::Rule::ALL {
            assert!(
                rt.find(&format!("guard-{}", rule.key())).is_some(),
                "{rule:?}"
            );
            assert!(rt.find(&format!("guard-{}-toggle", rule.key())).is_some());
        }
        tap_scrolled(&mut rt, "guard-publish-asks-captain-toggle");
        assert_eq!(texts_under(&rt, "rules-summary"), ["7 of 8 on"]);
        tap(&mut rt, "rules-activity");
        assert_eq!(rt.state.showing, Showing::Activity);
    }
}
