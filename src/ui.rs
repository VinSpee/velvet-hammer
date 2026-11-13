//! A native renderer for json-render specs (see [docs/json-render.md](../docs/json-render.md)):
//! the chief of staff describes a small interface as JSON with the `app_ui` tool, and the chat
//! draws it with the app's own views. This module holds everything but the drawing: the catalog
//! the model may use, reading a spec (whole or still streaming), its state, the tree to draw, and
//! the actions a control fires. A spec is untrusted model output: it is capped in size, unknown
//! props are dropped, and nothing in it runs code, touches the disk or reaches the network.

use std::collections::{BTreeMap, HashMap};

use rune::TextEdit;
use serde_json::{Map, Value, json};

/// The MCP tool that shows a spec, and its name as Claude Code reports it.
pub const TOOL: &str = "app_ui";
pub fn tool_name() -> String {
    format!("mcp__{}__{TOOL}", crate::bridge::SERVER)
}

/// The largest spec, as JSON text, the app reads.
pub const MAX_BYTES: usize = 64 * 1024;
/// The deepest a spec's JSON may nest.
pub const MAX_DEPTH: usize = 24;
/// The most elements a spec may define.
pub const MAX_ELEMENTS: usize = 200;
/// The most nodes one view draws, counting every copy a repeat makes.
pub const MAX_NODES: usize = 400;
/// The deepest the drawn tree may nest.
const MAX_TREE_DEPTH: usize = 16;

/// What a prop holds when it is written literally rather than as an expression.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Text,
    Number,
    Bool,
    Choice(&'static [&'static str]),
    /// An array of strings.
    Texts,
    /// An array of arrays of strings.
    Rows,
}

pub struct Prop {
    pub name: &'static str,
    pub kind: Kind,
    pub required: bool,
}

pub struct Component {
    pub name: &'static str,
    pub props: &'static [Prop],
    pub children: bool,
    pub events: &'static [&'static str],
    pub about: &'static str,
}

pub struct ActionDef {
    pub name: &'static str,
    pub params: &'static str,
    pub about: &'static str,
}

const fn prop(name: &'static str, kind: Kind) -> Prop {
    Prop {
        name,
        kind,
        required: false,
    }
}

const fn needs(name: &'static str, kind: Kind) -> Prop {
    Prop {
        name,
        kind,
        required: true,
    }
}

const GAP: Kind = Kind::Choice(&["none", "sm", "md", "lg"]);

/// The catalog: the only components and actions a spec may use. The renderer, the tool's
/// validation and the tool's description all read it. Names and props follow json-render's
/// shadcn catalog where it has the same component.
pub static CATALOG: &[Component] = &[
    Component {
        name: "Stack",
        props: &[
            prop("direction", Kind::Choice(&["vertical", "horizontal"])),
            prop("gap", GAP),
        ],
        children: true,
        events: &[],
        about: "Lays out its children in a column (vertical, the default) or a row",
    },
    Component {
        name: "Card",
        props: &[prop("title", Kind::Text), prop("description", Kind::Text)],
        children: true,
        events: &[],
        about: "A bordered box that groups its children",
    },
    Component {
        name: "Separator",
        props: &[],
        children: false,
        events: &[],
        about: "A horizontal line",
    },
    Component {
        name: "Heading",
        props: &[
            needs("text", Kind::Text),
            prop("level", Kind::Choice(&["h1", "h2", "h3", "h4"])),
        ],
        children: false,
        events: &[],
        about: "A heading",
    },
    Component {
        name: "Text",
        props: &[
            needs("text", Kind::Text),
            prop(
                "variant",
                Kind::Choice(&["body", "caption", "muted", "code"]),
            ),
        ],
        children: false,
        events: &[],
        about: "A paragraph",
    },
    Component {
        name: "Badge",
        props: &[
            needs("text", Kind::Text),
            prop(
                "variant",
                Kind::Choice(&["default", "secondary", "destructive", "outline"]),
            ),
        ],
        children: false,
        events: &[],
        about: "A short status label",
    },
    Component {
        name: "Alert",
        props: &[
            needs("title", Kind::Text),
            prop("message", Kind::Text),
            prop(
                "type",
                Kind::Choice(&["info", "success", "warning", "error"]),
            ),
        ],
        children: false,
        events: &[],
        about: "A banner",
    },
    Component {
        name: "Progress",
        props: &[
            needs("value", Kind::Number),
            prop("max", Kind::Number),
            prop("label", Kind::Text),
        ],
        children: false,
        events: &[],
        about: "A progress bar; max defaults to 100",
    },
    Component {
        name: "Image",
        props: &[needs("src", Kind::Text), prop("alt", Kind::Text)],
        children: false,
        events: &[],
        about: "An image file by absolute path; only files a tool in this session already named \
                are shown, and never a URL",
    },
    Component {
        name: "Table",
        props: &[
            needs("columns", Kind::Texts),
            needs("rows", Kind::Rows),
            prop("caption", Kind::Text),
        ],
        children: false,
        events: &[],
        about: "A table; rows is an array of rows of cell strings",
    },
    Component {
        name: "Button",
        props: &[
            needs("label", Kind::Text),
            prop("variant", Kind::Choice(&["primary", "secondary", "danger"])),
            prop("disabled", Kind::Bool),
        ],
        children: false,
        events: &["press"],
        about: "A button",
    },
    Component {
        name: "Input",
        props: &[
            needs("label", Kind::Text),
            prop("placeholder", Kind::Text),
            prop("value", Kind::Text),
        ],
        children: false,
        events: &["submit"],
        about: "A one-line text field; bind value with $bindState, submit fires on Return",
    },
    Component {
        name: "Checkbox",
        props: &[needs("label", Kind::Text), prop("checked", Kind::Bool)],
        children: false,
        events: &["change"],
        about: "A checkbox; bind checked with $bindState or $bindItem",
    },
];

