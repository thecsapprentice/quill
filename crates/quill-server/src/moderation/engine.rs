//! Compiling and running one moderation script in Rhai.
//!
//! Each script gets its own engine with its limits, the `text` and `common`
//! modules and the action functions. A script runs on a blocking thread;
//! what one run needs (its clock, its data, its printed lines) is kept in a
//! thread-local for the length of the run. Design notes: architecture
//! document, "Automated moderation: the engine".

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use quill_proto::board::ReportReason;
use rhai::module_resolvers::DummyModuleResolver;
use rhai::serde::to_dynamic;
use rhai::{
    AST, ASTNode, Array, Dynamic, Engine, EvalAltResult, Expr, FnCallExpr, ImmutableString, Module,
    Position, Scope, Stmt,
};

use super::Limits;
use super::data::{Entry, Snapshot};
use super::text::{self, TextCache};

/// Longest array or map a script may build.
const MAX_COLLECTION: usize = 100_000;
/// Lines of `print` output kept from one run, for dry runs.
const MAX_PRINTED: usize = 50;
/// The two `common` functions, the only ways to read common data.
const COMMON_FUNCTIONS: [&str; 2] = ["text", "lines"];

/// What a script decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Not handled: the script returned nothing. The chain carries on.
    Pass,
    /// Refuse the submission. `None`: the generic message.
    Reject { message: Option<String> },
    /// Let it through and file a report.
    Flag {
        reason: String,
        category: ReportReason,
    },
    /// Close the writer's report (`prompt_reported` only).
    Dismiss { reason: String },
}

impl Outcome {
    pub fn handled(&self) -> bool {
        !matches!(self, Self::Pass)
    }
}

/// An action a script returns, as Rhai sees it: only the action functions
/// make one, so a script can't hand back a look-alike map.
#[derive(Debug, Clone)]
struct Action(Outcome);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Refused at upload: the script doesn't parse, or breaks a rule.
    Compile,
    /// It failed while running.
    Runtime,
    /// It ran out of operations, time or memory.
    Limit,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ScriptError {
    pub kind: ErrorKind,
    pub message: String,
    /// Line in the script, when the engine knows it.
    pub line: Option<usize>,
}

impl ScriptError {
    fn compile(message: impl Into<String>, line: Option<usize>) -> Self {
        Self {
            kind: ErrorKind::Compile,
            message: message.into(),
            line,
        }
    }

    fn from_eval(err: &EvalAltResult) -> Self {
        let kind = match err {
            EvalAltResult::ErrorTooManyOperations(..)
            | EvalAltResult::ErrorTerminated(..)
            | EvalAltResult::ErrorDataTooLarge(..)
            | EvalAltResult::ErrorStackOverflow(..) => ErrorKind::Limit,
            _ => ErrorKind::Runtime,
        };
        let message = match err {
            EvalAltResult::ErrorTerminated(..) => "the script ran past its time limit".to_owned(),
            EvalAltResult::ErrorTooManyOperations(..) => {
                "the script ran past its operation limit".to_owned()
            }
            other => other.to_string(),
        };
        Self {
            kind,
            message,
            line: line_of(err.position()),
        }
    }
}

fn line_of(pos: Position) -> Option<usize> {
    pos.line()
}

/// A compiled script, ready to run on any number of posts.
pub struct Script {
    engine: Engine,
    ast: AST,
    /// The common data entries the script reads, from its source.
    data_names: BTreeSet<String>,
}

impl std::fmt::Debug for Script {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Script")
            .field("data_names", &self.data_names)
            .finish_non_exhaustive()
    }
}

/// What one run can see, set for the length of the run on its thread.
struct RunState {
    deadline: Instant,
    data: Snapshot,
    allowed: BTreeSet<String>,
    printed: Vec<String>,
}

thread_local! {
    static RUN: RefCell<Option<RunState>> = const { RefCell::new(None) };
}

