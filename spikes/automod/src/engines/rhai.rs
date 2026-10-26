//! Rhai: pure Rust, sandboxed by default, operation counting built in.

use rhai::serde::{from_dynamic, to_dynamic};
use rhai::{AST, Array, Dynamic, Engine as RhaiEngine, EvalAltResult, Module, Scope};
use serde_json::json;

use super::{Engine, MAX_OPERATIONS, MAX_STRING, Prepared, TIME_LIMIT_MESSAGE, out_of_time};
use crate::context::{Action, Post};
use crate::text;

pub struct Rhai;

struct Script {
    engine: RhaiEngine,
    ast: AST,
}

type RhaiResult<T> = Result<T, Box<EvalAltResult>>;

fn strings(list: Array) -> RhaiResult<Vec<String>> {
    list.into_iter()
        .map(|item| item.into_string().map_err(|t| format!("expected a list of text, found {t}").into()))
        .collect()
}

fn dynamic<T: serde::Serialize>(value: &T) -> RhaiResult<Dynamic> {
    to_dynamic(value)
}

fn text_module() -> Module {
    let mut m = Module::new();
    m.set_native_fn("matches", |p: &str, t: &str| Ok(text::matches(p, t)?));
    m.set_native_fn("find_all", |p: &str, t: &str| dynamic(&text::find_all(p, t)?));
    m.set_native_fn("captures", |p: &str, t: &str| dynamic(&text::captures(p, t)?));
    m.set_native_fn("replace", |p: &str, t: &str, w: &str| Ok(text::replace(p, t, w)?));
    m.set_native_fn("find_words", |list: Array, t: &str| dynamic(&text::find_words(&strings(list)?, t)?));
    m.set_native_fn("normalize", |t: &str| Ok(text::normalize(t)));
    m.set_native_fn("skeleton", |t: &str| Ok(text::skeleton(t)));
    m.set_native_fn("links", |t: &str| dynamic(&text::links(t)));
    m.set_native_fn("words", |t: &str| dynamic(&text::words(t)));
    m.set_native_fn("mixed_script_words", |t: &str| dynamic(&text::mixed_script_words(t)));
    m.set_native_fn("similarity", |a: &str, b: &str| Ok(text::similarity(a, b)));
    m.set_native_fn("caps_ratio", |t: &str| Ok(text::caps_ratio(t)));
    m
}

fn engine() -> RhaiEngine {
    // `Engine::new` has no file, network or process access to begin with.
    let mut engine = RhaiEngine::new();
    engine
        .set_max_operations(MAX_OPERATIONS)
        .set_max_string_size(MAX_STRING)
        .set_max_array_size(10_000)
        .set_max_map_size(10_000)
        .set_max_call_levels(32)
        .set_max_expr_depths(64, 32)
        // A misspelled field (`post.titel`) is an error, not a silent ().
        .set_fail_on_invalid_map_property(true);
    engine.on_progress(|_| out_of_time().then(|| TIME_LIMIT_MESSAGE.into()));
    engine.register_static_module("text", text_module().into());
    engine.register_fn("reject", |message: &str| dynamic(&json!({ "action": "reject", "message": message })));
    engine.register_fn("flag", |reason: &str, category: &str| {
        dynamic(&json!({ "action": "flag", "reason": reason, "category": category }))
    });
    engine
}

impl Engine for Rhai {
    fn name(&self) -> &'static str {
        "Rhai"
    }

    fn extension(&self) -> &'static str {
        "rhai"
    }

    fn prepare(&self, source: &str) -> Result<Box<dyn Prepared>, String> {
        let engine = engine();
        let ast = engine.compile(source).map_err(|e| e.to_string())?;
        Ok(Box::new(Script { engine, ast }))
    }
}

impl Prepared for Script {
    fn run(&self, post: &Post) -> Result<Action, String> {
        let mut scope = Scope::new();
        scope.push_constant("post", dynamic(post).map_err(|e| e.to_string())?);
        let result: Dynamic = self.engine.eval_ast_with_scope(&mut scope, &self.ast).map_err(|e| e.to_string())?;
        if result.is_unit() {
            return Ok(Action::Pass);
        }
        Action::from_json(from_dynamic(&result).map_err(|e| e.to_string())?)
    }
}
