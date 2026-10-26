//! Luau (Roblox's Lua) through `mlua`: built for untrusted code once its
//! sandbox mode is switched on.

use mlua::{Function, Lua, LuaSerdeExt, Value, VmState};
use serde_json::json;

use super::{Engine, MEMORY_LIMIT, Prepared, TIME_LIMIT_MESSAGE, out_of_time};
use crate::context::{Action, Post};
use crate::text;

pub struct Luau;

struct Script {
    lua: Lua,
    chunk: Function,
}

fn setup() -> mlua::Result<Lua> {
    // `Lua::new` loads only Luau's safe libraries: no `io`, `os.execute` or
    // `require` of native modules.
    let lua = Lua::new();
    lua.set_memory_limit(MEMORY_LIMIT)?;
    lua.set_interrupt(|_| {
        if out_of_time() { Err(mlua::Error::runtime(TIME_LIMIT_MESSAGE)) } else { Ok(VmState::Continue) }
    });

    let t = lua.create_table()?;
    let err = mlua::Error::runtime;
    t.set("matches", lua.create_function(move |_, (p, s): (String, String)| text::matches(&p, &s).map_err(err))?)?;
    t.set(
        "find_all",
        lua.create_function(move |_, (p, s): (String, String)| text::find_all(&p, &s).map_err(err))?,
    )?;
    t.set(
        "captures",
        lua.create_function(move |lua, (p, s): (String, String)| {
            lua.to_value(&text::captures(&p, &s).map_err(err)?)
        })?,
    )?;
    t.set(
        "replace",
        lua.create_function(move |_, (p, s, w): (String, String, String)| text::replace(&p, &s, &w).map_err(err))?,
    )?;
    t.set(
        "find_words",
        lua.create_function(move |_, (list, s): (Vec<String>, String)| text::find_words(&list, &s).map_err(err))?,
    )?;
    t.set("normalize", lua.create_function(|_, s: String| Ok(text::normalize(&s)))?)?;
    t.set("skeleton", lua.create_function(|_, s: String| Ok(text::skeleton(&s)))?)?;
    t.set("links", lua.create_function(|lua, s: String| lua.to_value(&text::links(&s)))?)?;
    t.set("words", lua.create_function(|_, s: String| Ok(text::words(&s)))?)?;
    t.set("mixed_script_words", lua.create_function(|_, s: String| Ok(text::mixed_script_words(&s)))?)?;
    t.set("similarity", lua.create_function(|_, (a, b): (String, String)| Ok(text::similarity(&a, &b)))?)?;
    t.set("caps_ratio", lua.create_function(|_, s: String| Ok(text::caps_ratio(&s)))?)?;

    let globals = lua.globals();
    globals.set("text", t)?;
    globals.set(
        "reject",
        lua.create_function(|lua, message: String| lua.to_value(&json!({ "action": "reject", "message": message })))?,
    )?;
    globals.set(
        "flag",
        lua.create_function(|lua, (reason, category): (String, String)| {
            lua.to_value(&json!({ "action": "flag", "reason": reason, "category": category }))
        })?,
    )?;
    // Makes every global read-only and gives each script its own environment.
    lua.sandbox(true)?;
    Ok(lua)
}

impl Engine for Luau {
    fn name(&self) -> &'static str {
        "Luau"
    }

    fn extension(&self) -> &'static str {
        "luau"
    }

    fn prepare(&self, source: &str) -> Result<Box<dyn Prepared>, String> {
        let lua = setup().map_err(|e| e.to_string())?;
        // `post` arrives as the chunk's argument; kept on line 1 so error
        // line numbers match the admin's file.
        let chunk = lua
            .load(format!("local post = ...; {source}"))
            .set_name("script")
            .into_function()
            .map_err(|e| e.to_string())?;
        Ok(Box::new(Script { lua, chunk }))
    }
}

impl Prepared for Script {
    fn run(&self, post: &Post) -> Result<Action, String> {
        let post = self.lua.to_value(post).map_err(|e| e.to_string())?;
        let result: Value = self.chunk.call(post).map_err(|e| e.to_string())?;
        Action::from_json(self.lua.from_value(result).map_err(|e| e.to_string())?)
    }
}