/// Clears the run state however the run ends.
struct RunGuard;

impl Drop for RunGuard {
    fn drop(&mut self) {
        RUN.with(|r| r.borrow_mut().take());
    }
}

fn out_of_time() -> bool {
    RUN.with(|r| {
        r.borrow()
            .as_ref()
            .is_some_and(|s| Instant::now() > s.deadline)
    })
}

type RhaiResult<T> = Result<T, Box<EvalAltResult>>;

fn fail<T>(message: impl Into<String>) -> RhaiResult<T> {
    Err(message.into().into())
}

/// An entry the running script may read: in the snapshot, and named in
/// its source (so upload checked it exists).
fn entry(name: &str) -> RhaiResult<Arc<Entry>> {
    RUN.with(|r| {
        let state = r.borrow();
        let Some(state) = state.as_ref() else {
            return fail("common data can only be read while a script runs");
        };
        if !state.allowed.contains(name) {
            return fail(format!(
                "common data {name:?} must be named in the script as written-out text"
            ));
        }
        state.data.get(name).cloned().map_or_else(
            || fail(format!("there is no common data named {name:?}")),
            Ok,
        )
    })
}

/// A list from common data: shared with every other run reading the same
/// version, never copied. Scripts index it, loop over it and use `in`.
#[derive(Debug, Clone)]
pub struct DataList {
    entry: Arc<Entry>,
    lines: Arc<[ImmutableString]>,
}

impl IntoIterator for DataList {
    type Item = ImmutableString;
    type IntoIter = DataListIter;

    fn into_iter(self) -> Self::IntoIter {
        DataListIter {
            lines: self.lines,
            next: 0,
        }
    }
}

/// Walks the shared lines without copying the list.
pub struct DataListIter {
    lines: Arc<[ImmutableString]>,
    next: usize,
}

impl Iterator for DataListIter {
    type Item = ImmutableString;

    fn next(&mut self) -> Option<Self::Item> {
        let line = self.lines.get(self.next)?.clone();
        self.next += 1;
        Some(line)
    }
}

fn index(len: usize, i: i64) -> Option<usize> {
    let len_i = i64::try_from(len).ok()?;
    let i = if i < 0 { len_i + i } else { i };
    usize::try_from(i).ok().filter(|i| *i < len)
}

fn strings(list: &Array) -> RhaiResult<Vec<String>> {
    list.iter()
        .map(|item| {
            item.clone()
                .into_immutable_string()
                .map(|s| s.to_string())
                .or_else(|t| fail(format!("expected a list of text, found {t} in it")))
        })
        .collect()
}

fn dynamic<T: serde::Serialize>(value: &T) -> RhaiResult<Dynamic> {
    to_dynamic(value)
}

fn report_reason(category: &str) -> RhaiResult<ReportReason> {
    category.parse().or_else(|_| {
        fail(format!(
            "{category:?} isn't a report category; use illegal, spam, harassment, wrong-maturity or other"
        ))
    })
}

fn regex(cache: &TextCache, pattern: &str) -> RhaiResult<Arc<regex::Regex>> {
    cache.regex(pattern).or_else(|e| fail(e.to_string()))
}

