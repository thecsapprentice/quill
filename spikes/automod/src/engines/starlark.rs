//! Starlark (Python-like, from Bazel) through Meta's `starlark` crate:
//! no `while`, no recursion, no file or network access by design.

use starlark::environment::{FrozenModule, Globals, GlobalsBuilder, Module};
use starlark::eval::{BeforeStmtFunc, BeforeStmtFuncDyn, Evaluator};
use starlark::syntax::{AstModule, Dialect};
use starlark::values::list::{AllocList, UnpackList};
use starlark::values::structs::AllocStruct;
use starlark::values::{Heap, OwnedFrozenValue, Value};
use starlark::{codemap::FileSpanRef, starlark_module};

use super::{Engine, Prepared, TIME_LIMIT_MESSAGE, out_of_time};
use crate::context::{Action, Post};

pub struct Starlark;

struct Script {
    globals: Globals,
    check: OwnedFrozenValue,
    _module: FrozenModule,
}

/// JSON objects become structs, so scripts write `post.author.handle`
/// rather than `post["author"]["handle"]`.
fn to_starlark<'v>(heap: Heap<'v>, value: &serde_json::Value) -> Value<'v> {
    match value {
        serde_json::Value::Object(map) => {
            heap.alloc(AllocStruct(map.iter().map(|(k, v)| (k.as_str(), to_starlark(heap, v)))))
        }
        serde_json::Value::Array(items) => heap.alloc(AllocList(items.iter().map(|v| to_starlark(heap, v)))),
        other => heap.alloc(other),
    }
}

fn json<'v, T: serde::Serialize>(heap: Heap<'v>, value: &T) -> anyhow::Result<Value<'v>> {
    Ok(to_starlark(heap, &serde_json::to_value(value)?))
}

#[starlark_module]
fn text_members(builder: &mut GlobalsBuilder) {
    fn matches(pattern: &str, s: &str) -> anyhow::Result<bool> {
        crate::text::matches(pattern, s).map_err(anyhow::Error::msg)
    }
    fn find_all(pattern: &str, s: &str) -> anyhow::Result<Vec<String>> {
        crate::text::find_all(pattern, s).map_err(anyhow::Error::msg)
    }
    fn captures<'v>(pattern: &str, s: &str, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        json(heap, &crate::text::captures(pattern, s).map_err(anyhow::Error::msg)?)
    }
    fn replace(pattern: &str, s: &str, with: &str) -> anyhow::Result<String> {
        crate::text::replace(pattern, s, with).map_err(anyhow::Error::msg)
    }
    fn find_words(list: UnpackList<String>, s: &str) -> anyhow::Result<Vec<String>> {
        crate::text::find_words(&list.items, s).map_err(anyhow::Error::msg)
    }
    fn normalize(s: &str) -> anyhow::Result<String> {
        Ok(crate::text::normalize(s))
    }
    fn skeleton(s: &str) -> anyhow::Result<String> {
        Ok(crate::text::skeleton(s))
    }
    fn links<'v>(s: &str, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        json(heap, &crate::text::links(s))
    }
    fn words(s: &str) -> anyhow::Result<Vec<String>> {
        Ok(crate::text::words(s))
    }
    fn mixed_script_words(s: &str) -> anyhow::Result<Vec<String>> {
        Ok(crate::text::mixed_script_words(s))
    }
    fn similarity(a: &str, b: &str) -> anyhow::Result<f64> {
        Ok(crate::text::similarity(a, b))
    }
    fn caps_ratio(s: &str) -> anyhow::Result<f64> {
        Ok(crate::text::caps_ratio(s))
    }
}

#[starlark_module]
fn actions(builder: &mut GlobalsBuilder) {
    fn reject<'v>(message: &str, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        json(heap, &serde_json::json!({ "action": "reject", "message": message }))
    }
    fn flag<'v>(reason: &str, category: &str, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        json(heap, &serde_json::json!({ "action": "flag", "reason": reason, "category": category }))
    }
}

/// Stops a script at its time limit. Starlark has no public interrupt API;
/// this uses the statement hook its debugger uses (see FINDINGS.md).
struct Deadline;

impl<'e> BeforeStmtFuncDyn<'e> for Deadline {
    fn call<'v>(&mut self, _: FileSpanRef, _: bool, _: &mut Evaluator<'v, '_, 'e>) -> starlark::Result<()> {
        if out_of_time() { Err(starlark::Error::new_other(anyhow::anyhow!(TIME_LIMIT_MESSAGE))) } else { Ok(()) }
    }
}

fn globals() -> Globals {
    let mut builder = GlobalsBuilder::standard();
    builder.namespace("text", text_members);
    actions(&mut builder);
    builder.build()
}

impl Engine for Starlark {
    fn name(&self) -> &'static str {
        "Starlark"
    }

    fn extension(&self) -> &'static str {
        "star"
    }

    fn prepare(&self, source: &str) -> Result<Box<dyn Prepared>, String> {
        let globals = globals();
        let ast = AstModule::parse("script.star", source.to_owned(), &Dialect::Standard).map_err(|e| e.to_string())?;
        let module = Module::with_temp_heap(|module| {
            {
                let mut eval = Evaluator::new(&module);
                // Setting the hook here makes `check` compile with statement hooks.
                eval.before_stmt_for_dap(BeforeStmtFunc::from_dyn(Box::new(Deadline)));
                eval.eval_module(ast, &globals).map_err(|e| e.to_string())?;
            }
            module.freeze().map_err(|e| format!("{e:?}"))
        })?;
        let check = module.get("check").map_err(|_| "the script needs a `def check(post):` function".to_owned())?;
        Ok(Box::new(Script { globals, check, _module: module }))
    }
}

impl Prepared for Script {
    fn run(&self, post: &Post) -> Result<Action, String> {
        let _ = &self.globals;
        let out = Module::with_temp_heap(|module| {
            let post = json(module.heap(), post).map_err(|e| e.to_string())?;
            let mut eval = Evaluator::new(&module);
            eval.before_stmt_for_dap(BeforeStmtFunc::from_dyn(Box::new(Deadline)));
            // SAFETY: the reference keeps `check`'s heap alive for this
            // module. The crate offers no safe way to call a frozen function
            // from a temporary heap (FINDINGS.md, "Embedding effort").
            let check = unsafe { self.check.owned_frozen_value(module.frozen_heap()) }.to_value();
            let result = eval.eval_function(check, &[post], &[]).map_err(|e| e.to_string());
            result?.to_json().map_err(|e| e.to_string())
        })?;
        Action::from_json(serde_json::from_str(&out).map_err(|e| e.to_string())?)
    }
}
