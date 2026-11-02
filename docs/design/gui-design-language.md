# Quill's design language for the desktop app

This is how every screen in the desktop app looks, behaves and speaks. The web client (Phase 4) inherits it. It turns the Phase 3 design principles (`docs/roadmap.md`: friendly first, responsive and alive, a pleasant place to write) into rules a screen can be checked against. The board mockups in `docs/design/board-layout/` set the layouts the review chose; this sets the parts they're built from.

The tokens and components live in `crates/quill-ui/src/style.css`, tokens first. The design reference page shows every component in every state, in light and dark: open Settings with advanced mode on, then "Design reference". Use it for hand checks with `scripts/dev-gui.py`.

## The idea: quiet chrome, the writing is the subject

Quill is a place to write with someone. Everything around the writing (navigation, settings, keys, the board's lists) stays calm and gets out of the way. The writing itself gets the type and the space.

- **Two voices of type.** A plain system sans-serif for the app's chrome, and a book serif for anything written: prompts, pitches, session notes, and the headings that name them. A glance tells you which is which.
- **One accent, used sparingly.** A deep teal marks the one thing to do next (the primary button), where you are (the current section), and links. It never decorates.
- **Flat, with hairlines.** Surfaces are separated by a hairline or a change of tone, not shadows. Cards appear only where something stands alone: a first-run question, or one item in a list.
- **Nothing waits silently.** Every action shows it's under way, then says what happened, in the place it happened.

## Tokens

All values are CSS custom properties on `:root`, redefined for dark. A screen never uses a raw colour, size or spacing value.

### Colour roles

| Token | Light | Dark | Use |
|---|---|---|---|
| `--paper` | `#eef1f4` | `#12161c` | The page behind everything |
| `--surface` | `#ffffff` | `#1a2029` | Cards, fields, panels, the sidebar |
| `--ink` | `#1c2430` | `#e4e8ee` | Text |
| `--muted` | `#5f6a7c` | `#9aa6b8` | Secondary text: hints, meta lines, quiet buttons |
| `--line` | `#d9dee5` | `#2c3440` | Hairlines between things. Decorative only |
| `--edge` | `#7d8899` | `#6b7687` | The border that shows where a control is (fields, selects, secondary buttons, checkboxes) |
| `--accent` | `#0e6c78` | `#5cc3cc` | The next action, the current place, links, focus |
| `--accent-ink` | `#ffffff` | `#0d1a1c` | Text on `--accent` |
| `--accent-soft` | `#d9eef0` | `#173a3f` | Selected and current backgrounds, nudges |
| `--warn` / `--warn-soft` | `#9a4b00` / `#fbeedd` | `#f0b36a` / `#3a2a14` | Something needs attention but nothing is wrong |
| `--bad` | `#a3242d` | `#f08a90` | Something is wrong, or an action can't be undone |

**Contrast** (WCAG 2.2 AA, checked by a unit test that reads these values from the stylesheet):
- Text in `--ink`, `--muted`, `--accent`, `--warn` and `--bad` reaches 4.5:1 on `--paper`, `--surface` and its soft background, in both themes.
- `--edge` and the focus ring reach 3:1 against `--paper` and `--surface`, as WCAG asks of a control's boundary.
- `--line` doesn't, and so it never marks a control's edge on its own.

### Type

| Token | Value | Use |
|---|---|---|
| `--ui` | system sans-serif stack | All chrome: labels, buttons, menus, hints, lists of settings |
| `--write` | book serif stack (Iowan Old Style, Charter, Sitka Text, Cambria, Georgia) | Prompts, pitches, notes, Markdown, and the headings that name written things or pages |
| `--mono` | system monospace | Codes people copy or compare (link words, fingerprints) and advanced-mode details |

The scale has six steps. A screen uses no other size.

| Token | Size | Line height | Use |
|---|---|---|---|
| `--text-xs` | 0.75rem | 1.4 | Badges, counts, chip notes |
| `--text-sm` | 0.875rem | 1.45 | Hints, meta lines, labels, table cells |
| `--text-base` | 1rem | 1.5 | Body text, controls |
| `--text-read` | 1.0625rem | 1.6 | Written text (serif) |
| `--text-lg` | 1.25rem | 1.3 | Section headings (`h2`) |
| `--text-xl` | 1.625rem | 1.25 | Page titles (`h1`) and a prompt's title in the reader |

Weights are 400 and 600 only. Headings are 600. Labels are 600 at `--text-sm`. Nothing is set in capitals, and no word is italicised or coloured for emphasis inside a heading.

Line length stays under 75 characters for anything read at length. Written text gets `--text-read` and 1.6 line height.

### Spacing

One scale, based on 4px. Padding, gaps and margins use only these.

| Token | Value |
|---|---|
| `--space-1` | 0.25rem (4px) |
| `--space-2` | 0.5rem (8px) |
| `--space-3` | 0.75rem (12px) |
| `--space-4` | 1rem (16px) |
| `--space-5` | 1.5rem (24px) |
| `--space-6` | 2rem (32px) |
| `--space-7` | 3rem (48px) |

Inside a component the gaps are `--space-1` to `--space-3`. Between components they're `--space-4`. Between sections they're `--space-5` or `--space-6`.

### Shape, edges, depth and motion

- **Radii:** `--radius-s` (6px) for controls and chips' neighbours; `--radius-m` (10px) for cards, panels and list items; `--radius-pill` (999px) for chips, tags and badges. Nothing else is rounded.
- **Borders:** 1px. A hairline (`--line`) between things; an edge (`--edge`) around controls.
- **Depth:** no shadows, except a single `--shadow-pop` under anything that floats above the page (none yet).
- **Focus:** `--focus`, a 2px `--accent` outline offset by 2px, on `:focus-visible` only. Never removed.
- **Motion:** `--quick` (120ms) for colour changes on hover and press. Movement only answers an action: something opening, closing or arriving. Under `prefers-reduced-motion: reduce` there's no motion at all.

### Sizes

- **Controls** (buttons, fields, selects) are at least `--control` (2.25rem, 36px) tall. Compact controls (segmented choices, chips' remove buttons, the letter strip) are at least `--control-compact` (1.75rem, 28px). WCAG 2.2's 24px minimum is a floor, not a target.
- **Widths:**
  - `--width-read` (42rem) for pages of reading and settings;
  - `--width-form` (34rem) for a lone form or first-run card;
  - full width for the board's two panes and the filter builder.

### Breakpoints

| Name | Width | What changes |
|---|---|---|
| narrow | ≤ 760px | The sidebar becomes a top bar; two panes become one, with a way back; side panels drop below |
| mid | ≤ 1150px | Three-column layouts (the filter builder) become two |
| small | ≤ 480px | Grids of two or three (link words, field rows) become one or two |

## Layout

- **The frame:** a sidebar (sections, and the server and persona switchers), the page, and a status line at the foot.
- **The page:**
  - its content starts at the top left, inside a gutter of `clamp(--space-4, 4vw, --space-7)`;
  - reading and settings pages keep to `--width-read`;
  - the board and filter builder use the full width.
- **The page header:** every page opens with one, holding the page title (`h1`) and the page's actions.
  - The title sits on the left, and actions follow on the right, aligned to the title's baseline.
  - On narrow windows the actions wrap below the title.
  - The primary action is last.
  - Search and filters sit in a tool row under the header, not in it.
- **Sections:** an `h2` and its content, separated from the next by `--space-6`. A section that groups settings is a `fieldset` with a `legend`.
- **Lists of things:**
  - Rows: a hairline between rows, no card each, the whole row a button when it opens something. The board's prompts work this way.
  - Items: a card each with its own actions, for short lists. Personas, devices and notices work this way.
- **Two panes** (list and reader):
  - The list keeps its own scroll; the reader stays in place.
  - On narrow windows one pane shows at a time, with "‹ Back to …" at the top of the second.
- **Side panels** (a filter's summary, the post form's preview and tags) sit on the right. On narrow and mid windows they drop below the main content.

## Components

Each is a class in `style.css` and appears on the design reference page in every state it has. The states are: rest, hover, focus, pressed, disabled, busy, and error where it applies.

### Buttons

| Class | Looks | Use |
|---|---|---|
| `.primary` | Filled `--accent`, `--accent-ink` text, 600 | The one action a screen is for. At most one per page header, form or card |
| `.secondary` | `--edge` border, no fill | Other actions |
| `.quiet` | No border or fill, `--muted` text | Cancel, Not now, Back, and lesser actions beside a primary |
| `.danger` | `--bad` border and text; filled `--bad` once it's the confirming button | Actions that can't be undone |
| `.link` | Underlined `--accent` text | An action inside running text |

- **Label:** a button's label names exactly what it does: "Post prompt", "Delete @ink_fox", "Link this computer". "OK", "Submit", "Yes" and "Confirm" are never used.
- **Busy:** a busy button is disabled, and its label says what's happening ("Posting…", "Linking…").
- **Disabled:** a disabled button says why, nearby: the form's "Needs …" line, or the field's hint.
- **Sizes:** buttons are `--control` tall, with `--space-2` × `--space-4` padding.
- **Pressed:** pressed buttons darken by one step (`filter: brightness(0.92)`).
- **In a row:** a row of buttons (`.actions`) has `--space-2` gaps and wraps.

### Fields

Single-line (`Field`), multi-line (`Area`), select, checkbox, radio, and segmented choice.

- **Labels:** every field has a visible label above it, in `--text-sm` 600. A placeholder is an example, never the label.
- **Hint:** a hint line sits under every field, and it always takes its space, so the layout doesn't jump. At rest it states the rule. While typing it tracks what's typed (see "Limits are said while typing"). When something's wrong it says what, and turns `--bad`.
- **Error:** a field in error has an `--bad` edge, `aria-invalid="true"`, and its hint tied to it by `aria-describedby`.
- **Edges and sizes:** fields have an `--edge` border on `--surface`, are `--control` tall, and have `--space-2` × `--space-3` padding. Multi-line fields use `--write` for anything that will be read as writing.
- **Two fields in a row** (`.field-row`) become one column on narrow windows.
- **Choices:** checkboxes and radios sit before their label, which is clickable. A group of them is a `fieldset` with a `legend`.
- **Segmented choices** (`.segmented`, as Any / Has / Hide) are for two to four exclusive options set in place. Each is a button with `aria-pressed`.
- **Text input:** fields keep the spike's rules (`spikes/gui/FINDINGS.md`):
  - uncontrolled inputs with an `initial_value`;
  - remounting by key to replace the text;
  - spellcheck on for prose and off for names;
  - `writingsuggestions="false"`.

### Surfaces

- **Card** (`.card`): one question on its own, as in first run. It's centred, on `--surface`, with a `--line` border, `--radius-m` and `--space-6` padding.
- **Item** (`.item`): one thing in a short list, with its actions.
- **Panel** (`.panel`): a section of a page set apart (a side panel, "Link a new computer"). It has the same look as an item.

### Telling people things

| Class | Use |
|---|---|
| `.nudge` | Something worth doing, with its button (link this computer, make a persona). `--accent-soft` |
| `.notice` | Something needs attention before going on (rules changed, a removal, a warning before an action that can't be undone). `--warn-soft` with a `--warn` left edge. `role="alert"` when it appears because of what the person just did |
| `.problem` (`ProblemLine`) | What went wrong, in `--bad`, then what to do. The raw error only in advanced mode |
| `.hint` with `role="status"` | What just happened ("Prompt posted.", "Link copied.") |
| `.status` (the status line) | The connection, and offline with "Try now" |

**Error codes (#76).** Every problem has a code, such as `Q-3001` (`docs/error-codes.md`). Advanced mode shows it beside the words (`app::Code`, `.code`): in problem lines, notices, the status line when offline or signed out, and inline "can't show this" reasons. The default view never shows codes; the words still say what happened and what to do. A code is for reporting, so it sits after the sentence, small and muted, never in its place.

### Small things

- **Chips** (`.chip`): a tag. They're pill-shaped, `--text-sm`, and coloured by the tag type's own colour. A chip that can be removed has a "×" button named "Remove {type}: {name}".
- **Tags** (`.tag`): a short status beside a name ("This computer", "Keys on another device"). They're pill-shaped and `--text-xs`, in `--warn-soft`.
- **Badges** (`.badge`): a count that needs the person (unread notices). They're pill-shaped and `--text-xs`, in `--bad`. The count's meaning is spoken too ("2 unread"), in `.sr-only` text.
- **Slots** (`.slot`): something still required, as a dashed `--muted` outline ("Length: pick 1").

### Empty, loading and done

- **Empty:** an empty list says why it's empty and what to do ("No prompts on the board yet." beside "Post a prompt"; "No prompts match. Try fewer filters, or another search.").
- **Loading:** loading says what's loading ("Loading prompts…", "Looking for the new computer…"). Loading that takes steps shows them (getting a persona ready).
- **The end of a list:** it's said only once it's reached ("That's every prompt.").

## Patterns

### Limits are said while typing

Any limit known ahead of time is shown while the person types, never only after they press the button. That covers lengths, counts, required parts, allowed characters, and per-type tag limits.

- **Hint:** the field's hint states the rule while empty ("Up to 200 characters."), then how much is used ("12 of 200 characters."), then what's wrong once over ("201 of 200 characters: 1 too many.").
- **Action:** the action waits, disabled, and the line under it says what it's waiting for ("Needs a title, 1 length tag before posting.").
- **Code:** `quill-ui`'s `checks` module holds these rules, counted as the server counts.

### One plain question

An action that can't be undone asks once, in place, never in a modal dialog.

- The question appears where the button was. It's a `.notice` that names the thing and says exactly what will happen ("Remove Study desktop? … This can't be undone.").
- It has two buttons: the `.danger` one names the action ("Remove Study desktop"), and the `.quiet` one keeps things as they are ("Keep it").
- Anything that can be undone doesn't ask; it's undone instead.

### Feedback

Every action:
- shows it's under way at once (a busy button);
- then says what happened, where it happened (a status line, or the result in place);
- or, if it failed, says why and what to do, keeps everything typed, and offers to try again.

Offline is a state, not an error. The page keeps what it has, the status line says the server can't be reached, and actions that need the server wait or say so.

### Lists that change

A list that's replaced (Refresh, a new search, a filter) starts at its top. Loading more keeps the place. New items found in the background are offered ("2 new prompts. Show them") rather than pushed in under the reader.

### A changed key

Quill keeps the identity key it first sees for each writer (core's pins). When a writer's key isn't the one kept, nothing goes to them and nothing from them is read until the person says so. This happens before a request is sent, before a pitch is shown, and, from 3f, in sessions. The app stops in place with a `.panel` (`requests::KeyCheck`), never a dialog:

- **What happened**, plainly: "@wren's identity key changed."
- **Why it usually happens, and why it matters:** they started over on a new computer without their old keys; or someone is using the server to pass as them. Quill's promise is that the server can't read what's written, and a server that swapped a key could. This is the one place that promise depends on the writer.
- **What to do:** compare the 12 words with the other writer some other way than Quill's requests. Both apps derive them from both personas' keys, so they match only if neither key was swapped (core's identity number, 2j). A fingerprint would ask people to compare 32 hex digits; words are easier to read aloud.
- **Where the other writer finds them (#72):** the stop names the place. "Compare words with @name" sits wherever two writers meet before a session (the reader, on someone else's prompt; each request in a queue), and the Personas page finds them for any handle, so they can be read out even when nothing brings the two together yet. The CLI prints them with `quill keys words`. Without this, the stop's advice couldn't be followed until a shared session existed.
- **Two buttons:** "The words match: carry on" pins the new key and does what was asked (sends, or shows the pitch). "Not now" leaves it, and keeps everything typed. A request from a writer whose key changed can still be declined unread, since declining tells the server nothing about the pitch.
- **Only the compared key is trusted.** Accepting names the new key's fingerprint, and core refuses if the published key changed again meanwhile, so the screen shows new words rather than trusting a key nobody saw.

## Writing

The app speaks plainly, as one considerate person to another.

- **Plain words:** say what a thing does, in the reader's terms: "your devices", not "the MLS group"; "the keys for @ink_fox", not "identity key material". Ids, keys and log positions appear only in advanced mode.
- **Sentence case** everywhere: titles, buttons, labels. Full stops end sentences in hints and messages, not labels or buttons.
- **Buttons** name their action, and the same action keeps its name through the flow: "Post prompt" leads to "Prompt posted.".
- **Errors** say what happened and what to do, without apology and without blame: "Those words don't match the computer that's waiting. Check each one against its screen."
- **Empty states** say why the screen is empty and what to do about it.
- **Security** that needs a person is said in words, with the stakes: what happened, why it matters, and what to do. It never says only "Invalid" or "Error".
- **No filler:** no "please", "simply" or "just", no exclamation marks, and no emoji in the interface.

## Accessibility

These rules are checked by the interaction-test driver (`Driver::accessibility_problems`) on every screen the tests visit. A screen that breaks one fails its test.

- Every button has an accessible name: its text, or an `aria-label` when it's an icon such as "×".
- Every field has a label: a `label for` its id, or an `aria-label` where a visible label would repeat the heading (the board's search).
- A field marked `aria-invalid="true"` names its hint with `aria-describedby`, and the hint says what's wrong.
- Ids are unique on a screen.
- Each screen has exactly one `h1`.
- Images that carry meaning (the link's QR code) have `role="img"` and a label; decorative ones are hidden from screen readers.
- Things that change on their own, or after an action, are announced: status lines use `role="status"`, and urgent ones `role="alert"`.
- Pressed and current states are spoken: `aria-pressed` for toggles and segmented choices, `aria-current` for the current section and the open row.

Beyond what the driver checks:
- **Keyboard:** everything works from the keyboard, in reading order, with focus always visible.
- **Focus:** a screen's first field takes focus when the screen opens to be filled in.
- **Contrast:** checked by the unit test on the tokens.
- **Sizes:** controls keep the sizes above.
- **Hand checks:** the webview's own behaviour (focus, input methods, screen readers) stays a hand check.
