//! Strict native actions attached to macOS development surfaces.

use std::{
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use dispatch2::DispatchQueue;
use nsh_agent_actions::StrictNative;
use nsh_core::{
    NodeId, ROOT_ID,
    desktop::WindowState,
    geometry::{Bounds, Point, Points, Size},
};
use nsh_platform_macos::{
    agent::MacNative,
    file_drop, input,
    surface::Surface,
    synth, text_input,
    view::{NshView, superview},
    window,
};
use nsh_testing::agent::{Key, Native, SystemName};
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSEventType, NSView, NSWindow};

/// The strict driver: upstream's ordinary macOS driver for input it did
/// not change, and buwiz's strict methods copied here against the doors.
#[doc(hidden)]
pub struct MacStrictNative {
    inner: MacNative,
    key: u64,
    target: Mutex<NodeId>,
    alive: Arc<AtomicBool>,
}

impl MacStrictNative {
    pub fn new(key: u64, alive: Arc<AtomicBool>) -> Self {
        Self {
            inner: MacNative::new(key),
            key,
            target: Mutex::new(ROOT_ID),
            alive,
        }
    }

    fn target(&self) -> NodeId {
        *self.target.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn on_main<R: Send>(&self, f: impl FnOnce(&Rc<Surface>, &NSWindow) -> R + Send) -> Option<R> {
        self.on_window(Which::Main, f)
    }

    fn on_target<R: Send>(&self, f: impl FnOnce(&Rc<Surface>, &NSWindow) -> R + Send) -> Option<R> {
        self.on_window(Which::Target(self.target()), f)
    }

    fn on_key<R: Send>(&self, f: impl FnOnce(&Rc<Surface>, &NSWindow) -> R + Send) -> Option<R> {
        self.on_window(Which::Key, f)
    }

    fn on_window<R: Send>(
        &self,
        which: Which,
        f: impl FnOnce(&Rc<Surface>, &NSWindow) -> R + Send,
    ) -> Option<R> {
        let key = self.key;
        let mut out = None;
        let slot = &mut out;
        DispatchQueue::main().exec_sync(move || {
            let Some(surface) = Surface::with_key(key) else {
                return;
            };
            let window = match which {
                Which::Main => surface.root().ns().window(),
                Which::Target(id) => window::window_of(&surface, id),
                Which::Key => window::key_window(&surface),
            };
            let Some(window) = window else {
                return;
            };
            *slot = Some(f(&surface, &window));
        });
        out
    }
}

#[derive(Clone, Copy)]
enum Which {
    Main,
    Target(NodeId),
    Key,
}

fn walk(view: &NshView, f: &mut impl FnMut(&NshView)) {
    f(view);
    for child in view.child_views() {
        walk(&child, f);
    }
}

fn find_view(root: &NshView, id: NodeId) -> Option<NshView> {
    if root.id() == id {
        return Some(root.clone());
    }
    for child in root.child_views() {
        if let Some(view) = find_view(&child, id) {
            return Some(view);
        }
    }
    None
}

fn target_view(surface: &Rc<Surface>, id: NodeId) -> Option<NshView> {
    if let Some(view) = find_view(&surface.root(), id) {
        return Some(view);
    }
    for root in window::roots(surface) {
        if let Some(view) = find_view(&root, id) {
            return Some(view);
        }
    }
    None
}

fn scan_pads(root: &NshView, hit: &NSView, pad: &mut Option<NodeId>) {
    walk(root, &mut |view| {
        if pad.is_none()
            && view
                .input()
                .is_some_and(|input| text_input::pad_owns(&input, hit))
        {
            *pad = Some(view.id());
        }
    });
}

fn pad_owner(surface: &Rc<Surface>, hit: &NSView) -> Option<NodeId> {
    let mut pad = None;
    scan_pads(&surface.root(), hit, &mut pad);
    if pad.is_none() {
        for root in window::roots(surface) {
            scan_pads(&root, hit, &mut pad);
        }
    }
    pad
}

impl StrictNative for MacStrictNative {
    fn available(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    fn hit_at(&self, at: Point<Points>) -> Result<Option<NodeId>, String> {
        self.on_target(move |surface, window| {
            let content = window.contentView()?;
            let window_point = synth::to_window(window, f64::from(at.x.0), f64::from(at.y.0));
            let point = match superview(&content) {
                Some(parent) => parent.convertPoint_fromView(window_point, None),
                None => window_point,
            };
            let hit = content.hitTest(point)?;
            // A surface root takes every press inside it and picks the
            // view itself, so ask it as the press would.
            let hit = match NshView::from_ns(&hit).and_then(|v| v.surface()) {
                Some(picker) => {
                    let root = nsh_platform_macos::window::root_in(&picker, window)?;
                    let local = root.ns().convertPoint_fromView(window_point, None);
                    let view = input::view_at(&root, local.x as f32, local.y as f32)?;
                    view.ns().retain()
                }
                None => hit,
            };
            // A lifted input's pad is hosted outside its view's subtree:
            // the press it takes goes to the input's editor, so the hit is
            // the input's own rather than an occlusion.
            if let Some(id) = pad_owner(surface, &hit) {
                return Some(id);
            }
            let mut current = Some(hit);
            while let Some(native) = current {
                if let Some(view) = NshView::from_ns(&native)
                    && std::ptr::eq::<NSView>(view.ns(), &*native)
                {
                    return Some(view.id());
                }
                current = superview(&native);
            }
            None
        })
        .ok_or_else(|| "the window is gone".to_owned())
    }

    fn focus_node(&self, id: NodeId) -> Result<(), String> {
        self.on_main(move |surface, _| {
            let view = target_view(surface, id);
            view.is_some_and(|view| {
                let Some(window) = view.ns().window() else {
                    return false;
                };
                if view.kind() == nsh_core::ViewKind::Input {
                    text_input::focus(&view);
                } else if id != ROOT_ID {
                    input::focus_control(surface, &view);
                }
                let focused = text_input::test_focused(&window) == Some(id)
                    || window.firstResponder().is_some_and(|responder| {
                        let mut current = responder
                            .downcast_ref::<NSView>()
                            .map(objc2::Message::retain);
                        while let Some(native) = current {
                            if let Some(owner) = NshView::from_ns(&native) {
                                return owner.id() == id;
                            }
                            current = superview(&native);
                        }
                        false
                    });
                if focused {
                    window.makeKeyAndOrderFront(None);
                    window.makeKeyWindow();
                }
                focused
            })
        })
        .ok_or_else(|| "the window is gone".to_owned())?
        .then_some(())
        .ok_or_else(|| format!("node {id} cannot receive native focus"))
    }

    fn app_focused(&self) -> bool {
        self.on_key(|_, window| {
            NSApplication::sharedApplication(MainThreadMarker::from(window)).isActive()
                && window.isKeyWindow()
        })
        .unwrap_or(false)
    }
}

impl Native for MacStrictNative {
    fn bounds(&self, id: NodeId) -> Option<Bounds<Points>> {
        self.inner.bounds(id)
    }

