# #43 spike findings

What the spike found. Run on Windows 11 (x86-64, MSVC), Rust 1.97, release
build. The numbers come from `cargo run --release`, `-- limits`,
`-- mistakes` and `measure-size.sh` in this folder.

## Summary

- **Every engine ran all four samples correctly on the first try.** Speed
  doesn't separate them: a check takes 1 to 15 µs per post, and compiling a
  script takes under 1 ms (3 ms for Starlark's first). The server's limits
  will be about bad scripts, never normal ones.
- **The `text` API does the hard work, so the scripts look alike in every
  language.** Each sample is 5 to 15 lines whichever engine runs it. What
  separates the candidates is syntax, what happens when an admin makes a
  mistake, and how well the sandbox holds.
- **Rhai is still the best fit**: every hostile script was stopped by a
  built-in limit (operations, string size), not only by the clock; a typo'd
  field is an error once a strict setting is on; it's pure Rust and the
  easiest to embed.
- **Starlark is the runner-up**, and the one to pick if Python-like syntax
  matters most for admins. It had the clearest error messages ("did you mean
  `title`?") and has no `while` or recursion. But it can't cap memory (a
  64 MB string went through), its only way to stop a long script is a hook
  the crate marks as internal, and calling a script's function needed
  `unsafe`.
- **Luau held up well** but brings a C++ dependency and Lua's quirks (`..`,
  `~=`, `#list`, lists counted from 1).
- **JavaScript is the most widely known**, but a misspelled field is
  silently `undefined`, and its own backtracking regular expressions sit
  right next to ours.
- **WebAssembly is out for non-technical admins**: they'd need a compiler
  toolchain and a build step. `scripts/wasm` shows what they'd write.
