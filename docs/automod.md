# Moderation scripts

Moderation scripts check what writers post on your server before it's
saved. They're short programs in [Rhai](https://rhai.rs/book/), a small
language that reads much like JavaScript. You don't need to write one from
scratch: the server comes with [templates](#templates) that you copy and
adjust, and most of the adjusting is editing a list.

Scripts only ever see public board content: prompts, comments on
announcements, new tags and reports. They never see sessions, login names,
or who filed a report.

## How it works

Each **hook** is a moment when something is posted:

| Hook | When |
| --- | --- |
| `prompt_created` | A writer posts a prompt. |
| `prompt_edited` | A writer edits a prompt's title, text, tags or rating. Changing only open/busy/closed doesn't count. |
| `comment_created` | A writer comments on an announcement. |
| `tag_requested` | A writer uses a tag your server doesn't have yet. |
| `prompt_reported` | A writer reports a prompt. |

Each hook has a **chain**: scripts that run in the order you choose. The
first script that does something (refuses, flags or dismisses) decides,
and the rest don't run. If no script does anything, the post goes through.

Scripts don't check what administrators post.

## A first script

```rhai
// Refuse prompts that mention "free crypto".
if text::find_words(["free crypto"], post.title + " " + post.body).len() > 0 {
    return reject("This isn't the place for that.");
}
```

- `post` is what's being posted (see [What a script is given](#what-a-script-is-given)).
- `text::find_words` looks for words and phrases (see [Text functions](#text-functions)).
- `reject(...)` refuses it with your message. `return` stops the script there.
- A script that reaches its end without returning anything does nothing.

## What scripts can do

| Action | What happens |
| --- | --- |
| `reject("message")` | The post isn't saved. The writer sees your message above their form, with what they wrote kept, so they can change it and try again. Your script's name isn't shown. A note goes on their account's notices with the title and your message (never what they wrote). For a report, the report isn't filed. |
| `reject()` | The same, with the message "This was refused by the server's moderation rules." |
| `flag("reason", "category")` | The post goes through, and a report lands in your inbox marked "Automated: *script name*", with your reason. The writer isn't told. Categories: `illegal`, `spam`, `harassment`, `wrong-maturity`, `other`. For a new tag, the tag is marked in the pending-tags queue instead. |
| `flag("reason")` | The same, in the `other` category. |
| `dismiss("reason")` | `prompt_reported` only: the report is filed already closed, and the admin log says why. |

Every action a script takes is in the admin log, under the script's name.

## What a script is given

`post` has the same fields at every hook, so one script can serve several
chains. Fields a hook has nothing for are empty.

| Field | What |
| --- | --- |
| `post.hook` | Which hook is running, such as `"prompt_created"`. |
| `post.kind` | `"prompt"`, `"comment"`, `"tag"` or `"report"`. |
| `post.title` | A prompt's title, or a new tag's name. Empty for a comment. |
| `post.body` | The text, in Markdown. Empty for a tag. |
| `post.tags` | Tags as `"genre:horror"`. |
| `post.maturity` | `"general"`, `"teen"`, `"mature"` or `"explicit"`. |
| `post.previous.title`, `.body`, `.tags`, `.maturity` | For an edit: the prompt as it was. |
| `post.tag.namespace`, `.slug`, `.display` | For a new tag: the tag. |
| `post.report.reason`, `.details` | For a report: the category and what the reporter wrote. |
| `post.author.handle` | The persona's handle. |
| `post.author.account_age_days` | How old the account is, in days (with a fraction). |
| `post.author.removals` | Prompts and comments administrators have removed from this account. |
| `post.author.rejections` | Posts moderation scripts have refused from this account. |

For a report, `post` is the reported prompt and `post.author` its writer.
Nothing about the reporter is given.

### Posting history

Counted per account, across all its personas.

| Field | What |
| --- | --- |
| `post.history.prompts_last_hour`, `_last_day`, `_last_week` | Prompts posted. |
| `post.history.edited_last_hour`, `_last_day` | Prompts whose latest edit was in that time. |
| `post.history.comments_last_hour`, `_last_day` | Comments posted. |
| `post.history.recent_minutes_ago` | Minutes since each of the account's last 20 prompts and comments, newest first. |
| `post.history.own_recent` | The account's own prompts from the last 30 days (up to 20), most alike first. |
| `post.history.similar_on_board` | The most alike prompts on the board from any account in the last 30 days (up to 5). |
| `post.history.most_similar_on_board` | The highest similarity in that list, 0 if none. |

Each prompt in `own_recent` and `similar_on_board` has `title`,
`similarity` (0 to 1), `same_account` (true or false) and `minutes_ago`.
These lists are filled for prompts and reports, not comments or tags.

## Text functions

Everything that matches words or patterns ignores capitals unless you say
otherwise.

### Words and phrases

`text::find_words(list, text)` gives back the entries of `list` that
appear in `text` as whole words. It sees through:

- spacing and dots: `s p a m`, `s.p.a.m`
- stretched letters: `spaaaam`
- accents and invisible characters: `spám`, `sp​am`
- lookalike letters: `pаypal` with a Cyrillic `а`

So your list only needs the plain spelling. "spam" doesn't match
"spammer"; add "spammer" if you want it.

```rhai
let found = text::find_words(common::lines("blocked_words"), post.body);
if found.len() > 0 {
    return flag("Uses " + found[0], "spam");
}
```

### Patterns

Regular expressions, in the form Rust's `regex` crate understands. Write
them between backticks so backslashes stay as written. They can't hang the
server, however they're written.

| Function | Gives back |
| --- | --- |
| `text::matches(pattern, text)` | `true` or `false`. |
| `text::find(pattern, text)` | The first match, with `text`, `start` and `end` (counted in characters), or `()`. |
| `text::find_all(pattern, text)` | Every match, the same way. |
| `text::captures(pattern, text)` | The first match's groups: by number (`caps["1"]`) and by name for `(?<name>...)`, or `()`. |
| `text::replace(pattern, text, with)` | `text` with every match replaced; `$1` or `$name` in `with` puts a group back. |
| `text::split(pattern, text)` | The pieces between matches. |

Start a pattern with `(?-i)` to make capitals count.

```rhai
if text::matches(`\+?\(?\d[\d\s().-]{6,}\d`, post.body) {
    return reject("Please don't share phone numbers.");
}
```

### Links

`text::links(markdown)` gives every link: written as Markdown links, or
just typed out, with or without `https://`. Each has `url` and `host`
(lower case, without `www.`).

```rhai
for link in text::links(post.body) {
    if link.host != "archiveofourown.org" {
        return reject("Links to other sites aren't allowed.");
    }
}
```

### Everything else

| Function | Gives back |
| --- | --- |
| `text::plain_text(markdown)` | The text without its formatting. |
| `text::words(text)` | The words. |
| `text::sentences(text)` | The sentences. |
| `text::normalize(text)` | Lower case, no accents or invisible characters. |
| `text::skeleton(text)` | `normalize`, with lookalike letters made the same. |
| `text::fold(text)` | What `find_words` compares: `skeleton`, with spacing and repeats squashed. |
| `text::similarity(a, b)` | 0 (nothing alike) to 1 (the same). |
| `text::caps_ratio(text)` | Share of letters that are capitals, 0 to 1. |
| `text::repeat_ratio(text)` | Share of characters in runs of three or more (`!!!`, `sooo`), 0 to 1. |
| `text::emoji_count(text)` | How many emoji. |
| `text::mixed_script_words(text)` | Words that mix writing systems, like `pаypal`. |
| `text::script_mix(text)` | The writing systems used, such as `["Latin", "Cyrillic"]`, most used first. |

## Common data

Lists and text you keep once and use in any script, such as blocked words
or allowed sites. Change the list and every script using it follows; no
script needs editing. Find it under **Server settings → Moderation → Common data**.

- Names are lowercase letters, digits and underscores: `blocked_words`.
- For a list, put one entry per line. Lines starting with `#` are notes.
- Every save is kept. You can see the history, and undo a save, which
  saves the old version again.
- If two administrators edit the same entry, the second save is refused
  rather than overwriting the first.

In a script:

| Function | Gives back |
| --- | --- |
| `common::lines("name")` | The entry as a list: one item per line, trimmed, blank and `#` lines left out. |
| `common::text("name")` | The entry exactly as saved. |

```rhai
let blocked = common::lines("blocked_words");
if "spam" in blocked { print("spam is on the list"); }
for word in blocked { print(word); }
print(blocked.len());
```

Write the name out in full, in quotes: `common::lines("blocked_words")`.
The server needs to see it to check the entry exists when you upload the
script, and to stop it being deleted while a script uses it. A name put
together while the script runs is refused.

## Chains

Under **Server settings → Moderation → Chains**, each hook has its list of
scripts. Number the ones to run, in order; leave a script blank to leave
it out. A script can be in several chains.

Each chain also says what happens when one of its scripts **fails**: has a
mistake that only shows on some posts, or takes too long.

| Setting | When a script fails |
| --- | --- |
| Carry on | Skip it and run the rest of the chain. |
| Refuse | Refuse the post with "This couldn't be checked by the server's moderation rules just now. Please try again later." |
| Flag | Let it through and file a report saying which script failed. |

Every failure is in the admin log, whatever the setting.

## Trying a script safely

- **When you upload**, the server checks the script: that it has no
  mistakes in how it's written, that the common data it names exists, and
  that it runs on a made-up post. Mistakes are shown with their line
  number. Adding a script to a chain runs it on a made-up post for that
  hook too.
- **Log only**: a script in log-only mode runs and logs what it would have
  done, without doing it. Start new scripts this way and watch the log.
- **Dry run**: run a chain on text you type, or on an existing prompt, and
  see what each script decided, what it printed, and how long it took.
  Nothing is done.
- **Off**: a script switched off doesn't run, but stays in its chains.

`print(...)` in a script shows its output in a dry run.

## Limits

A script is stopped if it runs too long, so a mistake like an endless loop
can't slow the server. The server's operator sets these:

| Setting | Default |
| --- | --- |
| `QUILL_AUTOMOD_MAX_OPERATIONS`: steps a script may take on one post | 1,000,000 |
| `QUILL_AUTOMOD_TIME_MS`: time a script may take on one post | 100 ms |
| `QUILL_AUTOMOD_DATA_ENTRY_BYTES`: largest common data entry | 1 MiB |
| `QUILL_AUTOMOD_DATA_TOTAL_BYTES`: all common data together | 16 MiB |

Scripts can't read files, reach the network, or load other scripts.

## Templates

Under **Server settings → Moderation → Scripts → Start from a template**. Choosing one
copies it into a new script of your own; edit it as you like. It also
creates any common data entry the template reads, if you don't have it yet.

| Template | Does | Reads |
| --- | --- | --- |
| Blocked words and phrases | Refuses posts using any word or phrase on the list. | `blocked_words` |
| Links to outside sites | Refuses (or, if you change one line, flags) links to sites not on the list. | `allowed_sites` |
| Phone numbers and email addresses | Refuses posts sharing contact details. | |
| Mixed alphabets | Flags words that mix writing systems. | |
| New accounts | Limits how fast new accounts post, and flags their near copies of others' prompts. | |
| Shouting and repeated characters | Flags posts mostly in capitals, or full of repeated characters. | |
| Posting the same prompt again | Refuses a prompt almost the same as one the account posted in the last 30 days. | |

Each template starts with the settings you're likely to change, like this
one from "New accounts":

```rhai
// How old, in days, an account must be to skip these checks.
let new_for_days = 3;
// Prompts a new account may post in an hour.
let per_hour = 3;
```

## From the command line

Everything on these pages is also in the `quill` CLI, for administrators:

```sh
quill admin automod templates
quill admin automod template blocked_words
quill admin automod data set blocked_words words.txt
quill admin automod chain prompt_created --on-error continue "Blocked words and phrases"
quill admin automod dry-run prompt_created --title "A deal" --body "s p a m for sale"
quill admin automod upload "My rule" my-rule.rhai --log-only
```

`quill admin automod --help` lists the rest, and the API is under
`/api/v1/admin/moderation` (see `docs/openapi.json`).

## More Rhai

The [Rhai book](https://rhai.rs/book/) covers the language. What you need
most:

```rhai
let x = 3;                       // a variable
let words = ["one", "two"];      // a list; words[0] is "one"
if x > 2 && words.len() == 2 { } // and: &&, or: ||, not: !
for w in words { }               // a loop
"a" + "b"                        // joining text
"Hello".contains("ell")          // true
"Hello".to_lower()               // "hello"
"x" in words                     // false
return;                          // stop here, doing nothing
```

A misspelled field, such as `post.titel`, is an error that names it. The
upload check catches most of these.