pub static ACTIONS: &[ActionDef] = &[
    ActionDef {
        name: "setState",
        params: "{statePath, value}",
        about: "Sets the value at statePath. Runs in the view only",
    },
    ActionDef {
        name: "pushState",
        params: "{statePath, value, clearStatePath?}",
        about: "Appends value to the array at statePath (\"$id\" in value becomes a new id), then \
                empties clearStatePath. Runs in the view only",
    },
    ActionDef {
        name: "removeState",
        params: "{statePath, index}",
        about: "Removes the item at index from the array at statePath. Runs in the view only",
    },
    ActionDef {
        name: "submit",
        params: "{any params you choose}",
        about: "Sends you a chat message naming the control, its resolved params and the value of \
                every bound input. The only way the view talks to you",
    },
];

pub fn component(name: &str) -> Option<&'static Component> {
    CATALOG.iter().find(|c| c.name == name)
}

fn action(name: &str) -> Option<&'static ActionDef> {
    ACTIONS.iter().find(|a| a.name == name)
}

/// The tool's description: the spec format and the whole catalog, compactly, in the shape of
/// json-render's own prompt.
pub fn description() -> String {
    let mut out = String::from(
        "Show the captain a small interactive view inline in the Velvet Hammer chat, described \
         as a json-render spec. The input is the spec: {\"root\": \"<element id>\", \"elements\": \
         {\"<id>\": {\"type\", \"props\", \"children\": [ids], \"visible\"?, \"repeat\"?, \"on\"?}}, \
         \"state\"?: {...}}. Only these components:\n",
    );
    for c in CATALOG {
        let props: Vec<String> = c
            .props
            .iter()
            .map(|p| {
                let kind = match p.kind {
                    Kind::Text => "string".to_string(),
                    Kind::Number => "number".to_string(),
                    Kind::Bool => "boolean".to_string(),
                    Kind::Choice(options) => options.join("|"),
                    Kind::Texts => "string[]".to_string(),
                    Kind::Rows => "string[][]".to_string(),
                };
                format!("{}{}: {kind}", p.name, if p.required { "" } else { "?" })
            })
            .collect();
        out.push_str(&format!(
            "- {}: {{{}}} - {}",
            c.name,
            props.join(", "),
            c.about
        ));
        if c.children {
            out.push_str(" [accepts children]");
        }
        if !c.events.is_empty() {
            out.push_str(&format!(" [events: {}]", c.events.join(", ")));
        }
        out.push('\n');
    }
    out.push_str(
        "Actions, bound with \"on\": {\"<event>\": {\"action\", \"params\"} or a list}:\n",
    );
    for a in ACTIONS {
        out.push_str(&format!("- {} {}: {}\n", a.name, a.params, a.about));
    }
    out.push_str(
        "Any prop or param may be {\"$state\": \"/json/pointer\"}, {\"$bindState\": \"/path\"} \
         (two-way, for Input value and Checkbox checked), {\"$template\": \"Hi ${/name}\"} or \
         {\"$cond\": <condition>, \"$then\": x, \"$else\": y}. \"repeat\": {\"statePath\": \
         \"/items\"} draws the children once per array item; inside, use {\"$item\": \"field\"}, \
         {\"$index\": true} and {\"$bindItem\": \"field\"}. \"visible\" is a condition: \
         {\"$state\"|\"$item\": path} with at most one of eq, neq, gt, gte, lt, lte, plus \"not\": \
         true; a list (all), {\"$and\": [...]} or {\"$or\": [...]}. Nothing in the view runs code \
         or reads files; the captain's submit comes back to you as a chat message. Limits: 64 KB, \
         200 elements.",
    );
    out
}

/// The tool's input schema: the spec itself.
pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "root": {"type": "string", "description": "The id of the element to draw first."},
            "elements": {"type": "object", "description": "Every element, by id."},
            "state": {"type": "object", "description": "The view's starting state."},
        },
        "required": ["root", "elements"],
    })
}

/// A spec as far as it has arrived. A spec still streaming may have no root yet, and elements
/// whose children are not defined yet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spec {
    pub root: Option<String>,
    pub elements: BTreeMap<String, Element>,
    pub state: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    /// The component type, which may be one the catalog lacks.
    pub kind: String,
    /// Only the props the catalog defines for `kind`.
    pub props: Map<String, Value>,
    pub children: Vec<String>,
    pub visible: Option<Value>,
    pub on: BTreeMap<String, Vec<Binding>>,
    /// `repeat.statePath`: a state path, or `{"$item": field}` inside another repeat.
    pub repeat: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub action: String,
    pub params: Map<String, Value>,
}

/// Reads a spec, refusing one past the limits. Whatever else is wrong (an unknown component or
/// action, a prop of the wrong kind) is dropped or drawn as a fallback, and named in the
/// warnings.
pub fn parse(input: &Value) -> Result<(Spec, Vec<String>), String> {
    let size = input.to_string().len();
    if size > MAX_BYTES {
        return Err(format!(
            "The view is {} KB, more than the {} KB limit, so it is not shown.",
            size.div_ceil(1024),
            MAX_BYTES / 1024
        ));
    }
    if depth(input) > MAX_DEPTH {
        return Err(format!(
            "The view nests deeper than {MAX_DEPTH} levels, so it is not shown."
        ));
    }
    let Some(object) = input.as_object() else {
        return Err("The view is not a JSON object.".into());
    };
    let empty = Map::new();
    let elements = object
        .get("elements")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    if elements.len() > MAX_ELEMENTS {
        return Err(format!(
            "The view has {} elements, more than the limit of {MAX_ELEMENTS}, so it is not shown.",
            elements.len()
        ));
    }
    let mut warnings = Vec::new();
    let mut spec = Spec {
        root: object
            .get("root")
            .and_then(Value::as_str)
            .map(str::to_string),
        elements: BTreeMap::new(),
        state: match object.get("state") {
            Some(Value::Object(state)) => Value::Object(state.clone()),
            _ => Value::Object(Map::new()),
        },
    };
    for (id, raw) in elements {
        match element(id, raw, &mut warnings) {
            Some(element) => {
                spec.elements.insert(id.clone(), element);
            }
            None => warnings.push(format!("Element \"{id}\" has no type, so it is left out.")),
        }
    }
    Ok((spec, warnings))
}

