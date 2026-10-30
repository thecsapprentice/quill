//! Mounted elements for the driver. The webview gives `onmounted`
//! handlers a live element to scroll or focus; the serialized converter
//! the driver otherwise uses gives one that refuses. This one records
//! what the app asked of it, so tests can check scrolling and focus.

use std::{cell::RefCell, future::Future, pin::Pin};

use dioxus_html::{
    AnimationData, CancelData, ClipboardData, CompositionData, DragData, FocusData, FormData,
    HtmlEventConverter, ImageData, KeyboardData, MediaData, MountedData, MountedResult, MouseData,
    PlatformEventData, PointerData, RenderedElementBacking, ResizeData, ScrollData,
    ScrollToOptions, SelectionData, SerializedHtmlEventConverter, ToggleData, TouchData,
    TransitionData, VisibleData, WheelData, geometry::PixelsVector2D,
};

/// What the app asked of a mounted element.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Asked {
    /// Scroll it into view (`scroll_to`).
    IntoView,
    /// Scroll its own content to this offset (`scroll`).
    ScrollTo {
        x: f64,
        y: f64,
    },
    Focus,
}

thread_local! {
    /// Requests by document node, in order. Each test runs on its own
    /// thread, so drivers in parallel tests don't mix.
    static ASKED: RefCell<Vec<(usize, Asked)>> = const { RefCell::new(Vec::new()) };
}

/// Takes the requests recorded so far.
pub(super) fn take() -> Vec<(usize, Asked)> {
    ASKED.with(|a| std::mem::take(&mut *a.borrow_mut()))
}

fn record(node: usize, asked: Asked) {
    ASKED.with(|a| a.borrow_mut().push((node, asked)));
}

/// The mounted event's data as the driver sends it: which node mounted.
pub(super) struct Mounted(pub usize);

struct Backing(usize);

impl RenderedElementBacking for Backing {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn scroll_to(
        &self,
        _options: ScrollToOptions,
    ) -> Pin<Box<dyn Future<Output = MountedResult<()>>>> {
        record(self.0, Asked::IntoView);
        Box::pin(async { Ok(()) })
    }

    fn scroll(
        &self,
        coordinates: PixelsVector2D,
        _behavior: dioxus_html::ScrollBehavior,
    ) -> Pin<Box<dyn Future<Output = MountedResult<()>>>> {
        record(
            self.0,
            Asked::ScrollTo {
                x: coordinates.x,
                y: coordinates.y,
            },
        );
        Box::pin(async { Ok(()) })
    }

    fn set_focus(&self, focus: bool) -> Pin<Box<dyn Future<Output = MountedResult<()>>>> {
        if focus {
            record(self.0, Asked::Focus);
        }
        Box::pin(async { Ok(()) })
    }
}

/// The serialized converter, but mounted elements record what they're asked.
pub(super) struct Converter;

macro_rules! delegate {
    ($($f:ident -> $t:ty),* $(,)?) => {
        $(fn $f(&self, event: &PlatformEventData) -> $t {
            SerializedHtmlEventConverter.$f(event)
        })*
    };
}

impl HtmlEventConverter for Converter {
    fn convert_mounted_data(&self, event: &PlatformEventData) -> MountedData {
        match event.downcast::<Mounted>() {
            Some(Mounted(node)) => MountedData::new(Backing(*node)),
            None => SerializedHtmlEventConverter.convert_mounted_data(event),
        }
    }

    delegate! {
        convert_animation_data -> AnimationData,
        convert_cancel_data -> CancelData,
        convert_clipboard_data -> ClipboardData,
        convert_composition_data -> CompositionData,
        convert_drag_data -> DragData,
        convert_focus_data -> FocusData,
        convert_form_data -> FormData,
        convert_image_data -> ImageData,
        convert_keyboard_data -> KeyboardData,
        convert_media_data -> MediaData,
        convert_mouse_data -> MouseData,
        convert_pointer_data -> PointerData,
        convert_resize_data -> ResizeData,
        convert_scroll_data -> ScrollData,
        convert_selection_data -> SelectionData,
        convert_toggle_data -> ToggleData,
        convert_touch_data -> TouchData,
        convert_transition_data -> TransitionData,
        convert_visible_data -> VisibleData,
        convert_wheel_data -> WheelData,
    }
}
