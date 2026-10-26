//! JavaScript through QuickJS (`rquickjs`): a C engine with no file, network
//! or process access unless the host adds its `std`/`os` modules, which we
//! don't.

use rquickjs::{Context, Ctx, Exception, Function, Object, Persistent, Runtime};

use super::{Engine, MEMORY_LIMIT, Prepared, out_of_time};
use crate::context::{Action, Post};
use crate::text;

pub struct Js;

struct Script {
    // Field order matters: the function must drop before its runtime.
    run: Persistent<Function<'static>>,
    context: Context,
    _runtime: Runtime,
}

/// Builds `text`, `reject` and `flag` from JSON-speaking natives, so the
/// adapter only has to move strings across.
const PRELUDE: &str = r#"
const text = {
  matches: (p, s) => __native.matches(p, s),
  find_all: (p, s) => JSON.parse(__native.find_all(p, s)),
  captures: (p, s) => JSON.parse(__native.captures(p, s)),
  replace: (p, s, w) => __native.replace(p, s, w),
  find_words: (list, s) => JSON.parse(__native.find_words(JSON.stringify(list), s)),
  normalize: (s) => __native.normalize(s),
  skeleton: (s) => __native.skeleton(s),
  links: (s) => JSON.parse(__native.links(s)),
  words: (s) => JSON.parse(__native.words(s)),
  mixed_script_words: (s) => JSON.parse(__native.mixed_script_words(s)),
  similarity: (a, b) => __native.similarity(a, b),
  caps_ratio: (s) => __native.caps_ratio(s),
};
const reject = (message) => ({ action: "reject", message });
const flag = (reason, category) => ({ action: "flag", reason, category });
"#;

fn throw(ctx: &Ctx<'_>, message: String) -> rquickjs::Error {
    Exception::throw_message(ctx, &message)
}

fn to_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned())
}

fn register(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let n = Object::new(ctx.clone())?;
    n.set("matches", Function::new(ctx.clone(), |ctx: Ctx<'_>, p: String, s: String| {
        text::matches(&p, &s).map_err(|e| throw(&ctx, e))
    })?)?;
    n.set("find_all", Function::new(ctx.clone(), |ctx: Ctx<'_>, p: String, s: String| {
        text::find_all(&p, &s).map(|v| to_json(&v)).map_err(|e| throw(&ctx, e))
    })?)?;
    n.set("captures", Function::new(ctx.clone(), |ctx: Ctx<'_>, p: String, s: String| {
        text::captures(&p, &s).map(|v| to_json(&v)).map_err(|e| throw(&ctx, e))
    })?)?;
    n.set("replace", Function::new(ctx.clone(), |ctx: Ctx<'_>, p: String, s: String, w: String| {
        text::replace(&p, &s, &w).map_err(|e| throw(&ctx, e))
    })?)?;
    n.set("find_words", Function::new(ctx.clone(), |ctx: Ctx<'_>, list: String, s: String| {
        let list: Vec<String> = serde_json::from_str(&list).map_err(|e| throw(&ctx, e.to_string()))?;
        text::find_words(&list, &s).map(|v| to_json(&v)).map_err(|e| throw(&ctx, e))
    })?)?;
    n.set("normalize", Function::new(ctx.clone(), |s: String| text::normalize(&s))?)?;
    n.set("skeleton", Function::new(ctx.clone(), |s: String| text::skeleton(&s))?)?;
    n.set("links", Function::new(ctx.clone(), |s: String| to_json(&text::links(&s)))?)?;
    n.set("words", Function::new(ctx.clone(), |s: String| to_json(&text::words(&s)))?)?;
    n.set("mixed_script_words", Function::new(ctx.clone(), |s: String| to_json(&text::mixed_script_words(&s)))?)?;
    n.set("similarity", Function::new(ctx.clone(), |a: String, b: String| text::similarity(&a, &b))?)?;
    n.set("caps_ratio", Function::new(ctx.clone(), |s: String| text::caps_ratio(&s))?)?;
    ctx.globals().set("__native", n)?;
    ctx.eval::<(), _>(PRELUDE)
}

fn describe(ctx: &Ctx<'_>, error: rquickjs::Error) -> String {
    if let rquickjs::Error::Exception = error {
        let thrown = ctx.catch();
        if let Some(e) = thrown.as_exception() {
            return format!("{} {}", e.message().unwrap_or_default(), e.stack().unwrap_or_default());
        }
        return format!("{thrown:?}");
    }
    error.to_string()
}

impl Engine for Js {
    fn name(&self) -> &'static str {
        "JavaScript"
    }

    fn extension(&self) -> &'static str {
        "js"
    }

    fn prepare(&self, source: &str) -> Result<Box<dyn Prepared>, String> {
        let runtime = Runtime::new().map_err(|e| e.to_string())?;
        runtime.set_memory_limit(MEMORY_LIMIT);
        runtime.set_max_stack_size(256 << 10);
        runtime.set_interrupt_handler(Some(Box::new(out_of_time)));
        let context = Context::full(&runtime).map_err(|e| e.to_string())?;
        let run = context.with(|ctx| {
            register(&ctx).map_err(|e| describe(&ctx, e))?;
            // The script becomes a function body, so a top-level `return`
            // works; it starts on line 1 so error line numbers match.
            let wrapped = format!(
                "(json) => JSON.stringify(((post) => {{ {source}\n}})(JSON.parse(json)) ?? null)"
            );
            let run: Function = ctx.eval(wrapped).map_err(|e| describe(&ctx, e))?;
            Ok::<_, String>(Persistent::save(&ctx, run))
        })?;
        Ok(Box::new(Script { run, context, _runtime: runtime }))
    }
}

impl Prepared for Script {
    fn run(&self, post: &Post) -> Result<Action, String> {
        let json = serde_json::to_string(post).map_err(|e| e.to_string())?;
        let out: String = self.context.with(|ctx| {
            let run = self.run.clone().restore(&ctx).map_err(|e| describe(&ctx, e))?;
            run.call((json,)).map_err(|e| describe(&ctx, e))
        })?;
        Action::from_json(serde_json::from_str(&out).map_err(|e| e.to_string())?)
    }
}