fn element(id: &str, raw: &Value, warnings: &mut Vec<String>) -> Option<Element> {
    let kind = raw["type"].as_str()?.to_string();
    let catalog = component(&kind);
    if catalog.is_none() {
        warnings.push(format!(
            "Element \"{id}\" has the type \"{kind}\", which is not in the catalog; it shows as a \
             placeholder."
        ));
    }
    let mut props = Map::new();
    for (name, value) in raw["props"].as_object().into_iter().flatten() {
        let Some(c) = catalog else { break };
        match c.props.iter().find(|p| p.name == name) {
            Some(p) if expression(value) || fits(p.kind, value) => {
                props.insert(name.clone(), value.clone());
            }
            Some(_) => warnings.push(format!(
                "Element \"{id}\": the prop \"{name}\" has the wrong kind of value, so it is ignored."
            )),
            None => warnings.push(format!(
                "Element \"{id}\": {kind} has no prop \"{name}\", so it is ignored."
            )),
        }
    }
    let mut on = BTreeMap::new();
    for (event, bound) in raw["on"].as_object().into_iter().flatten() {
        let list = match bound {
            Value::Array(list) => list.iter().collect(),
            single => vec![single],
        };
        let bindings: Vec<Binding> = list
            .into_iter()
            .filter_map(|b| {
                let name = b["action"].as_str()?;
                if action(name).is_none() {
                    warnings.push(format!(
                        "Element \"{id}\": the action \"{name}\" is not in the catalog, so it does \
                         nothing."
                    ));
                }
                Some(Binding {
                    action: name.to_string(),
                    params: b["params"].as_object().cloned().unwrap_or_default(),
                })
            })
            .collect();
        on.insert(event.clone(), bindings);
    }
    Some(Element {
        kind,
        props,
        children: raw["children"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c.as_str().map(str::to_string))
            .collect(),
        visible: raw.get("visible").cloned(),
        on,
        repeat: raw["repeat"].get("statePath").cloned(),
    })
}

/// What the tool tells the model about a spec: an error that stops it from being shown, or the
/// warnings about what was left out.
pub fn check(input: &Value) -> Result<Vec<String>, String> {
    let (spec, mut warnings) = parse(input)?;
    let Some(root) = &spec.root else {
        return Err("The view has no \"root\" element id.".into());
    };
    if !spec.elements.contains_key(root) {
        return Err(format!(
            "The root element \"{root}\" is not defined in \"elements\"."
        ));
    }
    for (id, element) in &spec.elements {
        let Some(c) = component(&element.kind) else {
            continue;
        };
        if !c.children && !element.children.is_empty() {
            warnings.push(format!(
                "Element \"{id}\": {} has no children, so they are not shown.",
                c.name
            ));
        }
        for event in element
            .on
            .keys()
            .filter(|e| !c.events.contains(&e.as_str()))
        {
            warnings.push(format!(
                "Element \"{id}\": {} has no event \"{event}\".",
                c.name
            ));
        }
        for p in c
            .props
            .iter()
            .filter(|p| p.required && !element.props.contains_key(p.name))
        {
            warnings.push(format!(
                "Element \"{id}\": {} needs the prop \"{}\".",
                c.name, p.name
            ));
        }
        for child in &element.children {
            if !spec.elements.contains_key(child) {
                warnings.push(format!(
                    "Element \"{id}\" names the child \"{child}\", which is not defined."
                ));
            }
        }
    }
    Ok(warnings)
}

/// The tool's answer to the model: what it tells the model once the view is shown, or why
/// it was refused.
pub fn answer(input: &Value) -> Result<String, String> {
    let mut text = "The view is shown to the captain in the chat. Their submit comes back to you \
                    as a message."
        .to_string();
    for warning in check(input)? {
        text.push_str("\n- ");
        text.push_str(&warning);
    }
    Ok(text)
}

fn depth(value: &Value) -> usize {
    match value {
        Value::Array(items) => 1 + items.iter().map(depth).max().unwrap_or(0),
        Value::Object(fields) => 1 + fields.values().map(depth).max().unwrap_or(0),
        _ => 0,
    }
}

const EXPRESSIONS: [&str; 7] = [
    "$state",
    "$item",
    "$index",
    "$bindState",
    "$bindItem",
    "$cond",
    "$template",
];

fn expression(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|o| EXPRESSIONS.iter().any(|k| o.contains_key(*k)))
}

fn fits(kind: Kind, value: &Value) -> bool {
    match kind {
        Kind::Text => value.is_string() || value.is_number(),
        Kind::Number => value.is_number(),
        Kind::Bool => value.is_boolean(),
        Kind::Choice(options) => value.as_str().is_some_and(|v| options.contains(&v)),
        Kind::Texts | Kind::Rows => value.is_array(),
    }
}

/// The value a JSON text still arriving would have if it ended at its last complete value:
/// unfinished strings, numbers and keys are left out, and open arrays and objects are closed.
/// Text that is not the start of JSON gives `Null`.
pub fn partial(text: &str) -> Value {
    #[derive(Clone, Copy, PartialEq)]
    enum Frame {
        Array,
        /// An object, and whether it is waiting for a key.
        Object(bool),
    }
    let mut stack: Vec<Frame> = Vec::new();
    // The end of the text that would parse, and the open containers there.
    let mut safe: Option<(usize, Vec<Frame>)> = None;
    let (mut in_string, mut escaped, mut in_token) = (false, false, false);
    // A value just ended at `end`: in an array or as a member's value, the text up to it parses.
    let ended =
        |end: usize, stack: &mut Vec<Frame>, safe: &mut Option<(usize, Vec<Frame>)>| match stack
            .last_mut()
        {
            Some(Frame::Object(waiting)) if *waiting => *waiting = false,
            _ => *safe = Some((end, stack.clone())),
        };
    for (at, c) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
                ended(at + 1, &mut stack, &mut safe);
            }
            continue;
        }
        let delimiter = c.is_whitespace() || matches!(c, ',' | ']' | '}' | ':');
        if in_token && delimiter {
            in_token = false;
            ended(at, &mut stack, &mut safe);
        }
        match c {
            '"' => in_string = true,
            '{' | '[' => {
                if stack.len() >= MAX_DEPTH * 2 {
                    break;
                }
                stack.push(if c == '{' {
                    Frame::Object(true)
                } else {
                    Frame::Array
                });
                safe = Some((at + 1, stack.clone()));
            }
            '}' | ']' => {
                stack.pop();
                ended(at + 1, &mut stack, &mut safe);
            }
            ',' => {
                if let Some(Frame::Object(waiting)) = stack.last_mut() {
                    *waiting = true;
                }
            }
            _ if !delimiter => in_token = true,
            _ => {}
        }
    }
    if in_token && stack.is_empty() {
        safe = Some((text.len(), Vec::new()));
    }
    let Some((end, open)) = safe else {
        return Value::Null;
    };
    let mut whole = text[..end].to_string();
    for frame in open.iter().rev() {
        whole.push(if *frame == Frame::Array { ']' } else { '}' });
    }
    serde_json::from_str(&whole).unwrap_or(Value::Null)
}

