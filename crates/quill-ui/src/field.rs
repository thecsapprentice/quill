//! Text fields that follow the 3a spike's rules (`spikes/gui/FINDINGS.md`):
//!
//! - **Uncontrolled.** The input owns its text while the writer types; the
//!   app hears it through `oninput` and never writes it back, because a
//!   controlled input on the desktop loses keystrokes. To replace the text,
//!   change `generation`: the input remounts with `initial`.
//! - **No browser writing help.** `writingsuggestions="false"` and
//!   `autocomplete="off"`, except passwords, which keep the password
//!   manager's hints.

use dioxus::prelude::*;

/// What kind of text a field takes, which sets its browser hints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A name, handle or address: no spellcheck, no capitals.
    Name,
    /// A new password (signing up).
    NewPassword,
    /// An existing password (signing in, unlocking).
    Password,
    /// Ordinary text, spellchecked.
    Text,
}

#[derive(Props, Clone, PartialEq)]
pub struct FieldProps {
    /// The visible label.
    pub label: String,
    /// Unique on the page: ties the label and hint to the input.
    pub id: String,
    pub kind: Kind,
    /// Text to start with.
    #[props(default)]
    pub initial: String,
    /// Change to replace the text with `initial` (remounts the input).
    #[props(default)]
    pub generation: u32,
    /// A line under the field: a rule, or what's wrong.
    #[props(default)]
    pub hint: Option<String>,
    /// Whether `hint` says something is wrong.
    #[props(default)]
    pub invalid: bool,
    #[props(default)]
    pub placeholder: String,
    #[props(default)]
    pub autofocus: bool,
    pub oninput: EventHandler<String>,
    /// Enter in the field.
    #[props(default)]
    pub onenter: Option<EventHandler<()>>,
}

#[component]
pub fn Field(props: FieldProps) -> Element {
    let (input_type, autocomplete, spellcheck) = match props.kind {
        Kind::Name => ("text", "off", "false"),
        Kind::NewPassword => ("password", "new-password", "false"),
        Kind::Password => ("password", "current-password", "false"),
        Kind::Text => ("text", "off", "true"),
    };
    let hint_id = format!("{}-hint", props.id);
    let key = format!("{}-{}", props.id, props.generation);
    let onenter = props.onenter;
    rsx! {
        div { class: if props.invalid { "field invalid" } else { "field" },
            label { r#for: "{props.id}", "{props.label}" }
            // A list of one, keyed: a new key remounts the input (Dioxus
            // ignores `key` on a lone element).
            for k in [key] {
                input {
                    key: "{k}",
                    id: "{props.id}",
                    r#type: input_type,
                    initial_value: "{props.initial}",
                    placeholder: "{props.placeholder}",
                    autocomplete: autocomplete,
                    "writingsuggestions": "false",
                    spellcheck: spellcheck,
                    autocapitalize: if props.kind == Kind::Text { "sentences" } else { "off" },
                    autofocus: props.autofocus,
                    aria_invalid: if props.invalid { "true" } else { "false" },
                    aria_describedby: "{hint_id}",
                    oninput: move |e| props.oninput.call(e.value()),
                    onkeydown: move |e: KeyboardEvent| {
                        if e.key() == Key::Enter
                            && let Some(h) = onenter
                        {
                            e.prevent_default();
                            h.call(());
                        }
                    },
                }
            }
            p { id: "{hint_id}", class: "hint", aria_live: "polite",
                if let Some(h) = &props.hint { "{h}" }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct AreaProps {
    pub label: String,
    pub id: String,
    #[props(default)]
    pub initial: String,
    #[props(default)]
    pub generation: u32,
    #[props(default)]
    pub hint: Option<String>,
    pub oninput: EventHandler<String>,
}

/// A multi-line field (a persona's summary), spellchecked.
#[component]
pub fn Area(props: AreaProps) -> Element {
    let hint_id = format!("{}-hint", props.id);
    let key = format!("{}-{}", props.id, props.generation);
    rsx! {
        div { class: "field",
            label { r#for: "{props.id}", "{props.label}" }
            for k in [key] {
                textarea {
                    key: "{k}",
                    id: "{props.id}",
                    rows: "4",
                    initial_value: "{props.initial}",
                    autocomplete: "off",
                    "writingsuggestions": "false",
                    spellcheck: "true",
                    aria_describedby: "{hint_id}",
                    oninput: move |e| props.oninput.call(e.value()),
                }
            }
            p { id: "{hint_id}", class: "hint",
                if let Some(h) = &props.hint { "{h}" }
            }
        }
    }
}
