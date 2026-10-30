//! A driver for interaction tests: the app in a headless document, worked
//! the way a person would, by button labels and field ids.
//!
//! The document is built from the same mutations the desktop webview
//! applies, and events go in as the webview's own JSON, through the same
//! `HtmlEvent` path dioxus-desktop uses. So a test exercises the views'
//! real handlers, tasks and model updates, with the fake backend or with
//! `quill-gui`'s native one against a real server (`quill-e2e`).

// Test support: a failed step panics with what the window shows.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    time::{Duration, Instant},
};

use dioxus::prelude::*;
use dioxus_core::{
    AttributeValue, ElementId, Template, TemplateAttribute, TemplateNode, WriteMutations,
};
use dioxus_html::HtmlEvent;

use crate::{App, Backend, Ui, app::StartWith, model::Model};

mod sim;
pub use sim::{Acceptance, Sim, SimServer};

/// How long a wait gives up after.
const PATIENCE: Duration = Duration::from_secs(20);

#[derive(Debug, Clone)]
enum Kind {
    Element {
        tag: String,
        attrs: Vec<(String, String)>,
    },
    Text(String),
    Placeholder,
}

#[derive(Debug, Clone)]
struct Node {
    kind: Kind,
    children: Vec<usize>,
    parent: Option<usize>,
    listeners: HashSet<String>,
    id: Option<ElementId>,
}

/// The headless document: an arena of nodes, kept up to date from the
/// virtual DOM's mutations (the webview interpreter's stack machine).
#[derive(Debug)]
struct Doc {
    nodes: Vec<Node>,
    ids: HashMap<ElementId, usize>,
    stack: Vec<usize>,
}

impl Doc {
    fn new() -> Self {
        let mut doc = Self {
            nodes: Vec::new(),
            ids: HashMap::new(),
            stack: Vec::new(),
        };
        let root = doc.add(Kind::Element {
            tag: "main".into(),
            attrs: Vec::new(),
        });
        doc.set_id(root, ElementId(0));
        doc
    }

    fn add(&mut self, kind: Kind) -> usize {
        self.nodes.push(Node {
            kind,
            children: Vec::new(),
            parent: None,
            listeners: HashSet::new(),
            id: None,
        });
        self.nodes.len() - 1
    }

    fn set_id(&mut self, node: usize, id: ElementId) {
        self.nodes[node].id = Some(id);
        self.ids.insert(id, node);
    }

    fn node(&self, id: ElementId) -> usize {
        *self
            .ids
            .get(&id)
            .unwrap_or_else(|| panic!("no node {id:?}"))
    }

    fn template(&mut self, t: &TemplateNode) -> usize {
        match t {
            TemplateNode::Element {
                tag,
                attrs,
                children,
                ..
            } => {
                let attrs = attrs
                    .iter()
                    .filter_map(|a| match a {
                        TemplateAttribute::Static { name, value, .. } => {
                            Some((name.to_string(), value.to_string()))
                        }
                        TemplateAttribute::Dynamic { .. } => None,
                    })
                    .collect();
                let el = self.add(Kind::Element {
                    tag: tag.to_string(),
                    attrs,
                });
                for child in children.iter() {
                    let c = self.template(child);
                    self.append(el, c);
                }
                el
            }
            TemplateNode::Text { text } => self.add(Kind::Text(text.to_string())),
            TemplateNode::Dynamic { .. } => self.add(Kind::Placeholder),
        }
    }

    fn detach(&mut self, node: usize) {
        if let Some(p) = self.nodes[node].parent.take() {
            self.nodes[p].children.retain(|c| *c != node);
        }
    }

    fn append(&mut self, parent: usize, child: usize) {
        self.detach(child);
        self.nodes[child].parent = Some(parent);
        self.nodes[parent].children.push(child);
    }

    /// Puts `new` where `old` is, in its parent.
    fn replace(&mut self, old: usize, new: Vec<usize>) {
        let Some(parent) = self.nodes[old].parent else {
            return;
        };
        for n in &new {
            self.detach(*n);
        }
        let at = self.nodes[parent]
            .children
            .iter()
            .position(|c| *c == old)
            .unwrap_or(0);
        self.nodes[old].parent = None;
        for n in &new {
            self.nodes[*n].parent = Some(parent);
        }
        self.nodes[parent].children.splice(at..=at, new);
    }