/// Where an element is drawn: inside which repeat item, if any.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scope {
    /// The state path of the current repeat item, such as `/todos/2`.
    pub item: Option<String>,
    pub index: Option<usize>,
    /// Appended to each element id to make its node's key unique across repeat copies.
    suffix: String,
}

/// The value at a JSON Pointer (RFC 6901) in `value`.
pub fn get<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() || path == "/" {
        return Some(value);
    }
    let pointer = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    value.pointer(&pointer)
}

fn segments(path: &str) -> Vec<String> {
    path.trim_start_matches('/')
        .split('/')
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect()
}

/// Sets the value at `path`, making the objects and arrays on the way. An array index past the
/// end is refused, so a write never leaves holes.
pub fn set(state: &mut Value, path: &str, value: Value) {
    let segments = segments(path);
    let Some((last, parents)) = segments.split_last() else {
        return;
    };
    let mut current = state;
    for (i, segment) in parents.iter().enumerate() {
        let next_is_index = segments[i + 1].parse::<usize>().is_ok() || segments[i + 1] == "-";
        let fresh = || if next_is_index { json!([]) } else { json!({}) };
        current = match current {
            Value::Object(map) => {
                let slot = map.entry(segment.clone()).or_insert_with(fresh);
                if !slot.is_object() && !slot.is_array() {
                    *slot = fresh();
                }
                slot
            }
            Value::Array(items) => match segment.parse::<usize>() {
                Ok(n) if n < items.len() => {
                    if !items[n].is_object() && !items[n].is_array() {
                        items[n] = fresh();
                    }
                    &mut items[n]
                }
                Ok(n) if n == items.len() => {
                    items.push(fresh());
                    &mut items[n]
                }
                _ => return,
            },
            _ => return,
        };
    }
    match current {
        Value::Object(map) => {
            map.insert(last.clone(), value);
        }
        Value::Array(items) => match last.parse::<usize>() {
            Ok(n) if n < items.len() => items[n] = value,
            Ok(n) if n == items.len() => items.push(value),
            _ if last == "-" => items.push(value),
            _ => {}
        },
        _ => {}
    }
}

/// The text a value shows as: strings as written, whole numbers without a fraction.
pub fn display(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// Joins a repeat item's path and a field inside it.
fn join(base: &str, field: &str) -> String {
    match field.trim_start_matches('/') {
        "" => base.to_string(),
        field => format!("{}/{field}", base.trim_end_matches('/')),
    }
}

/// Resolves a prop or param written as an expression, against `state` and the repeat item.
pub fn resolve(value: &Value, state: &Value, scope: &Scope) -> Value {
    let Some(o) = value.as_object() else {
        return match value {
            Value::Array(items) => items.iter().map(|v| resolve(v, state, scope)).collect(),
            other => other.clone(),
        };
    };
    let at = |path: &str| get(state, path).cloned().unwrap_or(Value::Null);
    let item = |field: &str| match &scope.item {
        Some(base) => at(&join(base, field)),
        None => Value::Null,
    };
    if let Some(path) = o.get("$state").and_then(Value::as_str) {
        at(path)
    } else if let Some(path) = o.get("$bindState").and_then(Value::as_str) {
        at(path)
    } else if let Some(field) = o.get("$item").and_then(Value::as_str) {
        item(field)
    } else if let Some(field) = o.get("$bindItem").and_then(Value::as_str) {
        item(field)
    } else if o.get("$index") == Some(&Value::Bool(true)) {
        scope.index.map_or(Value::Null, |i| json!(i))
    } else if let (Some(cond), Some(then), Some(otherwise)) =
        (o.get("$cond"), o.get("$then"), o.get("$else"))
    {
        let chosen = if visible(cond, state, scope) {
            then
        } else {
            otherwise
        };
        resolve(chosen, state, scope)
    } else if let Some(template) = o.get("$template").and_then(Value::as_str) {
        Value::String(fill(template, state, scope))
    } else {
        Value::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), resolve(v, state, scope)))
                .collect(),
        )
    }
}

/// A `$template`: `${/path}` reads state, and `${field}` reads the repeat item, then state.
fn fill(template: &str, state: &Value, scope: &Scope) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let Some(len) = rest[start..].find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &rest[start + 2..start + len];
        let value = if name.starts_with('/') {
            get(state, name)
        } else {
            scope
                .item
                .as_ref()
                .and_then(|base| get(state, &join(base, name)))
                .filter(|v| !v.is_null())
                .or_else(|| get(state, &format!("/{name}")))
        };
        out.push_str(&value.map(display).unwrap_or_default());
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    out
}

/// The absolute state path a `$bindState` or `$bindItem` prop writes to.
fn bound_path(value: Option<&Value>, scope: &Scope) -> Option<String> {
    let o = value?.as_object()?;
    if let Some(path) = o.get("$bindState").and_then(Value::as_str) {
        return Some(path.to_string());
    }
    let field = o.get("$bindItem").and_then(Value::as_str)?;
    Some(join(scope.item.as_ref()?, field))
}