    fn scroll_room(&self, id: NodeId, horizontal: bool) -> Option<(Points, Points)> {
        self.inner.scroll_room(id, horizontal)
    }

    fn target_window(&self, window: NodeId) -> Result<(), String> {
        self.inner.target_window(window)?;
        *self.target.lock().unwrap_or_else(|e| e.into_inner()) = window;
        Ok(())
    }

    fn window_state(&self, window: NodeId) -> Result<Option<WindowState>, String> {
        self.inner.window_state(window)
    }

    fn press_window_close(&self, window: NodeId) -> Result<(), String> {
        self.inner.press_window_close(window)
    }

    fn window_screenshot(&self, window: NodeId) -> Result<Vec<u8>, String> {
        self.inner.window_screenshot(window)
    }

    fn text_bounds(&self, text: NodeId, start: u32, end: u32) -> Option<Bounds<Points>> {
        self.inner.text_bounds(text, start, end)
    }

    fn viewport(&self) -> Size<Points> {
        self.inner.viewport()
    }

    fn press(&self, at: Point<Points>) {
        self.inner.press(at);
    }

    fn long_press(&self, at: Point<Points>, hold: Duration) {
        self.inner.long_press(at, hold);
    }

    fn scroll(&self, at: Point<Points>, delta: Point<Points>) {
        self.inner.scroll(at, delta);
    }

    fn drag(&self, from: Point<Points>, to: Point<Points>) {
        self.inner.drag(from, to);
    }

    fn mouse(&self) -> bool {
        self.inner.mouse()
    }

