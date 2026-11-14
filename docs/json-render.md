# Views in the chat (json-render)

The chief of staff can show the captain a small interactive view inline in the chat: a card with
a table and two buttons, a checklist to tick, a form to fill in. It describes the view as a
[json-render](https://json-render.dev/) spec and passes it to the app's `app_ui` tool. The app
draws the spec with its own Rune views; no web view, React code or network is involved. A press
or a submit comes back to the chief of staff as a chat message.

This page records the json-render format as read on 2026-11-13, what the app implements of it,
and what it leaves out. Sources: the json-render site and its repository
(`vercel-labs/json-render` at commit `f5c1240`, `@json-render/core` 0.22.0), chiefly
`packages/core/src/types.ts`, `props.ts`, `visibility.ts`, `actions.ts`, `schema.ts` (the
prompt it generates), `packages/react/src/schema.ts` (the built-in actions),
`packages/shadcn/src/catalog.ts` (the component props this catalog follows), and the docs pages
Specs, Streaming, Data binding and Visibility.

## The json-render format

A spec is a flat JSON object:

```json
{
  "root": "card",
  "elements": {
    "card": {"type": "Card", "props": {"title": "Hello"}, "children": ["text"]},
    "text": {"type": "Text", "props": {"text": {"$state": "/greeting"}}, "children": []}
  },
  "state": {"greeting": "Welcome"}
}
```

- `root` names the element to draw first, and `elements` maps each id to an element.
- An element has a `type`, a component name from the developer's **catalog**; `props`;
  `children`, a list of element ids; and optionally `visible`, `repeat` and `on`.
- `state` seeds the view's state model. Paths into it are JSON Pointers (RFC 6901), such as
  `/todos/0/title`.
- The model is given the catalog, so it can only name components and actions the catalog
  defines. json-render's own `catalog.prompt()` lists each component with its props, children
  and events, then the actions.

### Expressions

Any prop can be an expression instead of a literal:

| Expression | Value |
| --- | --- |
| `{"$state": "/path"}` | The state at the path |
| `{"$bindState": "/path"}` | The same, and the component writes back to the path (two-way binding) |
| `{"$item": "field"}` | A field of the current repeat item (`""` for the whole item) |
| `{"$index": true}` | The current repeat index |
| `{"$bindItem": "field"}` | Two-way binding to a field of the current repeat item |
| `{"$cond": c, "$then": a, "$else": b}` | `a` when the condition `c` holds, else `b` |
| `{"$template": "Hi ${/name}"}` | The text with `${/path}` read from state and `${field}` from the repeat item, then state |
| `{"$computed": "fn", "args": {...}}` | A function the developer registered |

### Repeat and visibility

`"repeat": {"statePath": "/items", "key": "id"}` draws the element once and its children once per
item of the array at `statePath`; inside another repeat, `statePath` may be `{"$item": "field"}`.
A repeat whose `visible` condition reads `$item` or `$index` filters its items.

`visible` is a condition: `true` or `false`; `{"$state": path}`, `{"$item": field}` or
`{"$index": true}` with at most one of `eq`, `neq`, `gt`, `gte`, `lt`, `lte` (no operator tests
truthiness) and an optional `"not": true`; a list (all must hold); `{"$and": [...]}`; or
`{"$or": [...]}`.

### Actions

`"on": {"press": {"action": "name", "params": {...}}}` binds an event of the component to an
action, or to a list of actions run in order. Params may be expressions; in params `$item`
resolves to the item's absolute state path, so it can be a `statePath`. The React renderer has
four built-in actions (`setState`, `pushState` with `"$id"` for new ids and `clearStatePath`,
`removeState`, `validateForm`); any other action is the developer's, declared in the catalog with
a handler. A binding may also carry `confirm`, `onSuccess` and `onError`.

### Streaming

json-render streams a spec as **SpecStream**: JSON Lines, each an RFC 6902 JSON Patch operation
(`{"op":"add","path":"/elements/card","value":{...}}`) that builds the spec up, and the renderer
draws what has arrived.

## What the app implements

**The tool.** The MCP server in `src/bridge.rs` has a second tool, `app_ui`. Its input is the spec
itself (`root`, `elements`, `state`), and its description is the whole catalog in the shape of
json-render's own prompt, generated from the catalog in `src/ui.rs`. The call checks the spec and
answers either an error the model can fix (over a limit, no root, a root that is not defined) or
"shown", followed by a line for each thing that was left out (an unknown component, prop or
action, a prop of the wrong kind, a missing child). It changes nothing in the app: the chat
draws the view from the tool use itself, which is a tool-call handler of its own kind
(`Handler::Ui` in `src/tools.rs`).

**Streaming.** The app does not use SpecStream. A tool's input already streams: with
`--include-partial-messages`, Claude Code reports a tool use when it starts and its input as JSON
fragments (`input_json_delta`). For `app_ui` only, `src/claude.rs` shows the view as soon as the
tool use starts and, after each fragment, reads the JSON so far with `ui::partial`, which keeps
every complete value, drops an unfinished string, number or key, and closes what is open. The
view draws what is complete: an element appears once its type has arrived, a child that is not
defined yet is skipped, and nothing half-formed can panic. The whole input replaces the streamed
one when the tool use is complete.

**The catalog.** The names and props follow json-render's shadcn catalog where it has the same
component. `src/ui.rs` holds the catalog once; the renderer, the check and the description read
it.

| Component | Props (`?` optional) | Children | Events |
| --- | --- | --- | --- |
| `Stack` | `direction?: vertical\|horizontal`, `gap?: none\|sm\|md\|lg` | yes | |
| `Card` | `title?`, `description?` | yes | |
| `Separator` | | | |
| `Heading` | `text`, `level?: h1\|h2\|h3\|h4` | | |
| `Text` | `text`, `variant?: body\|caption\|muted\|code` | | |
| `Badge` | `text`, `variant?: default\|secondary\|destructive\|outline` | | |
| `Alert` | `title`, `message?`, `type?: info\|success\|warning\|error` | | |
| `Progress` | `value`, `max?` (100), `label?` | | |
| `Image` | `src` (an absolute path), `alt?` | | |
| `Table` | `columns: string[]`, `rows: string[][]`, `caption?` | | |
| `Button` | `label`, `variant?: primary\|secondary\|danger`, `disabled?` | | `press` |
| `Input` | `label`, `placeholder?`, `value?` | | `submit` (Return) |
| `Checkbox` | `label`, `checked?` | | `change` |
| `Choice` | `question`, `options: {value, label, hint?}[]`, `recommended?` (an option's value), `multiple?`, `other?` (true) | | (sends its answer itself) |

| Action | Params | What it does |
| --- | --- | --- |
| `setState` | `statePath`, `value` | Sets the value, in the view only |
| `pushState` | `statePath`, `value`, `clearStatePath?` | Appends the value (`"$id"` becomes a new id), then empties `clearStatePath`, in the view only |
| `removeState` | `statePath`, `index` | Removes the item, in the view only |
| `submit` | any | Sends the chief of staff a message (below) |

**Expressions, repeat and visibility** are implemented as above, except `$computed`.

**Actions go back as messages.** `submit` is the catalog's one custom action. When the captain
presses a button, submits an input or ticks a checkbox whose bindings include `submit`, the
chat shows what they did as their own message ("Pressed “Send the list”.") and the chief of
staff's session receives:

> In the view you showed with app_ui (tool use toolu_…), I pressed “Send the list”. It sent the
> action "submit" with the params {…} and these input values: {"/draft": "…", …}.

The params are resolved against the state at the moment of the press, after any earlier action in
the same list ran. The input values are the value of every path an `Input` or `Checkbox` binds.
The stub agent layer answers with the params it got, so the round trip runs in headless scenes
and tests.

**A question with a fixed set of answers is a `Choice`.** It draws the question, one row per
option with its hint, a **Recommended** mark on the option whose value is `recommended`, and an
**Other** field for an answer of the captain's own (Return sends it) unless `other` is false. In a
single choice a press on an option sends it at once; with `multiple`, presses pick and unpick and
**Send** sends the picks (it is disabled until something is picked or typed). It needs no `on`
binding: the answer goes back by itself, once. The chat shows it as the captain's own message
("Chose “yolo off”.", "Answered “…”."), the `Choice` shows the answer and takes no more presses,
and the chief of staff's session receives each option by its exact `value`, so an answer can never
be an ambiguous "No":

> In the view you showed with app_ui (tool use toolu_…), I answered your question “Merge
> authority for velvet-hammer is yolo on. Do you want it off?” with the option "yolo-off"
> (“yolo off”).

Several picks are listed in the order the view shows them, and the Other text follows as "my own
words, “…”". The tool's answer warns about an option without a string `value` and `label` (it is
left out), a `Choice` with fewer than two options, and a `recommended` that is no option's value.
What was answered lives in the running app; a restart shows the `Choice` unanswered again, while
the captain's message about it stays in the chat.

## Safety

A spec is untrusted model output.

- **Limits.** A spec larger than 64 KB as JSON, nested deeper than 24 levels, or with more than
  200 elements is refused, by the tool and by the view alike. The drawn tree stops at 400 nodes
  (counting repeat copies, with a note saying so) and 16 levels, and an element that contains
  itself is drawn once.
- **Nothing runs.** No action runs code or touches the file system. The three state actions
  change only the view's own state in memory, and `submit` only sends a chat message, which is
  the captain's own message to their chief of staff. An action the catalog lacks does nothing.
- **Unknown input.** A component the catalog lacks draws as a small box naming its type. Props the
  catalog does not define, and literal props of the wrong kind, are dropped.
- **Images.** An `Image` shows only a file that another tool use in this session already named (in
  its input's `file_path`, `path` or `notebook_path`, or a `file://` link in its result), that
  exists and is a png, jpg, gif, webp or heic file under 32 MB, the same rules as the image
  handler. An `app_ui` call never counts, so a view cannot name its own file. Any other path and every URL is refused with a line saying why, without being read.
- **No network.** The renderer makes no request.

## Left out

- **SpecStream** (JSON Patch lines), and the nested-tree spec form (`nestedToFlat`). The app
  streams the tool's own input instead; see Streaming above.
- **`$computed`** (no functions are registered), custom directives, `watch`, and named `slots`.
- **`validateForm`** and the `checks` and `validateOn` validation props.
- **`confirm`, `onSuccess` and `onError`** on bindings, and `preventDefault`.
- **`repeat.key`**: copies are keyed by index, which is enough for a view redrawn from state.
- **Components** beyond the catalog above, such as Grid, Tabs, Dialog, Select, Slider, Avatar
  and Link. `Choice` covers what Radio and a multi-select would.
- **Running json-render's React code** or a web view, code export, and a visual spec editor.

## Adding a component

Add it to `CATALOG` in `src/ui.rs` with its props, then a `What` variant and its case in
`ui::what`, then its drawing in `src/view/ui.rs`. The tool's description and the check pick it up
from the catalog. Add a scene and a test.

## Screenshots

Headless renders of the `view` scenes (see the README's verification section):

| Scene | Screenshot |
| --- | --- |
| A static view with every kind of display component | [view-static.png](screens/json-render/view-static.png) |
| The same view with a little over half of its spec streamed | [view-streaming.png](screens/json-render/view-streaming.png) |
| State bound to a checkbox and an input, and a repeat, after ticking, typing and adding | [view-state.png](screens/json-render/view-state.png) |
| A submit and the stub's reply | [view-action.png](screens/json-render/view-action.png) |
| The merge-authority question asked with a `Choice`, yolo off recommended | [choice-ask.png](screens/choice/choice-ask.png) |
| The same after choosing yolo off, and the stub's reply to the exact value | [choice-answered.png](screens/choice/choice-answered.png) |
| A `Choice` of several, two picked and an answer of the captain's own typed | [choice-multiple.png](screens/choice/choice-multiple.png) |
| An unknown component | [view-unknown.png](screens/json-render/view-unknown.png) |
| A spec over the element limit | [view-too-large.png](screens/json-render/view-too-large.png) |