/// Evaluates a json-render visibility condition. A condition the app cannot read shows the
/// element, so nothing hides by mistake.
pub fn visible(cond: &Value, state: &Value, scope: &Scope) -> bool {
    match cond {
        Value::Bool(b) => *b,
        Value::Array(all) => all.iter().all(|c| visible(c, state, scope)),
        Value::Object(o) => {
            if let Some(Value::Array(all)) = o.get("$and") {
                return all.iter().all(|c| visible(c, state, scope));
            }
            if let Some(Value::Array(any)) = o.get("$or") {
                return any.iter().any(|c| visible(c, state, scope));
            }
            let subject = if o.contains_key("$state") || o.contains_key("$item") {
                let key = if o.contains_key("$state") {
                    "$state"
                } else {
                    "$item"
                };
                resolve(&json!({ key: o[key] }), state, scope)
            } else if o.get("$index") == Some(&Value::Bool(true)) {
                scope.index.map_or(Value::Null, |i| json!(i))
            } else {
                return true;
            };
            let operand = |k: &str| o.get(k).map(|v| resolve(v, state, scope));
            let number = |v: &Value| v.as_f64();
            let compare = |k: &str, f: fn(f64, f64) -> bool| {
                operand(k).map(|b| match (number(&subject), number(&b)) {
                    (Some(a), Some(b)) => f(a, b),
                    _ => false,
                })
            };
            let result = operand("eq")
                .map(|b| subject == b)
                .or_else(|| operand("neq").map(|b| subject != b))
                .or_else(|| compare("gt", |a, b| a > b))
                .or_else(|| compare("gte", |a, b| a >= b))
                .or_else(|| compare("lt", |a, b| a < b))
                .or_else(|| compare("lte", |a, b| a <= b))
                .unwrap_or_else(|| truthy(Some(&subject)));
            result != (o.get("not") == Some(&Value::Bool(true)))
        }
        _ => true,
    }
}

/// Whether a condition reads the repeat item, which makes it a filter on a repeat's items.
fn reads_item(cond: &Value) -> bool {
    match cond {
        Value::Array(all) => all.iter().any(reads_item),
        Value::Object(o) => {
            o.contains_key("$item") || o.contains_key("$index") || o.values().any(reads_item)
        }
        _ => false,
    }
}

/// One element as drawn, with its props resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// Unique within the view: the element id, plus the repeat indexes it was copied for.
    pub key: String,
    pub what: What,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum What {
    Stack {
        horizontal: bool,
        gap: String,
    },
    Card {
        title: String,
        description: String,
    },
    Separator,
    Heading {
        text: String,
        level: String,
    },
    Text {
        text: String,
        variant: String,
    },
    Badge {
        text: String,
        variant: String,
    },
    Alert {
        title: String,
        message: String,
        tone: String,
    },
    Progress {
        fraction: f32,
        label: String,
    },
    Image {
        src: String,
        alt: String,
    },
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
        caption: String,
    },
    Button {
        label: String,
        variant: String,
        disabled: bool,
        press: Option<Trigger>,
    },
    Input {
        label: String,
        placeholder: String,
        path: Option<String>,
        value: String,
        submit: Option<Trigger>,
    },
    Checkbox {
        label: String,
        path: Option<String>,
        checked: bool,
        change: Option<Trigger>,
    },
    /// A type the catalog lacks, drawn as a placeholder naming it.
    Unknown(String),
}

/// The actions one control fires, and where it was drawn, so their params resolve against the
/// state as it is when the captain acts.
#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    pub bindings: Vec<Binding>,
    pub scope: Scope,
}

/// A spec drawn against its state.
#[derive(Debug, Default, PartialEq)]
pub struct Tree {
    pub root: Option<Node>,
    /// Every state path an input writes to, for the message a submit sends.
    pub bound: Vec<String>,
    /// More than [`MAX_NODES`] would have been drawn.
    pub cut: bool,
    count: usize,
}

pub fn tree(spec: &Spec, state: &Value) -> Tree {
    let mut out = Tree::default();
    let mut path = Vec::new();
    if let Some(root) = &spec.root {
        out.root = build(spec, state, root, &Scope::default(), &mut path, &mut out);
    }
    out
}

fn build(
    spec: &Spec,
    state: &Value,
    id: &str,
    scope: &Scope,
    path: &mut Vec<String>,
    out: &mut Tree,
) -> Option<Node> {
    let element = spec.elements.get(id)?;
    if path.len() >= MAX_TREE_DEPTH || path.iter().any(|p| p == id) {
        return None;
    }
    if out.count >= MAX_NODES {
        out.cut = true;
        return None;
    }
    // A repeat whose condition reads the item filters its items instead of hiding itself.
    let filter = element
        .visible
        .as_ref()
        .filter(|c| element.repeat.is_some() && scope.item.is_none() && reads_item(c));
    if let Some(cond) = &element.visible
        && filter.is_none()
        && !visible(cond, state, scope)
    {
        return None;
    }
    let mut node = Node {
        key: format!("{id}{}", scope.suffix),
        what: what(element, state, scope, out),
        children: Vec::new(),
    };
    out.count += 1;
    path.push(id.to_string());
    let copies: Vec<Scope> = match &element.repeat {
        None => vec![scope.clone()],
        Some(source) => {
            let base = match source {
                Value::String(p) => Some(p.clone()),
                Value::Object(o) => o
                    .get("$item")
                    .and_then(Value::as_str)
                    .zip(scope.item.as_ref())
                    .map(|(field, base)| join(base, field)),
                _ => None,
            };
            let items = base
                .as_ref()
                .and_then(|b| get(state, b))
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            (0..items)
                .map(|i| {
                    let base = base.as_ref().expect("items come from base");
                    Scope {
                        item: Some(format!("{}/{i}", base.trim_end_matches('/'))),
                        index: Some(i),
                        suffix: format!("{}.{i}", scope.suffix),
                    }
                })
                .filter(|s| filter.is_none_or(|c| visible(c, state, s)))
                .collect()
        }
    };
    'copies: for copy in &copies {
        for child in &element.children {
            if out.count >= MAX_NODES {
                out.cut = true;
                break 'copies;
            }
            if let Some(n) = build(spec, state, child, copy, path, out) {
                node.children.push(n);
            }
        }
    }
    path.pop();
    Some(node)
}

