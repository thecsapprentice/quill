# Automated moderation spike (#43)

Throwaway code for one question before #43 builds the framework: which
language should admins write moderation scripts in? It runs the same four
sample scripts through five candidates, each with the same `text` API
registered, and checks the answers, the timings, the sandbox and what an
admin sees when they get something wrong. What it found is in
[FINDINGS.md](FINDINGS.md).

It's its own Cargo workspace, outside the main one, so the engines stay
out of the project's lockfile, CI and `cargo vet` until the decision is
made.

## The candidates

| Folder | Engine | Crate |
| --- | --- | --- |
| [`scripts/toml`](scripts/toml) | Rule files: conditions in TOML, no language | `toml` |
| [`scripts/rhai`](scripts/rhai) | Rhai | `rhai` |
| [`scripts/luau`](scripts/luau) | Luau (Roblox's Lua) | `mlua` |
| [`scripts/star`](scripts/star) | Starlark (Python-like, from Bazel) | `starlark` |
| [`scripts/js`](scripts/js) | JavaScript on QuickJS | `rquickjs` |
| [`scripts/wasm`](scripts/wasm) | WebAssembly, written in Rust | not run; see FINDINGS |

## The samples

Each folder has the same four scripts, so they can be read side by side:

- `words`: refuse a list of words and phrases, through spacing
  ("s p a m"), stretched letters and lookalike letters
- `links`: refuse links to outside sites, and phone numbers (a regex)
- `lookalikes`: flag words that mix alphabets ("pаypal" with a Cyrillic а)
- `newcomer`: limit how fast new accounts post, and flag near copies

`limits/` holds hostile scripts (an endless loop, a 64 MB string, reading a
file, a regex that hangs backtracking engines), and `mistakes/` holds a
misspelled field and a missing bracket.

## Running it

From this folder:

```sh
cargo run --release              # the samples, checked and timed
cargo run --release -- limits    # the hostile scripts
cargo run --release -- mistakes  # what an admin sees for each mistake
cargo test                       # the text API on its own
sh measure-size.sh               # what each engine adds to the binary
```

Add a folder name (`rhai`, `luau`, `star`, `js`, `toml`) to run one engine.
`limits` exits with an error while Starlark's 64 MB string gets through;
that's a finding, not a bug in the harness. Luau and QuickJS need a C/C++
compiler (MSVC, GCC or Clang).

## Layout

- `src/text.rs`: the `text` API, plain Rust, shared by every engine
- `src/context.rs`: what a script is given (`post`), what it returns, and
  the sample posts with the answer each sample should give
- `src/engines/`: one adapter per engine, behind one trait
- `src/main.rs`: the harness
