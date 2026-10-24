# GUI spike (3a)

Throwaway code that answers one question before Phase 3 builds the GUI:
can Dioxus give writers the editor the [design principles](../../docs/roadmap.md#phase-3-desktop-gui)
ask for, on the desktop and in the browser? It shows one made-up session
(no server, no keys) with three editor forms to compare. What it found is
in [FINDINGS.md](FINDINGS.md).

It is its own Cargo workspace, outside the main one, so its dependencies
stay out of the project's lockfile, CI and `cargo vet` until 3b decides
what to adopt. `.github/workflows/spike.yml` builds it on every platform
when it changes and keeps the builds as artifacts.

## Running it

**Desktop** (Windows, macOS or Linux), from this folder:

```sh
cargo run --release --features desktop
```

Linux needs WebKitGTK's development files first (Debian and Ubuntu:
`sudo apt install libwebkit2gtk-4.1-dev libxdo-dev`). Windows uses
WebView2, which Windows 10 and 11 include.

**Browser**: either download the `quill-gui-spike-web` artifact from the
"GUI spike" workflow run and serve the folder (`python -m http.server`
inside it, then open http://localhost:8000), or build it:

```sh
cargo install dioxus-cli --version "^0.7" --locked
dx serve --platform web
```

`QUILL_SPIKE_DEBUG=1` makes the desktop build print measurements once a
second (how far the live editor trails the text area, the viewport), as
used for the findings.

## What to try

The bar at the top switches between the editor forms:

- **Write + preview**: a plain text area, with the rendered note beside it
  (or toggled when narrow).
- **Live**: the Markdown styled as you type, in place.
- **Rich**: formatted in place, without Markdown marks.

Ctrl+B, Ctrl+I and Ctrl+E (Cmd on macOS) wrap the selection in bold,
italic and code; Ctrl+Enter sends. Images can be added with the button or
dropped on the editor. Drafts are kept per channel. **Focus** hides
everything but the editor. **Load a long draft** puts about 20,000 words
in the editor, to try typing in a long note.