    fn hover(&self, at: Point<Points>) -> Result<(), String> {
        let (x, y) = (f64::from(at.x.0), f64::from(at.y.0));
        // Straight to the surface's input: AppKit hands a posted mouse
        // move to the first responder only, not to the view under it.
        self.on_target(move |surface, window| {
            let event = crate::synth::mouse_event(
                window,
                NSEventType::MouseMoved,
                x,
                y,
                0,
                NSEventModifierFlags::empty(),
            );
            if let Some(event) = event {
                let target = self.target();
                let root = (target != ROOT_ID)
                    .then(|| window::root_view(surface, target))
                    .flatten();
                // The window stays the input's, as after a real move: what
                // the hover changes is picked again where the pointer is,
                // not in the main window.
                input::enter(surface, root);
                input::event(surface, &event);
            }
        })
        .ok_or_else(|| "the window is gone".to_owned())
    }

    fn right_click(&self, at: Point<Points>) -> Result<(), String> {
        self.inner.right_click(at)
    }

    fn focused(&self) -> Option<NodeId> {
        self.inner.focused()
    }

    fn paste(&self, content: &nsh_core::PasteEvent) -> Result<(), String> {
        self.inner.paste(content)
    }

    fn compose(&self, text: &str) -> Result<(), String> {
        self.inner.compose(text)
    }

    fn choose_menu_item(&self, path: &[String]) -> Result<(), String> {
        self.inner.choose_menu_item(path)
    }

    fn close_menu(&self) -> Result<(), String> {
        self.inner.close_menu()
    }

    fn system_menu_open(&self) -> bool {
        self.inner.system_menu_open()
    }

    fn system_control(&self, name: SystemName<'_>) -> Option<Bounds<Points>> {
        self.inner.system_control(name)
    }

    fn press_keys(&self, keys: &str) -> Result<(), String> {
        self.inner.press_keys(keys)
    }

    fn drag_and_hold(&self, from: Point<Points>, to: Point<Points>) -> Result<(), String> {
        self.inner.drag_and_hold(from, to)
    }

    fn drop_files(&self, at: Point<Points>, uris: Vec<String>) -> Result<(), String> {
        let (x, y) = (at.x.0, at.y.0);
        let target = self.target();
        let dropped = self.on_target(move |surface, _| {
            let root = (target != ROOT_ID)
                .then(|| window::root_view(surface, target))
                .flatten();
            // The agent's drop does not move the pointer (`input.last`):
            // the window it was in stays the input's, so what the drop
            // changes is picked again where the pointer is (a hover in
            // this window included), not in the main window.
            let event_root = input::event_root(surface);
            let pointer = (event_root != surface.root()).then_some(event_root);
            input::enter(surface, root);
            let dropped = file_drop::drop_uris(surface, x, y, uris);
            input::enter(surface, pointer);
            dropped
        });
        match dropped {
            Some(true) => Ok(()),
            Some(false) => Err("no view there listens to `onFileDrop`".into()),
            None => Err("the window is gone".into()),
        }
    }

    fn drop_at(&self, at: Point<Points>) -> Result<(), String> {
        self.inner.drop_at(at)
    }

    fn hold_keys(&self, keys: &str, down: bool) -> Result<(), String> {
        self.inner.hold_keys(keys, down)
    }

    fn type_text(&self, text: &str) {
        self.inner.type_text(text);
    }

    fn key(&self, key: Key) {
        self.inner.key(key);
    }

    fn input_text(&self, id: NodeId) -> Option<String> {
        self.inner.input_text(id)
    }

    fn back(&self) -> Result<(), String> {
        // Escape, as a user goes back: in a focused field it only leaves
        // it, so then once more. The field is the one in the window the
        // keys go to; its window is where the pointer last was.
        self.on_key(|surface, window| {
            if text_input::editing_in(&input::root_in(surface, window)) {
                crate::synth::key(surface, window, "Escape");
            }
            crate::synth::key(surface, window, "Escape");
        })
        .ok_or_else(|| "the window is gone".to_owned())
    }

    fn forward(&self) -> Result<(), String> {
        self.inner.forward()
    }

    fn answer_dialog(&self, button: Option<&str>) -> Result<(), String> {
        self.inner.answer_dialog(button)
    }

    fn animating(&self) -> bool {
        self.inner.animating()
    }

    fn screenshot(&self) -> Result<Vec<u8>, String> {
        self.inner.screenshot()
    }

    fn resize(&self, width: Points, height: Points) -> Result<(), String> {
        self.inner.resize(width, height)
    }

    fn set_dark(&self, dark: bool) -> Result<(), String> {
        self.inner.set_dark(dark)
    }

    fn quit(&self) {
        self.inner.quit();
    }
}