    fn insert(&mut self, anchor: usize, new: Vec<usize>, after: bool) {
        let Some(parent) = self.nodes[anchor].parent else {
            return;
        };
        for n in &new {
            self.detach(*n);
            self.nodes[*n].parent = Some(parent);
        }
        let at = self.nodes[parent]
            .children
            .iter()
            .position(|c| *c == anchor)
            .unwrap_or(0)
            + usize::from(after);
        self.nodes[parent].children.splice(at..at, new);
    }

    fn pop(&mut self, m: usize) -> Vec<usize> {
        let at = self.stack.len() - m;
        self.stack.split_off(at)
    }

    fn child_at(&self, path: &[u8]) -> usize {
        let mut node = *self.stack.last().expect("an empty stack");
        for step in path {
            node = self.nodes[node].children[usize::from(*step)];
        }
        node
    }

    fn set_attr(&mut self, node: usize, name: &str, value: Option<String>) {
        if let Kind::Element { attrs, .. } = &mut self.nodes[node].kind {
            attrs.retain(|(n, _)| n != name);
            if let Some(v) = value {
                attrs.push((name.to_owned(), v));
            }
        }
    }

    fn attr(&self, node: usize, name: &str) -> Option<&str> {
        match &self.nodes[node].kind {
            Kind::Element { attrs, .. } => attrs
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str()),
            _ => None,
        }
    }

    fn tag(&self, node: usize) -> Option<&str> {
        match &self.nodes[node].kind {
            Kind::Element { tag, .. } => Some(tag),
            _ => None,
        }
    }

    /// The text inside `node`, whitespace collapsed.
    fn text_of(&self, node: usize) -> String {
        fn walk(doc: &Doc, n: usize, out: &mut String) {
            match &doc.nodes[n].kind {
                Kind::Text(t) => out.push_str(t),
                // Not shown: a stylesheet's text.
                Kind::Element { tag, .. } if tag == "style" => {}
                Kind::Element { .. } => {
                    for c in &doc.nodes[n].children {
                        walk(doc, *c, out);
                    }
                    out.push(' ');
                }
                Kind::Placeholder => {}
            }
        }
        let mut out = String::new();
        walk(self, node, &mut out);
        out.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// The nodes attached under the root, in document order.
    fn attached(&self) -> Vec<usize> {
        fn walk(doc: &Doc, n: usize, out: &mut Vec<usize>) {
            out.push(n);
            for c in &doc.nodes[n].children {
                walk(doc, *c, out);
            }
        }
        let mut out = Vec::new();
        walk(self, self.node(ElementId(0)), &mut out);
        out
    }

    /// The nearest node at or above `node` that the virtual DOM knows by
    /// id with a listener: where the webview would send its event.
    fn event_target(&self, mut node: usize) -> ElementId {
        loop {
            if let (Some(id), false) = (self.nodes[node].id, self.nodes[node].listeners.is_empty())
            {
                return id;
            }
            node = self.nodes[node]
                .parent
                .expect("no element with a listener above the target");
        }
    }
}

fn attr_text(value: &AttributeValue) -> Option<String> {
    match value {
        AttributeValue::Text(t) => Some(t.clone()),
        AttributeValue::Float(f) => Some(f.to_string()),
        AttributeValue::Int(i) => Some(i.to_string()),
        // As the webview does: a false boolean attribute is absent.
        AttributeValue::Bool(true) => Some("true".into()),
        AttributeValue::Bool(false) | AttributeValue::None => None,
        AttributeValue::Listener(_) | AttributeValue::Any(_) => None,
    }
}

impl WriteMutations for Doc {
    fn append_children(&mut self, id: ElementId, m: usize) {
        let parent = self.node(id);
        for c in self.pop(m) {
            self.append(parent, c);
        }
    }

    fn assign_node_id(&mut self, path: &'static [u8], id: ElementId) {
        let node = self.child_at(path);
        self.set_id(node, id);
    }

    fn create_placeholder(&mut self, id: ElementId) {
        let n = self.add(Kind::Placeholder);
        self.set_id(n, id);
        self.stack.push(n);
    }

    fn create_text_node(&mut self, value: &str, id: ElementId) {
        let n = self.add(Kind::Text(value.to_owned()));
        self.set_id(n, id);
        self.stack.push(n);
    }

    fn load_template(&mut self, template: Template, index: usize, id: ElementId) {
        let n = self.template(&template.roots[index]);
        self.set_id(n, id);
        self.stack.push(n);
    }

