//! The writing client's views and app state, shared by the desktop GUI
//! (`quill-gui`) and, later, the web client. Nothing here is platform code:
//! views reach `quill-core` only through the [`Backend`] trait, which each
//! client implements.

pub mod backend;
pub mod field;

pub use backend::Backend;

/// The client's stylesheet, put in the page by the app's root view.
pub const STYLE: &str = include_str!("style.css");