fn text_module(cache: Arc<TextCache>) -> Module {
    let mut m = Module::new();
    let c = cache.clone();
    m.set_native_fn("matches", move |p: &str, t: &str| {
        Ok(text::is_match(&*regex(&c, p)?, t))
    });
    let c = cache.clone();
    m.set_native_fn("find", move |p: &str, t: &str| {
        text::find(&*regex(&c, p)?, t).map_or(Ok(Dynamic::UNIT), |f| dynamic(&f))
    });
    let c = cache.clone();
    m.set_native_fn("find_all", move |p: &str, t: &str| {
        dynamic(&text::find_all(&*regex(&c, p)?, t))
    });
    let c = cache.clone();
    m.set_native_fn("captures", move |p: &str, t: &str| {
        text::captures(&*regex(&c, p)?, t).map_or(Ok(Dynamic::UNIT), |caps| dynamic(&caps))
    });
    let c = cache.clone();
    m.set_native_fn("replace", move |p: &str, t: &str, with: &str| {
        Ok(text::replace(&*regex(&c, p)?, t, with))
    });
    let c = cache.clone();
    m.set_native_fn("split", move |p: &str, t: &str| {
        dynamic(&text::split(&*regex(&c, p)?, t))
    });
    let c = cache;
    m.set_native_fn("find_words", move |list: Array, t: &str| {
        let words = c
            .word_list(&strings(&list)?)
            .or_else(|e| fail(e.to_string()))?;
        dynamic(&words.find_in(t))
    });
    m.set_native_fn("find_words", |list: DataList, t: &str| {
        let words = list.entry.word_list().or_else(|e| fail(e.to_string()))?;
        dynamic(&words.find_in(t))
    });
    m.set_native_fn("normalize", |t: &str| Ok(text::normalize(t)));
    m.set_native_fn("skeleton", |t: &str| Ok(text::skeleton(t)));
    m.set_native_fn("fold", |t: &str| Ok(text::fold(t)));
    m.set_native_fn("links", |t: &str| dynamic(&text::links(t)));
    m.set_native_fn("plain_text", |t: &str| Ok(text::plain_text(t)));
    m.set_native_fn("words", |t: &str| dynamic(&text::words(t)));
    m.set_native_fn("sentences", |t: &str| dynamic(&text::sentences(t)));
    m.set_native_fn("mixed_script_words", |t: &str| {
        dynamic(&text::mixed_script_words(t))
    });
    m.set_native_fn("script_mix", |t: &str| dynamic(&text::script_mix(t)));
    m.set_native_fn("similarity", |a: &str, b: &str| Ok(text::similarity(a, b)));
    m.set_native_fn("caps_ratio", |t: &str| Ok(text::caps_ratio(t)));
    m.set_native_fn("repeat_ratio", |t: &str| Ok(text::repeat_ratio(t)));
    m.set_native_fn("emoji_count", |t: &str| {
        Ok(i64::try_from(text::emoji_count(t)).unwrap_or(i64::MAX))
    });
    m
}

fn common_module() -> Module {
    let mut m = Module::new();
    m.set_native_fn("text", |name: &str| Ok(entry(name)?.content.clone()));
    m.set_native_fn("lines", |name: &str| {
        let entry = entry(name)?;
        let lines = entry.lines();
        Ok(DataList { entry, lines })
    });
    m
}

fn register_actions(engine: &mut Engine) {
    engine.register_type_with_name::<Action>("Action");
    engine.register_fn("reject", || Action(Outcome::Reject { message: None }));
    engine.register_fn("reject", |message: &str| {
        let message = message.trim();
        Action(Outcome::Reject {
            message: (!message.is_empty()).then(|| message.to_owned()),
        })
    });
    engine.register_fn("flag", |reason: &str| {
        Action(Outcome::Flag {
            reason: reason.to_owned(),
            category: ReportReason::Other,
        })
    });
    engine.register_fn(
        "flag",
        |reason: &str, category: &str| -> RhaiResult<Action> {
            Ok(Action(Outcome::Flag {
                reason: reason.to_owned(),
                category: report_reason(category)?,
            }))
        },
    );
    engine.register_fn("dismiss", |reason: &str| {
        Action(Outcome::Dismiss {
            reason: reason.to_owned(),
        })
    });
}

