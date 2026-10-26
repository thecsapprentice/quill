//! The writing client's views and app state, shared by the desktop GUI
//! (`quill-gui`) and, later, the web client. Nothing here is platform code:
//! views reach `quill-core` only through the [`Backend`] trait, which each
//! client implements.

pub mod backend;

pub use backend::Backend;
