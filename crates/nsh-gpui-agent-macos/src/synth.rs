//! Synthesized input whose provenance the plug can mark.
//!
//! NSH's public synth door constructs the events, but cannot stamp them or
//! call the plug's provenance scope. Mouse events therefore use upstream's
//! construction here with buwiz's synthesized-event number; key dispatch is
//! the public door inside the plug's key scope.

use std::rc::Rc;

use objc2::{MainThreadMarker, rc::Retained};
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSWindow};

use nsh_platform_macos::surface::Surface;

/// Posts a mouse event of `kind` to `window` at `(x, y)` (content points).
#[cfg_attr(not(feature = "gpui-agent"), allow(dead_code))]
pub(crate) fn mouse(window: &NSWindow, kind: NSEventType, x: f64, y: f64, clicks: isize) {
    if let Some(event) = mouse_event(window, kind, x, y, clicks, NSEventModifierFlags::empty()) {
        // Through the event queue, as real clicks come: the event becomes
        // `NSApp.currentEvent` while it is handled (responders look at it).
        // The run loop handles it before the next step's timer.
        NSApplication::sharedApplication(MainThreadMarker::from(window))
            .postEvent_atStart(&event, false);
    }
}

/// A mouse event of `window` at `(x, y)` (content points), stamped as
/// synthesized input exactly as buwiz stamped it.
pub(crate) fn mouse_event(
    window: &NSWindow,
    kind: NSEventType,
    x: f64,
    y: f64,
    clicks: isize,
    mods: NSEventModifierFlags,
) -> Option<Retained<NSEvent>> {
    NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind,
        nsh_platform_macos::synth::to_window(window, x, y),
        mods,
        nsh_platform_macos::synth::timestamp(),
        window.windowNumber(),
        None,
        crate::provenance::SYNTH_EVENT,
        clicks,
        if clicks > 0 { 1.0 } else { 0.0 },
    )
}

/// Presses the key `name` names as buwiz's provenance-aware synth did.
pub(crate) fn key(surface: &Rc<Surface>, window: &NSWindow, name: &str) {
    crate::provenance::synth_key(|| nsh_platform_macos::synth::key(surface, window, name));
}

/// Types `text` as keys in `window`, inside the same provenance scope a
/// single synthesized key uses.
#[cfg_attr(not(feature = "gpui-agent"), allow(dead_code))]
pub(crate) fn type_text(surface: &Rc<Surface>, window: &NSWindow, text: &str) {
    crate::provenance::synth_key(|| nsh_platform_macos::synth::type_text(surface, window, text));
}