    fn replace_node_with(&mut self, id: ElementId, m: usize) {
        let old = self.node(id);
        let new = self.pop(m);
        self.replace(old, new);
    }

    fn replace_placeholder_with_nodes(&mut self, path: &'static [u8], m: usize) {
        let new = self.pop(m);
        let old = self.child_at(path);
        self.replace(old, new);
    }

    fn insert_nodes_after(&mut self, id: ElementId, m: usize) {
        let anchor = self.node(id);
        let new = self.pop(m);
        self.insert(anchor, new, true);
    }

    fn insert_nodes_before(&mut self, id: ElementId, m: usize) {
        let anchor = self.node(id);
        let new = self.pop(m);
        self.insert(anchor, new, false);
    }

    fn set_attribute(
        &mut self,
        name: &'static str,
        _ns: Option<&'static str>,
        value: &AttributeValue,
        id: ElementId,
    ) {
        let node = self.node(id);
        self.set_attr(node, name, attr_text(value));
    }

    fn set_node_text(&mut self, value: &str, id: ElementId) {
        let node = self.node(id);
        self.nodes[node].kind = Kind::Text(value.to_owned());
    }

    fn create_event_listener(&mut self, name: &'static str, id: ElementId) {
        let node = self.node(id);
        self.nodes[node].listeners.insert(name.to_owned());
    }

    fn remove_event_listener(&mut self, name: &'static str, id: ElementId) {
        let node = self.node(id);
        self.nodes[node].listeners.remove(name);
    }

    fn remove_node(&mut self, id: ElementId) {
        let node = self.node(id);
        self.detach(node);
    }

    fn push_root(&mut self, id: ElementId) {
        let node = self.node(id);
        self.stack.push(node);
    }
}

/// The app, running headless, and what it shows.
pub struct Driver {
    dom: VirtualDom,
    doc: Doc,
}

impl Driver {
    /// The app as it opens with `backend`.
    pub fn new(backend: Rc<dyn Backend>) -> Self {
        Self::start(backend, None)
    }

    /// The app starting from `model` instead of the backend's settings.
    pub fn with_model(backend: Rc<dyn Backend>, model: Model) -> Self {
        Self::start(backend, Some(model))
    }

    fn start(backend: Rc<dyn Backend>, model: Option<Model>) -> Self {
        dioxus_html::set_event_converter(Box::new(dioxus_html::SerializedHtmlEventConverter));
        let mut dom = VirtualDom::new(App);
        dom.provide_root_context(Ui(backend));
        if let Some(m) = model {
            dom.provide_root_context(StartWith(m));
        }
        let mut doc = Doc::new();
        dom.rebuild(&mut doc);
        Self { dom, doc }
    }

    /// Lets pending work (backend calls, timers, re-renders) run until
    /// nothing has happened for a moment.
    pub async fn settle(&mut self) {
        loop {
            let quiet = tokio::time::timeout(Duration::from_millis(60), self.dom.wait_for_work())
                .await
                .is_err();
            self.dom.render_immediate(&mut self.doc);
            if quiet {
                return;
            }
        }
    }