fn what(element: &Element, state: &Value, scope: &Scope, out: &mut Tree) -> What {
    let raw = |name: &str| element.props.get(name);
    let value = |name: &str| raw(name).map_or(Value::Null, |v| resolve(v, state, scope));
    let text = |name: &str| display(&value(name));
    let choice = |name: &str, default: &str| {
        let v = text(name);
        if v.is_empty() { default.to_string() } else { v }
    };
    let trigger = |event: &str| {
        element
            .on
            .get(event)
            .filter(|b| !b.is_empty())
            .map(|b| Trigger {
                bindings: b.clone(),
                scope: scope.clone(),
            })
    };
    let strings =
        |v: &Value| -> Vec<String> { v.as_array().into_iter().flatten().map(display).collect() };
    match element.kind.as_str() {
        "Stack" => What::Stack {
            horizontal: text("direction") == "horizontal",
            gap: choice("gap", "md"),
        },
        "Card" => What::Card {
            title: text("title"),
            description: text("description"),
        },
        "Separator" => What::Separator,
        "Heading" => What::Heading {
            text: text("text"),
            level: choice("level", "h2"),
        },
        "Text" => What::Text {
            text: text("text"),
            variant: choice("variant", "body"),
        },
        "Badge" => What::Badge {
            text: text("text"),
            variant: choice("variant", "default"),
        },
        "Alert" => What::Alert {
            title: text("title"),
            message: text("message"),
            tone: choice("type", "info"),
        },
        "Progress" => {
            let max = value("max").as_f64().filter(|m| *m > 0.0).unwrap_or(100.0);
            let fraction = value("value").as_f64().unwrap_or(0.0) / max;
            What::Progress {
                fraction: fraction.clamp(0.0, 1.0) as f32,
                label: text("label"),
            }
        }
        "Image" => What::Image {
            src: text("src"),
            alt: text("alt"),
        },
        "Table" => What::Table {
            columns: strings(&value("columns")),
            rows: value("rows")
                .as_array()
                .into_iter()
                .flatten()
                .map(strings)
                .collect(),
            caption: text("caption"),
        },
        "Button" => What::Button {
            label: text("label"),
            variant: choice("variant", "primary"),
            disabled: truthy(Some(&value("disabled"))),
            press: trigger("press"),
        },
        "Input" => {
            let path = bound_path(raw("value"), scope);
            out.bound.extend(path.clone());
            What::Input {
                label: text("label"),
                placeholder: text("placeholder"),
                path,
                value: text("value"),
                submit: trigger("submit"),
            }
        }
        "Checkbox" => {
            let path = bound_path(raw("checked"), scope);
            out.bound.extend(path.clone());
            What::Checkbox {
                label: text("label"),
                path,
                checked: truthy(Some(&value("checked"))),
                change: trigger("change"),
            }
        }
        other => What::Unknown(other.to_string()),
    }
}

/// What firing one action did.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// It changed the view's state, or nothing.
    Local,
    /// `submit`: the params to send the chief of staff.
    Send(Value),
    /// An action the catalog does not have; it does nothing.
    Unknown(String),
}

/// Runs one action against the view's state. Only `submit` reaches beyond the view, and only
/// as a message; nothing runs code or touches the disk.
pub fn run(state: &mut Value, binding: &Binding, scope: &Scope, ids: &mut u64) -> Outcome {
    // In params, `$item` names the item's path, so it can be a statePath.
    let param = |name: &str, state: &Value| -> Value {
        match binding.params.get(name) {
            Some(Value::Object(o)) if o.contains_key("$item") => {
                match (o["$item"].as_str(), &scope.item) {
                    (Some(field), Some(base)) => Value::String(join(base, field)),
                    _ => Value::Null,
                }
            }
            Some(v) => resolve(v, state, scope),
            None => Value::Null,
        }
    };
    let path = |state: &Value| param("statePath", state).as_str().map(str::to_string);
    match binding.action.as_str() {
        "setState" => {
            if let Some(p) = path(state) {
                let value = param("value", state);
                set(state, &p, value);
            }
            Outcome::Local
        }
        "pushState" => {
            if let Some(p) = path(state) {
                let value = with_ids(param("value", state), ids);
                match get(state, &p) {
                    Some(Value::Array(_)) => {}
                    _ => set(state, &p, json!([])),
                }
                if let Some(Value::Array(items)) = state.pointer_mut(&pointer(&p)) {
                    items.push(value);
                }
                if let Some(clear) = param("clearStatePath", state).as_str() {
                    set(state, clear, json!(""));
                }
            }
            Outcome::Local
        }
        "removeState" => {
            if let (Some(p), Some(i)) = (path(state), param("index", state).as_u64())
                && let Some(Value::Array(items)) = state.pointer_mut(&pointer(&p))
                && (i as usize) < items.len()
            {
                items.remove(i as usize);
            }
            Outcome::Local
        }
        "submit" => Outcome::Send(Value::Object(
            binding
                .params
                .keys()
                .map(|k| (k.clone(), param(k, state)))
                .collect(),
        )),
        other => Outcome::Unknown(other.to_string()),
    }
}

