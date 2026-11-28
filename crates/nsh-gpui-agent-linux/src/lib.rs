//! Strict native actions attached to Linux development surfaces.

#![cfg(target_os = "linux")]

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use nsh_agent_actions::{Agent, NATIVE_KEYBINDING_UNSUPPORTED, SemanticKeybinding, StrictNative};
use nsh_core::{
    NodeId,
    desktop::{MenuItem, WindowState},
    geometry::{Bounds, Point, Points, Size},
};
use nsh_gpui_agent::{NshAgentHost, PlatformKind, WorkerHandle};
use nsh_platform_linux::{
    dev::{DevSurface, DevSurfaceView},
    gtk4::{self as gtk, gdk, prelude::*},
};
use nsh_testing::agent::{Key, Model, Native, OpenedSurface, SurfaceAttachment};

struct Attachment {
    alive: Arc<AtomicBool>,
    _worker: Option<WorkerHandle>,
}

impl Drop for Attachment {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

/// Installs the development surface hook before opening any Linux surfaces.
pub fn install() {
    let _ = nsh_testing::agent::set_surface_hook(opened);
}

fn opened(opened: &OpenedSurface<'_>) -> Option<SurfaceAttachment> {
    let surface = opened.surface.downcast_ref::<DevSurface>()?.clone();
    if let Ok(None) = nsh_gpui_agent::worker_config_from_env() {
        // The agent is off: attach nothing, and no mount batch is mirrored.
        return None;
    }
    let model = Arc::new(Model::default());
    let alive = Arc::new(AtomicBool::new(true));
    let native = LinuxNative {
        native: surface.native(),
        surface,
        window: Mutex::new(nsh_core::ROOT_ID),
        alive: alive.clone(),
    };
    let agent = Arc::new(Agent::requiring_native_hit_test(
        model.clone(),
        Box::new(native),
    ));
    let worker = match NshAgentHost::new("Native Shell", PlatformKind::Desktop, agent) {
        Ok(host) => match nsh_gpui_agent::start_from_env(host) {
            Ok(worker) => worker,
            Err(_) => {
                log::error!("invalid GPUI_AGENT development configuration");
                None
            }
        },
        Err(_) => {
            log::error!("cannot construct the GPUI-agent development host");
            None
        }
    };
    Some(SurfaceAttachment::new(
        Some(model),
        Box::new(Attachment {
            alive,
            _worker: worker,
        }),
    ))
}

struct LinuxNative {
    native: Box<dyn Native>,
    surface: DevSurface,
    /// The window input targets (`target_window`): hit tests and resizes
    /// go to it too ([`nsh_core::ROOT_ID`]: the main window).
    window: Mutex<NodeId>,
    alive: Arc<AtomicBool>,
}

impl StrictNative for LinuxNative {
    fn focus_known(&self) -> bool {
        true
    }

