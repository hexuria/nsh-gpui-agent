use serde::{Deserialize, Serialize};

/// A synthetic mouse pointer event phase.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SynthPointerPhase {
    /// A button was pressed.
    Down,
    /// The pointer moved.
    Move,
    /// A button was released.
    Up,
}

/// A synthetic keyboard event phase.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SynthKeyPhase {
    /// A key was pressed.
    Down,
    /// A key was released.
    Up,
}

/// An untrusted DOM event dispatched by a web page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(crate) enum SynthEvent {
    /// Dispatches pointer and mouse events at viewport-relative CSS pixels.
    Pointer {
        /// The event phase.
        phase: SynthPointerPhase,
        /// The horizontal CSS-pixel coordinate.
        x: f32,
        /// The vertical CSS-pixel coordinate.
        y: f32,
        /// The pressed mouse buttons bit mask.
        buttons: u32,
    },
    /// Dispatches a wheel event at viewport-relative CSS pixels.
    Wheel {
        /// The horizontal CSS-pixel coordinate.
        x: f32,
        /// The vertical CSS-pixel coordinate.
        y: f32,
        /// The horizontal scroll delta in CSS pixels.
        dx: f32,
        /// The vertical scroll delta in CSS pixels.
        dy: f32,
    },
    /// Dispatches a keyboard event on the focused element.
    Key {
        /// The event phase.
        phase: SynthKeyPhase,
        /// The DOM key name.
        key: String,
        /// The DOM physical key code.
        code: String,
        /// Whether Shift is held.
        shift: bool,
        /// Whether Control is held.
        ctrl: bool,
        /// Whether Alt is held.
        alt: bool,
        /// Whether Meta is held.
        meta: bool,
    },
    /// Inserts plain text into the focused editable element.
    InsertText {
        /// The text to insert.
        text: String,
    },
    /// Focuses the DOM element for a view.
    Focus {
        /// The view's node id.
        node: u32,
    },
    /// Navigates back in the page's history.
    Back,
}
