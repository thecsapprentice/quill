//! The writing client's views and app state, shared by the desktop GUI
//! (`quill-gui`) and, later, the web client. Nothing here is platform code:
//! views reach `quill-core` only through the [`Backend`] trait, which each
//! client implements.

pub mod app;
pub mod backend;
#[cfg(test)]
mod fake;
pub mod field;
mod first_run;
mod frame;
pub mod model;
mod personas;
pub mod rules;
mod settings;
#[cfg(test)]
mod view_tests;

pub use app::{App, Ui};
pub use backend::Backend;

/// The client's stylesheet, for the platform to put in the page's head.
pub const STYLE: &str = include_str!("style.css");