    fn available(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    fn hit_at(&self, at: Point<Points>) -> Result<Option<NodeId>, String> {
        let window = *self.window.lock().unwrap_or_else(|e| e.into_inner());
        self.surface
            .on_main(move |surface| {
                if window == nsh_core::ROOT_ID {
                    surface.node_at(at)
                } else {
                    surface.window_node_at(window, at)
                }
            })
            .ok_or_else(|| "the window is gone".to_owned())
    }

    fn validate_keys(&self, keys: &str) -> Result<(), String> {
        use nsh_core::keymap::Keystroke;
        let stroke =
            Keystroke::parse(keys, nsh_core::platform::Os::Linux).map_err(|e| e.to_string())?;
        let has_binding = self
            .surface
            .on_main(move |surface| surface.supports(stroke))
            .ok_or_else(|| "the window is gone".to_owned())?;
        if has_binding || synthesized(stroke).is_some() {
            Ok(())
        } else {
            Err(format!("unsupported Linux key chord `{keys}`"))
        }
    }

    fn focus_node(&self, id: NodeId) -> Result<(), String> {
        self.surface
            .on_main(move |surface| {
                let view = surface.widget(id);
                view.is_some_and(|widget| {
                    let focused = widget.root().and_then(|root| root.focus());
                    focused
                        .as_ref()
                        .is_some_and(|focused| focused == &widget || focused.is_ancestor(&widget))
                        || focus_native(&widget)
                })
            })
            .ok_or_else(|| "the window is gone".to_owned())?
            .then_some(())
            .ok_or_else(|| format!("node {id} cannot receive native focus"))
    }

    fn app_focused(&self) -> bool {
        self.surface
            .on_main(|surface| {
                surface.window().is_some_and(|window| window.is_active())
                    || surface
                        .app_windows()
                        .iter()
                        .any(|id| surface.window_focused(*id) == Some(true))
            })
            .unwrap_or(false)
    }

    fn fire_keybinding(&self, binding: &SemanticKeybinding) -> Result<(), String> {
        let Some(id) = binding.keymap_id() else {
            return Err(NATIVE_KEYBINDING_UNSUPPORTED.into());
        };
        self.surface
            .on_main(move |surface| surface.key_command(id))
            .ok_or_else(|| "the window is gone".to_owned())
    }

    fn fire_menu_keybinding(&self, binding: &SemanticKeybinding) -> Result<(), String> {
        let binding = binding.clone();
        self.surface
            .on_main(move |surface| fire_menu_keybinding(surface, &binding))
            .unwrap_or_else(|| Err("the window is gone".into()))
    }
}

impl Native for LinuxNative {
    fn bounds(&self, id: NodeId) -> Option<Bounds<Points>> {
        self.native.bounds(id)
    }
    fn scroll_room(&self, id: NodeId, horizontal: bool) -> Option<(Points, Points)> {
        self.native.scroll_room(id, horizontal)
    }
    fn target_window(&self, window: NodeId) -> Result<(), String> {
        let result = self.native.target_window(window);
        if result.is_ok() {
            *self.window.lock().unwrap_or_else(|e| e.into_inner()) = window;
        }
        result
    }
    fn window_state(&self, window: NodeId) -> Result<Option<WindowState>, String> {
        self.native.window_state(window)
    }
    fn press_window_close(&self, window: NodeId) -> Result<(), String> {
        self.native.press_window_close(window)
    }
    fn window_screenshot(&self, window: NodeId) -> Result<Vec<u8>, String> {
        self.native.window_screenshot(window)
    }
    fn text_bounds(&self, text: NodeId, start: u32, end: u32) -> Option<Bounds<Points>> {
        self.native.text_bounds(text, start, end)
    }
    fn viewport(&self) -> Size<Points> {
        self.native.viewport()
    }
    fn press(&self, at: Point<Points>) {
        self.native.press(at);
    }
    fn long_press(&self, at: Point<Points>, hold: Duration) {
        self.native.long_press(at, hold);
    }
    fn scroll(&self, at: Point<Points>, delta: Point<Points>) {
        self.native.scroll(at, delta);
    }
    fn drag(&self, from: Point<Points>, to: Point<Points>) {
        self.native.drag(from, to);
    }
    fn mouse(&self) -> bool {
        self.native.mouse()
    }
    fn hover(&self, at: Point<Points>) -> Result<(), String> {
        self.native.hover(at)
    }
    fn right_click(&self, at: Point<Points>) -> Result<(), String> {
        self.native.right_click(at)
    }
    fn focused(&self) -> Option<NodeId> {
        self.native.focused()
    }
    fn paste(&self, content: &nsh_core::PasteEvent) -> Result<(), String> {
        self.native.paste(content)
    }
    fn compose(&self, text: &str) -> Result<(), String> {
        self.native.compose(text)
    }
    fn choose_menu_item(&self, path: &[String]) -> Result<(), String> {
        self.native.choose_menu_item(path)
    }
    fn close_menu(&self) -> Result<(), String> {
        self.native.close_menu()
    }
    fn system_menu_open(&self) -> bool {
        self.native.system_menu_open()
    }
    fn system_control(&self, name: nsh_testing::agent::SystemName<'_>) -> Option<Bounds<Points>> {
        self.native.system_control(name)
    }
    fn press_keys(&self, keys: &str) -> Result<(), String> {
        self.native.press_keys(keys)
    }
    fn hold_keys(&self, keys: &str, down: bool) -> Result<(), String> {
        self.native.hold_keys(keys, down)
    }
    fn type_text(&self, text: &str) {
        self.native.type_text(text);
    }
    fn key(&self, key: Key) {
        self.native.key(key);
    }
    fn input_text(&self, id: NodeId) -> Option<String> {
        self.native.input_text(id)
    }
    fn back(&self) -> Result<(), String> {
        self.native.back()
    }
    fn forward(&self) -> Result<(), String> {
        self.native.forward()
    }
    fn answer_dialog(&self, button: Option<&str>) -> Result<(), String> {
        self.native.answer_dialog(button)
    }
    fn animating(&self) -> bool {
        self.native.animating()
    }
    fn screenshot(&self) -> Result<Vec<u8>, String> {
        self.native.screenshot()
    }
    fn resize(&self, width: Points, height: Points) -> Result<(), String> {
        let target = *self.window.lock().unwrap_or_else(|e| e.into_inner());
        if target != nsh_core::ROOT_ID {
            return self
                .surface
                .on_main(move |surface| surface.resize_window(target, width, height))
                .unwrap_or(false)
                .then_some(())
                .ok_or_else(|| format!("window {target} is not mounted"));
        }
        self.surface
            .on_main(move |surface| {
                let window = surface
                    .window()
                    .ok_or_else(|| "the window is gone".to_owned())?;
                if window.is_maximized() || window.is_fullscreen() {
                    return Err(
                        "cannot resize the viewport of a maximized or fullscreen window".into(),
                    );
                }
                let (window_width, window_height) = window.default_size();
                let root = surface.root();
                let (content_width, content_height) = (root.width(), root.height());
                if window_width <= 0
                    || window_height <= 0
                    || content_width <= 0
                    || content_height <= 0
                {
                    return Err("the viewport is not laid out yet".into());
                }
                // GTK's default size, not its widget allocation, is the resize
                // coordinate space. Preserve its measured difference from content.
                window.set_default_size(
                    viewport_window_dimension(width, window_width, content_width),
                    viewport_window_dimension(height, window_height, content_height),
                );
                Ok(())
            })
            .unwrap_or_else(|| Err("the window is gone".to_owned()))
    }
    fn set_dark(&self, dark: bool) -> Result<(), String> {
        self.native.set_dark(dark)
    }
    fn quit(&self) {
        self.native.quit();
    }
    fn drag_and_hold(&self, from: Point<Points>, to: Point<Points>) -> Result<(), String> {
        self.native.drag_and_hold(from, to)
    }
    fn drop_files(&self, at: Point<Points>, uris: Vec<String>) -> Result<(), String> {
        self.native.drop_files(at, uris)
    }
    fn drop_at(&self, at: Point<Points>) -> Result<(), String> {
        self.native.drop_at(at)
    }
}

fn viewport_window_dimension(requested: Points, window: i32, content: i32) -> i32 {
    (requested.0.round() as i32)
        .saturating_add(window.saturating_sub(content))
        .max(1)
}

#[allow(dead_code)]
enum Synthesized {
    Enter,
    Backspace,
    SelectAll,
    Undo { redo: bool },
    Char(char),
    Back(gdk::Key),
}

fn synthesized(stroke: nsh_core::keymap::Keystroke) -> Option<Synthesized> {
    use nsh_core::keymap::Key as K;
    let m = stroke.modifiers;
    let plain = !(m.ctrl || m.alt || m.cmd);
    Some(match stroke.key {
        K::Enter if plain => Synthesized::Enter,
        K::Backspace if plain => Synthesized::Backspace,
        K::Char('a') if m.ctrl && !m.alt && !m.cmd => Synthesized::SelectAll,
        K::Char('z') if m.ctrl && !m.alt && !m.cmd => Synthesized::Undo { redo: m.shift },
        K::Char(c) if plain => Synthesized::Char(if m.shift { shifted(c) } else { c }),
        K::Escape => Synthesized::Back(gdk::Key::Escape),
        K::Left => Synthesized::Back(gdk::Key::Left),
        K::Right => Synthesized::Back(gdk::Key::Right),
        K::Up => Synthesized::Back(gdk::Key::Up),
        K::Down => Synthesized::Back(gdk::Key::Down),
        K::Tab => Synthesized::Back(if m.shift {
            gdk::Key::ISO_Left_Tab
        } else {
            gdk::Key::Tab
        }),
        K::Home => Synthesized::Back(gdk::Key::Home),
        K::End => Synthesized::Back(gdk::Key::End),
        K::PageUp => Synthesized::Back(gdk::Key::Page_Up),
        K::PageDown => Synthesized::Back(gdk::Key::Page_Down),
        K::Delete => Synthesized::Back(gdk::Key::Delete),
        K::Insert => Synthesized::Back(gdk::Key::Insert),
        _ => return None,
    })
}

fn focus_native(widget: &gtk::Widget) -> bool {
    if let Some(text) = text_inside(widget)
        && text.grab_focus_without_selecting()
    {
        return true;
    }
    if widget.grab_focus() {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if c.is_focusable() && c.grab_focus() {
            return true;
        }
        if focus_native(&c) {
            return true;
        }
        child = c.next_sibling();
    }
    false
}

fn text_inside(widget: &gtk::Widget) -> Option<gtk::Text> {
    if let Some(text) = widget.downcast_ref::<gtk::Text>() {
        return Some(text.clone());
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(text) = text_inside(&c) {
            return Some(text);
        }
        child = c.next_sibling();
    }
    None
}

fn fire_menu_keybinding(
    surface: DevSurfaceView<'_>,
    binding: &SemanticKeybinding,
) -> Result<(), String> {
    let action_id = binding
        .menu_action_id
        .ok_or("menu shortcut has no action id")?;
    let path = binding
        .menu_path
        .as_deref()
        .ok_or("menu shortcut has no action path")?;
    let items = surface
        .menu()
        .ok_or("the application menu is unavailable")?;
    let mut level: &[MenuItem] = &items;
    let mut chosen = None;
    for (index, label) in path.iter().enumerate() {
        let last = index + 1 == path.len();
        let item = level
            .iter()
            .find(|item| match item {
                MenuItem::Action(action) => {
                    last && action.enabled && action.id == action_id && action.label == *label
                }
                MenuItem::Submenu {
                    label: submenu,
                    enabled: true,
                    ..
                } => !last && submenu == label,
                MenuItem::Separator | MenuItem::Submenu { .. } => false,
            })
            .ok_or_else(|| format!("the application menu has no action {label:?}"))?;
        match item {
            MenuItem::Action(action) => chosen = Some((action.id, action.role)),
            MenuItem::Submenu { items, .. } => level = items,
            MenuItem::Separator => {}
        }
    }
    let (id, role) = chosen.ok_or("no menu action chosen")?;
    surface.choose(id, role);
    Ok(())
}

fn shifted(c: char) -> char {
    const PAIRS: [(char, char); 21] = [
        ('1', '!'),
        ('2', '@'),
        ('3', '#'),
        ('4', '$'),
        ('5', '%'),
        ('6', '^'),
        ('7', '&'),
        ('8', '*'),
        ('9', '('),
        ('0', ')'),
        ('-', '_'),
        ('=', '+'),
        ('[', '{'),
        (']', '}'),
        ('\\', '|'),
        (';', ':'),
        ('\'', '"'),
        (',', '<'),
        ('.', '>'),
        ('/', '?'),
        ('`', '~'),
    ];
    PAIRS
        .iter()
        .find(|(plain, _)| *plain == c)
        .map_or(c.to_ascii_uppercase(), |(_, up)| *up)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_attachment_marks_surface_unavailable() {
        let alive = Arc::new(AtomicBool::new(true));
        let attachment = Attachment {
            alive: alive.clone(),
            _worker: None,
        };
        assert!(alive.load(Ordering::Acquire));
        drop(attachment);
        assert!(!alive.load(Ordering::Acquire));
    }

    #[test]
    fn synthesized_keys_match_linux_fallback() {
        use nsh_core::{keymap::Keystroke, platform::Os};
        assert!(matches!(
            synthesized(Keystroke::parse("Enter", Os::Linux).unwrap()),
            Some(Synthesized::Enter)
        ));
        assert!(matches!(
            synthesized(Keystroke::parse("ctrl+shift+z", Os::Linux).unwrap()),
            Some(Synthesized::Undo { redo: true })
        ));
        assert!(matches!(
            synthesized(Keystroke::parse("Tab", Os::Linux).unwrap()),
            Some(Synthesized::Back(gdk::Key::Tab))
        ));
        assert!(matches!(
            synthesized(Keystroke::parse("Right", Os::Linux).unwrap()),
            Some(Synthesized::Back(gdk::Key::Right))
        ));
        assert!(matches!(
            synthesized(Keystroke::parse("shift+Tab", Os::Linux).unwrap()),
            Some(Synthesized::Back(gdk::Key::ISO_Left_Tab))
        ));
        assert!(synthesized(Keystroke::parse("F12", Os::Linux).unwrap()).is_none());
        assert!(Keystroke::parse("", Os::Linux).is_err());
    }

    #[test]
    fn viewport_dimensions_compensate_chrome_without_accumulating_it() {
        assert_eq!(viewport_window_dimension(Points(1000.0), 800, 790), 1010);
        assert_eq!(viewport_window_dimension(Points(1000.0), 1010, 1000), 1010);
        assert_eq!(viewport_window_dimension(Points(390.0), 1010, 1000), 400);
        assert_eq!(viewport_window_dimension(Points(844.0), 900, 850), 894);
        assert_eq!(viewport_window_dimension(Points(390.0), 800, 800), 390);
    }
}
