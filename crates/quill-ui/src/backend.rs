//! What the views need from a client: implemented over `quill-core` by
//! `quill-gui` (and later the web client), and by a fake in tests.

/// The client's side of the app. Grows as the views do.
pub trait Backend {}