fn pointer(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

/// `"$id"` in a pushed value becomes a new id.
fn with_ids(value: Value, ids: &mut u64) -> Value {
    match value {
        Value::String(s) if s == "$id" => {
            *ids += 1;
            Value::String(format!("id-{ids}"))
        }
        Value::Object(o) => {
            Value::Object(o.into_iter().map(|(k, v)| (k, with_ids(v, ids))).collect())
        }
        Value::Array(a) => Value::Array(a.into_iter().map(|v| with_ids(v, ids)).collect()),
        other => other,
    }
}

/// One view's state in the chat: the spec's own state until the captain changes something,
/// and the text fields being typed in, by the state path they write.
#[derive(Default)]
pub struct Live {
    local: Option<Value>,
    pub edits: HashMap<String, TextEdit>,
    ids: u64,
}

impl Live {
    /// The state as the captain sees it: the spec's, or the captain's changes to it, with what
    /// they are typing.
    pub fn state(&self, spec: &Spec) -> Value {
        let mut state = self.local.clone().unwrap_or_else(|| spec.state.clone());
        // A field bound to nothing is keyed by `#` and its node, and stays out of the state.
        for (path, edit) in self.edits.iter().filter(|(p, _)| p.starts_with('/')) {
            set(&mut state, path, Value::String(edit.text.clone()));
        }
        state
    }

    /// The text field for `path`, starting from its value in the state.
    pub fn edit(&mut self, spec: &Spec, path: &str) -> &mut TextEdit {
        if !self.edits.contains_key(path) {
            let text = get(&self.state(spec), path)
                .map(display)
                .unwrap_or_default();
            let mut edit = TextEdit::default();
            edit.text = text;
            self.edits.insert(path.to_string(), edit);
        }
        self.edits.get_mut(path).expect("inserted above")
    }

    /// Writes `value` at `path`, as a checkbox does.
    pub fn write(&mut self, spec: &Spec, path: &str, value: Value) {
        let mut state = self.state(spec);
        set(&mut state, path, value);
        self.edits.remove(path);
        self.local = Some(state);
    }

    /// Fires a control's actions in order, and returns the params of each `submit`. The text
    /// fields start again from the state, since an action may have changed what they show.
    pub fn fire(&mut self, spec: &Spec, trigger: &Trigger) -> Vec<Value> {
        let mut state = self.state(spec);
        self.edits.clear();
        let mut sent = Vec::new();
        for binding in &trigger.bindings {
            if let Outcome::Send(params) = run(&mut state, binding, &trigger.scope, &mut self.ids) {
                sent.push(params);
            }
        }
        self.local = Some(state);
        sent
    }
}

/// What the captain did in a view, which goes to the chief of staff as a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiAction {
    /// The `app_ui` tool use that showed the view.
    pub tool: String,
    /// What the captain did, in a few words, such as "Pressed “Approve”".
    pub did: String,
    /// The `submit` params, resolved.
    pub params: Value,
    /// The value of every bound input, by its state path.
    pub inputs: Value,
}

impl UiAction {
    /// The message the chief of staff receives.
    pub fn message(&self) -> String {
        format!(
            "In the view you showed with {TOOL} (tool use {}), I {}. It sent the action \"submit\" \
             with the params {} and these input values: {}.",
            self.tool,
            lowercase_first(&self.did),
            self.params,
            self.inputs
        )
    }
}

fn lowercase_first(words: &str) -> String {
    let mut chars = words.chars();
    chars
        .next()
        .map(|c| c.to_lowercase().chain(chars).collect())
        .unwrap_or_default()
}