fn register_data_list(engine: &mut Engine) {
    engine.register_type_with_name::<DataList>("DataList");
    engine.register_iterator::<DataList>();
    engine.register_fn("len", |l: &mut DataList| {
        i64::try_from(l.lines.len()).unwrap_or(i64::MAX)
    });
    engine.register_fn("is_empty", |l: &mut DataList| l.lines.is_empty());
    // Makes `"spam" in list` work.
    engine.register_fn("contains", |l: &mut DataList, s: ImmutableString| {
        l.lines.contains(&s)
    });
    engine.register_fn("to_array", |l: &mut DataList| -> Array {
        l.lines.iter().cloned().map(Dynamic::from).collect()
    });
    engine.register_indexer_get(|l: &mut DataList, i: i64| -> RhaiResult<ImmutableString> {
        index(l.lines.len(), i)
            .map(|i| l.lines[i].clone())
            .map_or_else(|| fail(format!("{} has no line {i}", l.entry.name)), Ok)
    });
}

fn engine(limits: &Limits) -> Engine {
    let mut engine = Engine::new();
    engine
        // No `import`: the default resolver would read script files from
        // the server's disk.
        .set_module_resolver(DummyModuleResolver::new())
        .set_max_operations(limits.max_operations)
        .set_max_string_size(limits.data_entry_bytes.max(1 << 20))
        .set_max_array_size(MAX_COLLECTION)
        .set_max_map_size(MAX_COLLECTION)
        .set_max_call_levels(32)
        .set_max_expr_depths(64, 32)
        // `post.titel` is an error, not a silent ().
        .set_fail_on_invalid_map_property(true)
        // Undeclared variables are refused at upload.
        .set_strict_variables(true);
    // Scripts run only their own uploaded source.
    engine.disable_symbol("eval");
    engine.on_progress(|_| out_of_time().then(|| Dynamic::from("time limit")));
    engine.on_print(|s| {
        RUN.with(|r| {
            if let Some(state) = r.borrow_mut().as_mut()
                && state.printed.len() < MAX_PRINTED
            {
                state.printed.push(s.to_owned());
            }
        })
    });
    engine.on_debug(|_, _, _| {});
    engine.register_static_module("common", common_module().into());
    register_actions(&mut engine);
    register_data_list(&mut engine);
    engine
}

/// A function call in the syntax tree. A call used as a statement on its
/// own is a statement node, not an expression.
fn as_call<'a>(node: Option<&'a ASTNode<'a>>) -> Option<(&'a FnCallExpr, Position)> {
    match node? {
        ASTNode::Expr(Expr::FnCall(call, pos)) | ASTNode::Stmt(Stmt::FnCall(call, pos)) => {
            Some((call, *pos))
        }
        _ => None,
    }
}

/// The common data entries a script reads. Every call to `common::text` or
/// `common::lines` must name its entry as written-out text, so upload can
/// check it exists and deleting it can be refused while the script uses it.
fn data_names(ast: &AST) -> Result<BTreeSet<String>, ScriptError> {
    let mut names = BTreeSet::new();
    let mut problem: Option<ScriptError> = None;
    ast.walk(&mut |path: &[ASTNode]| {
        let Some((call, pos)) = as_call(path.last()) else {
            return true;
        };
        if call.namespace.is_empty() || call.namespace.root() != "common" {
            return true;
        }
        if !COMMON_FUNCTIONS.contains(&call.name.as_str()) {
            problem = Some(ScriptError::compile(
                format!("common has no function {:?}; use common::text or common::lines", call.name),
                line_of(pos),
            ));
            return false;
        }
        match call.args.as_slice() {
            [Expr::StringConstant(name, _)] => {
                names.insert(name.to_string());
                true
            }
            _ => {
                problem = Some(ScriptError::compile(
                    format!(
                        "common::{} needs the entry's name written out, like common::{}(\"blocked_words\")",
                        call.name, call.name
                    ),
                    line_of(pos),
                ));
                false
            }
        }
    });
    match problem {
        Some(p) => Err(p),
        None => Ok(names),
    }
}

