//! One adapter per candidate engine, behind a common trait so the harness
//! runs the same samples through each.

use std::cell::Cell;
use std::time::{Duration, Instant};

use crate::context::{Action, Post};

#[cfg(feature = "js")]
mod js;
#[cfg(feature = "luau")]
mod luau;
#[cfg(feature = "rhai")]
mod rhai;
#[cfg(feature = "rules")]
mod rules;
#[cfg(feature = "starlark")]
mod starlark;

/// Limits every engine enforces, as closely as it can (see FINDINGS.md).
pub const MAX_OPERATIONS: u64 = 1_000_000;
pub const MEMORY_LIMIT: usize = 16 << 20;
pub const MAX_STRING: usize = 1 << 20;

pub trait Engine {
    fn name(&self) -> &'static str;
    /// The folder under `scripts/`, which is also the file extension.
    fn extension(&self) -> &'static str;
    /// Compiles a script, refusing syntax errors before it ever runs.
    fn prepare(&self, source: &str) -> Result<Box<dyn Prepared>, String>;
}

pub trait Prepared {
    fn run(&self, post: &Post) -> Result<Action, String>;
}

pub fn all() -> Vec<Box<dyn Engine>> {
    #[allow(unused_mut)]
    let mut engines: Vec<Box<dyn Engine>> = Vec::new();
    #[cfg(feature = "rules")]
    engines.push(Box::new(rules::Rules));
    #[cfg(feature = "rhai")]
    engines.push(Box::new(rhai::Rhai));
    #[cfg(feature = "luau")]
    engines.push(Box::new(luau::Luau));
    #[cfg(feature = "starlark")]
    engines.push(Box::new(starlark::Starlark));
    #[cfg(feature = "js")]
    engines.push(Box::new(js::Js));
    engines
}

thread_local! {
    static DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Starts the wall-clock cap for the script about to run on this thread.
pub fn start_clock(cap: Duration) {
    DEADLINE.with(|d| d.set(Some(Instant::now() + cap)));
}

/// Polled by each engine's interrupt hook.
pub fn out_of_time() -> bool {
    DEADLINE.with(|d| d.get().is_some_and(|deadline| Instant::now() > deadline))
}

pub const TIME_LIMIT_MESSAGE: &str = "script ran past its time limit";