- **Rule files (TOML) were the easiest to write and to get errors from**,
  but every new kind of condition needs server code. Rather than a second
  engine, the same ease can come from **templates**: the sample scripts with
  the lists and numbers at the top, which admins copy and edit (see "Ease of
  writing").

**Recommendation:** Rhai with the `text` API, shipped with a set of
templates. Decision still open (#43, "Still open" 4).

## 1. The samples

`cargo run --release`, mean per post over 200 runs of each of the six
sample posts; compile time is the first script each engine compiled.

| Engine | words | links | lookalikes | newcomer | Compile |
| --- | --- | --- | --- | --- | --- |
| Rule file | 9.0 µs | 0.6 µs | 2.4 µs | 0.3 µs | 0.02–0.7 ms |
| Rhai | 11.3 µs | 2.8 µs | 4.5 µs | 2.1 µs | 0.2–0.7 ms |
| Luau | 15.4 µs | 7.5 µs | 8.3 µs | 6.3 µs | 0.2–0.7 ms |
| Starlark | 10.7 µs | 3.3 µs | 4.8 µs | 3.6 µs | 0.08–3 ms |
| JavaScript | 13.4 µs | 5.4 µs | 6.3 µs | 4.4 µs | 0.2–0.8 ms |

All 24 script and post pairs gave the expected answer in every engine.
`words` is the slowest everywhere because it folds the post (normalizing,
squashing, skeletons) before matching; that's Rust time, the same for all.

## 2. Limits and the sandbox

`cargo run --release -- limits`, with a 100 ms wall-clock cap, 16 MB of
memory (where the engine can limit it) and Rhai's 1 million operations.

| | Endless loop | 64 MB string | Read a file | Backtracking regex |
| --- | --- | --- | --- | --- |
| Rhai | stopped by operation count, 23 ms | stopped by string cap, 1 ms | no such function | linear, 1 ms |
| Luau | stopped by clock, 100 ms | stopped by memory cap, 3 ms | `io` doesn't exist | linear, 0.2 ms |
| Starlark | `while` refused at upload; nested `range` stopped by clock, 100 ms | **not stopped** | no loader | linear, 0.2 ms |
| JavaScript | stopped by clock, 100 ms | stopped by memory cap, 0.3 ms | no `require` | ours linear; JS's own `/.../` backtracks, stopped by clock |
| Rule file | can't loop | can't build strings | nothing to call | linear |

Notes:

- **Rhai's operation count is the best limit**: it stops a runaway script
  at the same point every time, whatever the server's load. The others
  only have the clock, so a busy server stops scripts earlier.
- **Starlark has no memory limit at all.** A script that builds a large
  enough string or list can run the server out of memory before the clock
  stops it. Its clock check uses `before_stmt_for_dap`, which the crate
  documents as "not public API".
- **Luau's sandbox mode** makes the shared globals read-only, so one script
  can't replace `text.find_words` or `reject` for the others
  (`escape-globals.luau`).
- **QuickJS joins strings lazily**, so doubling a string 26 times allocated
  nothing; `"x".repeat(1 << 26)` was needed to reach the memory cap. Its
  interrupt does reach inside its own regex engine, so `/^(a+)+$/` didn't
  hang the server, but it still used the whole time budget.
- Rust's `regex` is linear-time for every pattern, which is why the `text`
  API's regex is safe on user text in every engine.

## 3. Mistakes

`cargo run --release -- mistakes`: a misspelled field (`post.titel`) and a
list missing its closing bracket.

| | Misspelled field | Missing bracket |
| --- | --- | --- |
| Rule file | refused at upload: unknown field `any_word`, with the list of valid ones | refused at upload, line 7, column 6 |
| Rhai | error when run: `Property not found: titel (line 1, position 45)` (needs `fail_on_invalid_map_property`; silent without it) | refused at upload: `Unexpected 'let' (line 5, position 1)` |
| Luau | error when run: `attempt to concatenate nil with string` at line 1 | refused at upload, line 5 |
| Starlark | error when run: `has no attribute titel, did you mean title?`, with the line underlined | refused at upload, line 5, underlined |
| JavaScript | **no error**: `post.titel` is `undefined`, and the check quietly runs on the wrong text | refused at upload, line 5 |

A field typo can only be found when the script runs, in every language.
The framework's dry run (#43 step 2) is where admins will see it, so the
dry run should run every script against a sample post on upload, not only
when asked.

## 4. Ease of writing

The four samples side by side are the best way to judge this:
[`scripts/`](scripts). What stood out:

- **Rule files** read like settings, not code. But conditions can only be
  combined with "and" (each "or" is another rule), and each new kind of
  condition (`links_except`, `similar_on_board_at_least`) had to be written
  in Rust first.
- **Starlark** looks like Python, which more non-programmers have met than
  any other language here. It needs a `def check(post):` around the script
  and cares about indentation.
- **Rhai** reads like JavaScript without the surprises: `if`, `for`,
  `return`, `let`. The odd parts were `text::matches` (two colons) and
  `!(link.host in allowed_sites)`.
- **Luau**: `..` to join text, `~=` for "not equal", `#found` for a
  count, `then` and `end`, and lists that start at 1. Regexes need `[[ ]]`
  strings to keep their backslashes.
- **JavaScript** is the most familiar to anyone who has touched a web page,
  and the scripts were the shortest. The silent `undefined` (above) is the
  cost.
- **WebAssembly**: the Rust in `scripts/wasm` isn't harder to read than the
  others, but the admin has to install Rust, add a target, build, and
  upload a binary, and redo that for every change.

In every language, the part an admin edits is the same: a list of words or
sites, a number, a message. Templates that put those at the top with a
comment (as the samples do) mean most admins never write logic at all,
which matters more than the choice of language. AI assistants know Python,
JavaScript and Lua far better than Rhai, but they'd still need our `text`
API and `post` fields, so the scripting reference has to carry that
either way.

## 5. Embedding effort

| | Adapter size | Notes |
| --- | --- | --- |
| Rhai | 94 lines | `serde` support built in; limits are one call each |
| Luau | 101 lines | `serde` support built in; sandbox, memory and interrupt are one call each |
| JavaScript | 127 lines | complex values cross as JSON strings through a JS prelude; the script is wrapped in a function so a top-level `return` works |
| Starlark | 148 lines | heap lifetimes were the hardest part; calling the script's frozen `check` needed `unsafe`; JSON objects had to be turned into structs by hand so scripts can write `post.author` |
| Rule file | 125 lines | every condition is Rust code |

## 6. Dependencies and size

`measure-size.sh` (release, LTO, stripped) and `cargo tree`. With no
engine the compiler drops the unused `text` API, so each total includes
it. The rule-file build is roughly the `text` API alone (its Unicode
tables, regex and Aho-Corasick), about 2.3 MB, and the last column takes
that off.

| Engine | Binary added | Engine alone, about | Extra crates | Native code |
| --- | --- | --- | --- | --- |
| Rule file | 2.4 MB | 0.1 MB | 6 | none |
| Rhai | 5.1 MB | 2.8 MB | 22 | none |
| Luau | 3.8 MB | 1.5 MB | 18 | C++ (vendored Luau) |
| Starlark | 6.8 MB | 4.5 MB | 166 | none |
| JavaScript | 3.3 MB | 1.0 MB | 7 | C (vendored QuickJS) |

Rhai is the second-largest, but pure Rust with few crates, so it fits
`cargo vet` and `cargo deny` without a C toolchain in the server build.
Starlark's 166 crates would be a large addition to `supply-chain/`.
Luau and QuickJS each vendor a native engine that `cargo vet` can't
review as Rust, and need a C or C++ compiler on every build machine.

## 7. WebAssembly

Not run. `wasmtime` with fuel metering would give the strongest isolation
and a deterministic limit like Rhai's, and the guest can bring its own
libraries. But the admin writes, builds and uploads compiled code, which
rules it out for the admins #43 is for. It could come back later as an
option for admins who want it, behind the same hooks, context and actions.

## What's left for #43

- Record the decision in the roadmap and the architecture document.
- The `text` API in `src/text.rs` is a working start: regex (cached,
  linear, size-capped), word lists (Aho-Corasick over folded text), NFKC
  and accent stripping, UTS #39 skeletons, squashing, links, mixed scripts,
  similarity and caps ratio. Still to add from the plan: `captures` with
  positions, `split`, sentences, `plain_text` from Markdown, links from the
  server's own Markdown parser, `repeat_ratio`, `emoji_count`,
  `script_mix`. Its caches are per thread and unbounded; the server needs a
  bounded cache per script.
- `find_words` matching the longest entry first can miss a shorter entry at
  the same place when the longer one fails the word-boundary check. Fine
  for the samples; worth a test in the real version.