impl Script {
    /// Compiles `source`, refusing syntax errors, undeclared variables,
    /// invalid literal patterns and data names that aren't written out.
    /// `variables` are the names the script is given (such as `post`).
    pub fn compile(source: &str, variables: &[&str], limits: &Limits) -> Result<Self, ScriptError> {
        let cache = Arc::new(TextCache::default());
        let mut engine = engine(limits);
        engine.register_static_module("text", text_module(cache.clone()).into());

        let mut scope = Scope::new();
        for name in variables {
            scope.push_constant(*name, ());
        }
        let ast = engine
            .compile_with_scope(&scope, source)
            .map_err(|e| ScriptError::compile(e.to_string(), line_of(e.position())))?;
        let data_names = data_names(&ast)?;
        check_literal_patterns(&ast)?;
        Ok(Self {
            engine,
            ast,
            data_names,
        })
    }

    pub fn data_names(&self) -> &BTreeSet<String> {
        &self.data_names
    }

    /// Runs the script once. `variables` must give a value for each name
    /// given to [`Script::compile`].
    pub fn run(&self, variables: &[(&str, Dynamic)], data: &Snapshot, time_limit: Duration) -> Run {
        RUN.with(|r| {
            *r.borrow_mut() = Some(RunState {
                deadline: Instant::now() + time_limit,
                data: data.clone(),
                allowed: self.data_names.clone(),
                printed: Vec::new(),
            });
        });
        let _guard = RunGuard;
        let started = Instant::now();
        let mut scope = Scope::new();
        for (name, value) in variables {
            scope.push_constant(*name, value.clone());
        }
        let result = self
            .engine
            .eval_ast_with_scope::<Dynamic>(&mut scope, &self.ast)
            .map_err(|e| ScriptError::from_eval(&e))
            .and_then(to_outcome);
        let printed = RUN.with(|r| {
            r.borrow_mut()
                .as_mut()
                .map(|s| std::mem::take(&mut s.printed))
        });
        Run {
            result,
            printed: printed.unwrap_or_default(),
            took: started.elapsed(),
        }
    }
}

/// What one run produced.
#[derive(Debug, Clone)]
pub struct Run {
    pub result: Result<Outcome, ScriptError>,
    /// What the script printed, for dry runs.
    pub printed: Vec<String>,
    pub took: Duration,
}

fn to_outcome(value: Dynamic) -> Result<Outcome, ScriptError> {
    if value.is_unit() {
        return Ok(Outcome::Pass);
    }
    match value.try_cast::<Action>() {
        Some(Action(outcome)) => Ok(outcome),
        None => Err(ScriptError {
            kind: ErrorKind::Runtime,
            message: "a script must end by returning reject(...), flag(...) or dismiss(...), or nothing at all".into(),
            line: None,
        }),
    }
}