    /// Waits until `ready` holds of what's shown, letting work run.
    pub async fn wait_until(&mut self, what: &str, ready: impl Fn(&Self) -> bool) {
        let start = Instant::now();
        loop {
            self.settle().await;
            if ready(self) {
                return;
            }
            assert!(
                start.elapsed() < PATIENCE,
                "gave up waiting for {what}; the window shows:\n{}",
                self.text()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Waits until `text` is shown.
    pub async fn wait_for_text(&mut self, text: &str) {
        self.wait_until(&format!("\"{text}\""), |d| d.has_text(text))
            .await;
    }

    /// Everything the window shows, as text.
    pub fn text(&self) -> String {
        self.doc.text_of(self.doc.node(ElementId(0)))
    }

    pub fn has_text(&self, text: &str) -> bool {
        self.text().contains(text)
    }

    /// The main heading (the screen's title).
    pub fn heading(&self) -> Option<String> {
        self.doc
            .attached()
            .into_iter()
            .find(|n| self.doc.tag(*n) == Some("h1"))
            .map(|n| self.doc.text_of(n))
    }

    fn by_id(&self, id: &str) -> usize {
        self.doc
            .attached()
            .into_iter()
            .find(|n| self.doc.attr(*n, "id") == Some(id))
            .unwrap_or_else(|| panic!("no #{id}; the window shows:\n{}", self.text()))
    }

    /// A button's accessible name: its `aria-label`, else its text, as a
    /// screen reader would read it.
    fn name_of(&self, n: usize) -> String {
        self.doc
            .attr(n, "aria-label")
            .map_or_else(|| self.doc.text_of(n), str::to_owned)
    }

    fn button(&self, label: &str) -> Option<usize> {
        self.doc
            .attached()
            .into_iter()
            .find(|n| self.doc.tag(*n) == Some("button") && self.name_of(*n) == label)
    }

    /// The `n`th button labelled `label`, counting from 0 in page order.
    pub async fn click_nth(&mut self, label: &str, n: usize) {
        let b = self
            .doc
            .attached()
            .into_iter()
            .filter(|b| self.doc.tag(*b) == Some("button") && self.name_of(*b) == label)
            .nth(n)
            .unwrap_or_else(|| {
                panic!(
                    "no button \"{label}\" #{n}; the window shows:\n{}",
                    self.text()
                )
            });
        assert!(
            self.doc.attr(b, "disabled").is_none(),
            "\"{label}\" #{n} is disabled"
        );
        self.send(b, "click", mouse());
        self.settle().await;
    }

    /// The id of the first element whose id starts with `prefix`.
    pub fn field_id_with_prefix(&self, prefix: &str) -> String {
        self.doc
            .attached()
            .into_iter()
            .find_map(|n| self.doc.attr(n, "id").filter(|id| id.starts_with(prefix)))
            .unwrap_or_else(|| panic!("no #{prefix}…; the window shows:\n{}", self.text()))
            .to_owned()
    }

    /// An attribute of the app's root element (`data-theme`, say).
    pub fn root_attr(&self, name: &str) -> Option<String> {
        self.doc
            .attached()
            .into_iter()
            .find(|n| self.doc.attr(*n, "class") == Some("quill"))
            .and_then(|n| self.doc.attr(n, name).map(str::to_owned))
    }

    /// The text of the page's own stylesheet (text size, say).
    pub fn page_style(&self) -> String {
        self.doc
            .attached()
            .into_iter()
            .filter(|n| self.doc.tag(*n) == Some("style"))
            .flat_map(|n| self.doc.nodes[n].children.clone())
            .filter_map(|c| match &self.doc.nodes[c].kind {
                Kind::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    /// Whether any element with this tag is in the window (`script`, say).
    pub fn has_element(&self, tag: &str) -> bool {
        self.doc
            .attached()
            .into_iter()
            .any(|n| self.doc.tag(n) == Some(tag))
    }

    pub fn has_button(&self, label: &str) -> bool {
        self.button(label).is_some()
    }

    pub fn has_field(&self, id: &str) -> bool {
        self.doc
            .attached()
            .into_iter()
            .any(|n| self.doc.attr(n, "id") == Some(id))
    }

    /// Whether the button labelled `label` is disabled.
    pub fn button_disabled(&self, label: &str) -> bool {
        let b = self
            .button(label)
            .unwrap_or_else(|| panic!("no button \"{label}\"; the window shows:\n{}", self.text()));
        self.doc.attr(b, "disabled").is_some()
    }

    /// An attribute of the element with `id`.
    pub fn attr(&self, id: &str, name: &str) -> Option<String> {
        self.doc.attr(self.by_id(id), name).map(str::to_owned)
    }

    fn send(&mut self, node: usize, name: &str, data: serde_json::Value) {
        let target = self.doc.event_target(node);
        let event: HtmlEvent = serde_json::from_value(serde_json::json!({
            "element": target.0,
            "name": name,
            "bubbles": !matches!(name, "focus" | "blur" | "mounted"),
            "data": data,
        }))
        .expect("event JSON as the webview sends it");
        let event = dioxus_core::Event::new(event.data.into_any(), event.bubbles);
        self.dom.runtime().handle_event(name, event, target);
        self.dom.render_immediate(&mut self.doc);
    }

    /// Clicks the button labelled `label`. A disabled button can't be
    /// clicked, as in the window, so that's a failure here.
    pub async fn click(&mut self, label: &str) {
        let b = self
            .button(label)
            .unwrap_or_else(|| panic!("no button \"{label}\"; the window shows:\n{}", self.text()));
        assert!(
            self.doc.attr(b, "disabled").is_none(),
            "\"{label}\" is disabled; the window shows:\n{}",
            self.text()
        );
        self.send(b, "click", mouse());
        self.settle().await;
    }

    /// Clicks the first button whose text contains `text` (a list row,
    /// whose label is its title and details).
    pub async fn click_containing(&mut self, text: &str) {
        let b = self
            .doc
            .attached()
            .into_iter()
            .find(|n| self.doc.tag(*n) == Some("button") && self.doc.text_of(*n).contains(text))
            .unwrap_or_else(|| {
                panic!(
                    "no button with \"{text}\"; the window shows:\n{}",
                    self.text()
                )
            });
        self.send(b, "click", mouse());
        self.settle().await;
    }

    /// Whether a button's text contains `text`.
    pub fn has_button_containing(&self, text: &str) -> bool {
        self.doc
            .attached()
            .into_iter()
            .any(|n| self.doc.tag(n) == Some("button") && self.doc.text_of(n).contains(text))
    }

    /// Clicks the element with `id` (a button among others with the same
    /// label, such as a row's Has).
    pub async fn click_id(&mut self, id: &str) {
        let el = self.by_id(id);
        assert!(
            self.doc.attr(el, "disabled").is_none(),
            "#{id} is disabled; the window shows:\n{}",
            self.text()
        );
        self.send(el, "click", mouse());
        self.settle().await;
    }

    /// Whether the element with `id` is pressed (`aria-pressed`).
    pub fn pressed(&self, id: &str) -> bool {
        self.attr(id, "aria-pressed").as_deref() == Some("true")
    }

    /// Scrolls the element with class `class` to its end, as reaching the
    /// bottom of a list does.
    pub async fn scroll_to_end(&mut self, class: &str) {
        let el = self
            .doc
            .attached()
            .into_iter()
            .find(|n| {
                self.doc
                    .attr(*n, "class")
                    .is_some_and(|c| c.split_whitespace().any(|w| w == class))
            })
            .unwrap_or_else(|| panic!("no .{class}; the window shows:\n{}", self.text()));
        let data = dioxus_html::SerializedScrollData {
            scroll_top: 1000.0,
            scroll_left: 0.0,
            scroll_width: 400,
            scroll_height: 1600,
            client_width: 400,
            client_height: 600,
        };
        self.send(
            el,
            "scroll",
            serde_json::to_value(data).expect("scroll data"),
        );
        self.settle().await;
    }

    /// Types `text` into the field with `id`, replacing what's there.
    pub async fn type_into(&mut self, id: &str, text: &str) {
        let field = self.by_id(id);
        self.doc.set_attr(field, "value", Some(text.to_owned()));
        self.send(field, "input", form(text));
        self.settle().await;
    }

    /// Presses Enter in the field with `id`.
    pub async fn enter(&mut self, id: &str) {
        let field = self.by_id(id);
        let key = dioxus_html::SerializedKeyboardData::new(
            dioxus_html::Key::Enter,
            dioxus_html::Code::Enter,
            dioxus_html::Location::Standard,
            false,
            dioxus_html::Modifiers::empty(),
            false,
        );
        self.send(
            field,
            "keydown",
            serde_json::to_value(key).expect("keyboard data"),
        );
        self.settle().await;
    }

    /// Ticks or unticks the checkbox with `id`.
    pub async fn set_checked(&mut self, id: &str, on: bool) {
        let field = self.by_id(id);
        self.send(field, "change", form(if on { "true" } else { "false" }));
        self.settle().await;
    }

    /// Chooses the option reading `label` in the select with `id`, as a
    /// writer picks from the list.
    pub async fn choose_labelled(&mut self, id: &str, label: &str) {
        let select = self.by_id(id);
        let value = self.doc.nodes[select]
            .children
            .iter()
            .find(|c| self.doc.tag(**c) == Some("option") && self.doc.text_of(**c) == label)
            .and_then(|c| self.doc.attr(*c, "value"))
            .unwrap_or_else(|| panic!("#{id} has no option {label:?}"))
            .to_owned();
        self.choose(id, &value).await;
    }

    /// Chooses the option with `value` in the select with `id`.
    pub async fn choose(&mut self, id: &str, value: &str) {
        let field = self.by_id(id);
        self.send(field, "change", form(value));
        self.settle().await;
    }
}

fn mouse() -> serde_json::Value {
    serde_json::to_value(dioxus_html::SerializedMouseData::default()).expect("mouse data")
}

fn form(value: &str) -> serde_json::Value {
    serde_json::json!({ "value": value, "values": [], "valid": true })
}