/// The value at each of `paths` in `state`, as one object.
pub fn values(state: &Value, paths: &[String]) -> Value {
    Value::Object(
        paths
            .iter()
            .map(|p| (p.clone(), get(state, p).cloned().unwrap_or(Value::Null)))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scripts/fixtures")
            .join(name);
        std::fs::read_to_string(path).unwrap()
    }

    fn keys(node: &Node, out: &mut Vec<String>) {
        out.push(node.key.clone());
        for child in &node.children {
            keys(child, out);
        }
    }

    fn drawn(input: &Value) -> Vec<String> {
        let (spec, _) = parse(input).unwrap();
        let mut out = Vec::new();
        if let Some(root) = tree(&spec, &spec.state).root {
            keys(&root, &mut out);
        }
        out
    }

    #[test]
    fn partial_json_keeps_only_what_is_complete() {
        assert_eq!(partial(""), Value::Null);
        assert_eq!(partial("{"), json!({}));
        assert_eq!(partial(r#"{"root":"ma"#), json!({}));
        assert_eq!(partial(r#"{"root":"main","#), json!({"root": "main"}));
        assert_eq!(partial(r#"{"a":[1,2,3"#), json!({"a": [1, 2]}));
        assert_eq!(partial(r#"{"a":[1,2,3]"#), json!({"a": [1, 2, 3]}));
        assert_eq!(partial(r#"{"a":{"b":tru"#), json!({"a": {}}));
        assert_eq!(partial(r#"{"a":"x\"y","b"#), json!({"a": "x\"y"}));
        assert_eq!(partial(r#"{"a" : "éé" , "b": nul"#), json!({"a": "éé"}));
        assert_eq!(partial("[[[[[[["), json!([[[[[[[]]]]]]]));
        assert_eq!(partial("not json"), Value::Null);
    }

    #[test]
    fn every_prefix_of_a_streamed_spec_parses_and_draws_more_as_it_arrives() {
        let whole = fixture("ui-release.json");
        let full = drawn(&serde_json::from_str(&whole).unwrap());
        let mut most = 0;
        for (end, _) in whole.char_indices() {
            let input = partial(&whole[..end]);
            let Ok((spec, _)) = parse(&input) else {
                continue;
            };
            let count = tree(&spec, &spec.state).root.map_or(0, |r| {
                let mut out = Vec::new();
                keys(&r, &mut out);
                out.len()
            });
            most = most.max(count);
            if end == whole.len() / 2 {
                assert!(count > 0 && count < full.len(), "{count} at the middle");
            }
        }
        assert_eq!(drawn(&partial(&whole)), full);
        assert_eq!(most, full.len());
    }

    #[test]
    fn unknown_props_and_types_are_dropped_or_drawn_as_placeholders() {
        let input: Value = serde_json::from_str(&fixture("ui-unknown.json")).unwrap();
        let (spec, warnings) = parse(&input).unwrap();
        assert!(!spec.elements["main"].props.contains_key("glow"));
        assert!(warnings.iter().any(|w| w.contains("\"BarChart\"")));
        assert!(warnings.iter().any(|w| w.contains("no prop \"glow\"")));
        let root = tree(&spec, &spec.state).root.unwrap();
        assert_eq!(root.children[1].what, What::Unknown("BarChart".into()));
        // A literal of the wrong kind is dropped too; an expression is kept.
        let input = json!({"root": "p", "elements": {"p": {"type": "Progress",
            "props": {"value": "lots", "label": {"$state": "/l"}}}}});
        let (spec, _) = parse(&input).unwrap();
        assert_eq!(
            spec.elements["p"].props.keys().collect::<Vec<_>>(),
            ["label"]
        );
    }

    #[test]
    fn over_the_limits_is_refused() {
        let big = json!({"root": "t", "elements": {"t": {"type": "Text",
            "props": {"text": "x".repeat(MAX_BYTES)}}}});
        assert!(parse(&big).unwrap_err().contains("KB limit"));
        let mut deep = json!("leaf");
        for _ in 0..MAX_DEPTH {
            deep = json!([deep]);
        }
        assert!(
            parse(&json!({"state": deep}))
                .unwrap_err()
                .contains("deeper")
        );
        assert!(
            check(&json!({"elements": {}}))
                .unwrap_err()
                .contains("no \"root\"")
        );
        assert!(
            check(&json!({"root": "x", "elements": {}}))
                .unwrap_err()
                .contains("not defined")
        );
    }

    #[test]
    fn a_cycle_or_a_huge_repeat_stops_instead_of_looping() {
        let cycle = json!({"root": "a", "elements": {
            "a": {"type": "Stack", "children": ["b"]},
            "b": {"type": "Stack", "children": ["a"]}}});
        assert_eq!(drawn(&cycle), ["a", "b"]);
        let items: Vec<Value> = (0..1000).map(|i| json!(i)).collect();
        let huge = json!({"root": "l", "state": {"items": items}, "elements": {
            "l": {"type": "Stack", "repeat": {"statePath": "/items"}, "children": ["t"]},
            "t": {"type": "Text", "props": {"text": {"$item": ""}}}}});
        let (spec, _) = parse(&huge).unwrap();
        let tree = tree(&spec, &spec.state);
        assert!(tree.cut);
        assert_eq!(tree.root.unwrap().children.len(), MAX_NODES - 1);
    }

    #[test]
    fn expressions_read_state_items_and_conditions() {
        let state = json!({"name": "Ada", "n": 3, "todos": [{"title": "a", "done": true}]});
        let item = Scope {
            item: Some("/todos/0".into()),
            index: Some(0),
            ..Scope::default()
        };
        let r = |v: Value| resolve(&v, &state, &item);
        assert_eq!(r(json!({"$state": "/name"})), "Ada");
        assert_eq!(r(json!({"$item": "title"})), "a");
        assert_eq!(r(json!({"$index": true})), 0);
        assert_eq!(
            r(json!({"$template": "Hi ${/name}, ${title} (${n})"})),
            "Hi Ada, a (3)"
        );
        assert_eq!(
            r(json!({"$cond": {"$state": "/n", "gt": 2}, "$then": "big", "$else": "small"})),
            "big"
        );
        let v = |c: Value| visible(&c, &state, &item);
        assert!(v(json!({"$item": "done"})));
        assert!(!v(json!({"$item": "done", "not": true})));
        assert!(v(json!({"$state": "/n", "lte": {"$state": "/n"}})));
        assert!(v(json!({"$or": [false, {"$state": "/name", "eq": "Ada"}]})));
        assert!(!v(json!([true, {"$state": "/missing"}])));
        assert!(v(json!({"$index": true, "eq": 0})));
    }

    #[test]
    fn actions_change_the_state_and_only_submit_sends() {
        let input: Value = serde_json::from_str(&fixture("ui-todos.json")).unwrap();
        let (spec, _) = parse(&input).unwrap();
        let mut live = Live::default();
        live.edit(&spec, "/draft").text = "Ship".into();
        let add = Trigger {
            bindings: spec.elements["add"].on["press"].clone(),
            scope: Scope::default(),
        };
        assert!(live.fire(&spec, &add).is_empty());
        let state = live.state(&spec);
        assert_eq!(
            state["todos"][3],
            json!({"id": "id-1", "title": "Ship", "done": false})
        );
        assert_eq!(state["draft"], "");
        let remove = Trigger {
            bindings: spec.elements["remove"].on["press"].clone(),
            scope: Scope {
                item: Some("/todos/0".into()),
                index: Some(0),
                ..Scope::default()
            },
        };
        live.fire(&spec, &remove);
        live.write(&spec, "/todos/0/done", json!(true));
        let state = live.state(&spec);
        assert_eq!(state["todos"].as_array().unwrap().len(), 3);
        assert_eq!(state["todos"][0]["title"], "Write the changelog");
        assert_eq!(state["todos"][0]["done"], true);
        let send = Trigger {
            bindings: spec.elements["send"].on["press"].clone(),
            scope: Scope::default(),
        };
        let sent = live.fire(&spec, &send);
        assert_eq!(sent, [json!({"list": state["todos"]})]);
        let mut state = json!({});
        let unknown = Binding {
            action: "rm -rf".into(),
            params: Map::new(),
        };
        assert_eq!(
            run(&mut state, &unknown, &Scope::default(), &mut 0),
            Outcome::Unknown("rm -rf".into())
        );
        assert_eq!(state, json!({}));
    }

    #[test]
    fn the_description_names_every_component_and_action() {
        let text = description();
        for c in CATALOG {
            assert!(text.contains(&format!("- {}: {{", c.name)), "{}", c.name);
        }
        for a in ACTIONS {
            assert!(text.contains(&format!("- {} ", a.name)), "{}", a.name);
        }
        for fixture_name in ["ui-release.json", "ui-todos.json"] {
            let input: Value = serde_json::from_str(&fixture(fixture_name)).unwrap();
            assert_eq!(check(&input), Ok(vec![]), "{fixture_name}");
        }
    }
}