/// Patterns written out in `text::` calls are compiled at upload, so a bad
/// one is refused then rather than failing on every post.
fn check_literal_patterns(ast: &AST) -> Result<(), ScriptError> {
    const PATTERN_FIRST: [&str; 6] = [
        "matches", "find", "find_all", "captures", "replace", "split",
    ];
    let mut problem = None;
    ast.walk(&mut |path: &[ASTNode]| {
        if let Some((call, pos)) = as_call(path.last())
            && !call.namespace.is_empty()
            && call.namespace.root() == "text"
            && PATTERN_FIRST.contains(&call.name.as_str())
            && let Some(Expr::StringConstant(pattern, _)) = call.args.first()
            && let Err(e) = text::compile(pattern)
        {
            problem = Some(ScriptError::compile(e.to_string(), line_of(pos)));
            return false;
        }
        true
    });
    problem.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn limits() -> Limits {
        Limits {
            max_operations: 100_000,
            time_ms: 200,
            ..Limits::default()
        }
    }

    fn compile(source: &str) -> Result<Script, ScriptError> {
        Script::compile(source, &["post"], &limits())
    }

    fn post(title: &str, body: &str) -> Dynamic {
        to_dynamic(json!({ "title": title, "body": body, "author": { "account_age_days": 400.0 } }))
            .unwrap()
    }

    fn run_with(script: &Script, post: Dynamic, data: &Snapshot) -> Run {
        script.run(&[("post", post)], data, Duration::from_millis(200))
    }

    fn run(source: &str, title: &str, body: &str) -> Result<Outcome, ScriptError> {
        run_with(
            &compile(source).unwrap(),
            post(title, body),
            &Snapshot::default(),
        )
        .result
    }

    #[test]
    fn actions() {
        assert_eq!(run("", "t", "b").unwrap(), Outcome::Pass);
        assert_eq!(
            run("reject()", "t", "b").unwrap(),
            Outcome::Reject { message: None }
        );
        assert_eq!(
            run(r#"return reject("No links");"#, "t", "b").unwrap(),
            Outcome::Reject {
                message: Some("No links".into())
            }
        );
        assert_eq!(
            run(r#"flag("odd", "spam")"#, "t", "b").unwrap(),
            Outcome::Flag {
                reason: "odd".into(),
                category: ReportReason::Spam
            }
        );
        assert_eq!(
            run(r#"flag("odd")"#, "t", "b").unwrap(),
            Outcome::Flag {
                reason: "odd".into(),
                category: ReportReason::Other
            }
        );
        let bad = run(r#"flag("odd", "rude")"#, "t", "b").unwrap_err();
        assert!(bad.message.contains("isn't a report category"), "{bad}");
    }

    #[test]
    fn a_look_alike_map_is_not_an_action() {
        let err = run(r#"#{ action: "reject" }"#, "t", "b").unwrap_err();
        assert!(err.message.contains("must end by returning"), "{err}");
    }

    #[test]
    fn the_text_api_is_there() {
        let source = r#"
            let found = text::find_words(["spam"], post.body);
            if found.len() > 0 { return reject("found " + found[0]); }
            if text::matches(`\d{7,}`, post.body) { return reject("phone"); }
            for link in text::links(post.body) {
                if link.host != "quill.example" { return reject("link"); }
            }
        "#;
        assert_eq!(
            run(source, "t", "S P A M").unwrap(),
            Outcome::Reject {
                message: Some("found spam".into())
            }
        );
        assert_eq!(
            run(source, "t", "call 5550107788").unwrap(),
            Outcome::Reject {
                message: Some("phone".into())
            }
        );
        assert_eq!(
            run(source, "t", "see evil.biz/x").unwrap(),
            Outcome::Reject {
                message: Some("link".into())
            }
        );
        assert_eq!(
            run(source, "t", "see quill.example/rules").unwrap(),
            Outcome::Pass
        );
    }

    #[test]
    fn mistakes_are_caught_at_upload_where_possible() {
        let typo_var = compile("pots.title").unwrap_err();
        assert_eq!(typo_var.kind, ErrorKind::Compile);
        assert!(typo_var.message.contains("pots"), "{typo_var}");

        let syntax = compile("let x = [1, 2,\nlet y = 3;").unwrap_err();
        assert_eq!((syntax.kind, syntax.line), (ErrorKind::Compile, Some(2)));

        let pattern = compile(r#"text::matches("(unclosed", post.body)"#).unwrap_err();
        assert!(pattern.message.contains("invalid pattern"), "{pattern}");

        // A misspelled field can only be found by running the script.
        let field = run("post.titel + 1", "t", "b").unwrap_err();
        assert_eq!(field.kind, ErrorKind::Runtime);
        assert!(field.message.contains("titel"), "{field}");
    }

    #[test]
    fn scripts_cannot_reach_outside() {
        let import = run(r#"import "secrets" as s;"#, "t", "b").unwrap_err();
        assert!(import.message.contains("secrets"), "{import}");
        assert!(compile(r#"eval("reject()")"#).is_err());
        let file = run(r#"open("Cargo.toml")"#, "t", "b").unwrap_err();
        assert!(file.message.contains("open"), "{file}");
    }

    #[test]
    fn limits_stop_runaway_scripts() {
        let ops = run("loop {}", "t", "b").unwrap_err();
        assert_eq!(ops.kind, ErrorKind::Limit);
        let script = Script::compile(
            "loop {}",
            &["post"],
            &Limits {
                max_operations: u64::MAX / 2,
                ..limits()
            },
        )
        .unwrap();
        let clock = script.run(
            &[("post", post("t", "b"))],
            &Snapshot::default(),
            Duration::from_millis(30),
        );
        let err = clock.result.unwrap_err();
        assert_eq!(err.kind, ErrorKind::Limit);
        assert!(err.message.contains("time limit"), "{err}");
        assert!(clock.took < Duration::from_secs(2));

        let big = run(r#"let s = "x"; loop { s += s; }"#, "t", "b").unwrap_err();
        assert_eq!(big.kind, ErrorKind::Limit);
        let deep = run("fn f(n) { f(n + 1) } f(0)", "t", "b").unwrap_err();
        assert_eq!(deep.kind, ErrorKind::Limit);
    }

    #[test]
    fn common_data_names_must_be_written_out() {
        let script = compile(r#"common::lines("blocked_words"); common::text("note");"#).unwrap();
        assert_eq!(
            script.data_names().iter().collect::<Vec<_>>(),
            ["blocked_words", "note"]
        );
        let built = compile(r#"let n = "blocked"; common::lines(n + "_words")"#).unwrap_err();
        assert!(built.message.contains("written out"), "{built}");
        let unknown = compile(r#"common::write("x")"#).unwrap_err();
        assert!(unknown.message.contains("no function"), "{unknown}");
    }

    #[test]
    fn common_data_is_read_from_the_snapshot() {
        let data = Snapshot::from_entries([
            (
                "blocked_words".to_owned(),
                3,
                "# refused\nspam\nfree crypto".to_owned(),
            ),
            ("allowed_sites".to_owned(), 1, "quill.example".to_owned()),
        ]);
        let script = compile(
            r#"
            let blocked = common::lines("blocked_words");
            if blocked.len() != 2 || blocked[0] != "spam" || blocked[-1] != "free crypto" { return reject("lines"); }
            if !("spam" in blocked) { return reject("in"); }
            let n = 0; for w in blocked { n += 1; }
            if n != 2 { return reject("loop"); }
            if text::find_words(blocked, post.body).len() > 0 { return reject("found"); }
            if common::text("allowed_sites") != "quill.example" { return reject("text"); }
            "#,
        )
        .unwrap();
        assert_eq!(
            run_with(&script, post("t", "fine"), &data).result.unwrap(),
            Outcome::Pass
        );
        assert_eq!(
            run_with(&script, post("t", "FREE crypto"), &data)
                .result
                .unwrap(),
            Outcome::Reject {
                message: Some("found".into())
            }
        );
        // An entry missing from the snapshot is an error, not empty.
        let missing = run_with(&script, post("t", "fine"), &Snapshot::default())
            .result
            .unwrap_err();
        assert!(missing.message.contains("no common data"), "{missing}");
    }

    #[test]
    fn printed_lines_are_kept_for_dry_runs() {
        let script =
            compile(r#"print("checking " + post.title); for i in 0..100 { print(i); }"#).unwrap();
        let run = run_with(&script, post("A title", "b"), &Snapshot::default());
        assert_eq!(run.printed.len(), MAX_PRINTED);
        assert_eq!(run.printed[0], "checking A title");
    }

    #[test]
    fn numbers_compare_across_int_and_float() {
        assert_eq!(
            run("if post.author.account_age_days > 3 { reject() }", "t", "b").unwrap(),
            Outcome::Reject { message: None }
        );
    }
}
