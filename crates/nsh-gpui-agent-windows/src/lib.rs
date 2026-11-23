//! Strict native actions attached to Windows development surfaces.

#![cfg(windows)]

use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use nsh_agent_actions::{Agent, NATIVE_KEYBINDING_UNSUPPORTED, SemanticKeybinding, StrictNative};
use nsh_core::{
    NodeId,
    desktop::{MenuAction, MenuItem, MenuRole, WindowState},
    geometry::{Bounds, Point, Points, Size},
    keymap::Modifiers,
};
use nsh_gpui_agent::{NshAgentHost, PlatformKind, WorkerHandle};
use nsh_platform_windows::{DevSurface, DevSurfaceRef};
use nsh_testing::agent::{Key, Model, Native, OpenedSurface, SurfaceAttachment};

const MAIN: Duration = Duration::from_secs(10);

thread_local! {
    static AGENT_OWNER: RefCell<Weak<Attachment>> = const { RefCell::new(Weak::new()) };
}

struct Attachment {
    model: Arc<Model>,
    available: Arc<AtomicBool>,
    worker: RefCell<Option<WorkerHandle>>,
}

impl Attachment {
    fn agent_running(&self) -> bool {
        self.worker
            .borrow()
            .as_ref()
            .is_some_and(WorkerHandle::is_running)
    }

    fn release_agent(&self) {
        if self.worker.borrow_mut().take().is_none() {
            return;
        }
        self.available.store(false, Ordering::Release);
        self.model.reset();
        let _ = AGENT_OWNER.try_with(|owner| {
            let owns = owner
                .borrow()
                .upgrade()
                .is_some_and(|owner| std::ptr::eq(owner.as_ref(), self));
            if owns {
                *owner.borrow_mut() = Weak::new();
            }
        });
    }
}

struct Guard(Rc<Attachment>);

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.release_agent();
        self.0.available.store(false, Ordering::Release);
    }
}

/// Installs the development surface hook before opening any Windows surfaces.
pub fn install() {
    let _ = nsh_testing::agent::set_surface_hook(opened);
}

fn opened(opened: &OpenedSurface<'_>) -> Option<SurfaceAttachment> {
    let surface = opened.surface.downcast_ref::<DevSurface>()?.clone();
    if let Ok(None) = nsh_gpui_agent::worker_config_from_env() {
        // The agent is off: attach nothing, and no mount batch is mirrored.
        return None;
    }
    let attachment = Rc::new(Attachment {
        model: Arc::new(Model::default()),
        available: Arc::new(AtomicBool::new(true)),
        worker: RefCell::new(None),
    });
    start_agent(surface, &attachment);
    Some(SurfaceAttachment::new(
        Some(attachment.model.clone()),
        Box::new(Guard(attachment)),
    ))
}

fn start_agent(surface: DevSurface, attachment: &Rc<Attachment>) {
    let owner = AGENT_OWNER.with(|owner| owner.borrow().upgrade());
    if let Some(owner) = owner {
        if owner.agent_running() {
            return;
        }
        owner.release_agent();
    }
    let (addr, token) = match nsh_gpui_agent::worker_config_from_env() {
        Ok(Some(config)) => config,
        Ok(None) => return,
        Err(error) => {
            log::error!("invalid GPUI_AGENT development configuration: {error}");
            return;
        }
    };
    let native = WindowsNative {
        model: attachment.model.clone(),
        native: surface.native(),
        surface,
        window: Mutex::new(nsh_core::ROOT_ID),
        available: attachment.available.clone(),
        failure: Arc::default(),
    };
    let agent = Arc::new(Agent::requiring_native_hit_test(
        attachment.model.clone(),
        Box::new(native),
    ));
    let host = match NshAgentHost::new("Native Shell", PlatformKind::Desktop, agent) {
        Ok(host) => host,
        Err(_) => {
            log::error!("cannot construct the GPUI-agent development host");
            return;
        }
    };
    let worker = match nsh_gpui_agent::spawn_worker(addr, token, host) {
        Ok(worker) => worker,
        Err(error) => {
            log::error!("cannot start the GPUI-agent worker on {addr}: {error}");
            return;
        }
    };
    *attachment.worker.borrow_mut() = Some(worker);
    AGENT_OWNER.with(|owner| *owner.borrow_mut() = Rc::downgrade(attachment));
}

struct WindowsNative {
    model: Arc<Model>,
    native: Box<dyn Native>,
    surface: DevSurface,
    /// The window input targets (`target_window`): hit tests and resizes
    /// go to it too ([`nsh_core::ROOT_ID`]: the main window).
    window: Mutex<NodeId>,
    available: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<String>>>,
}

impl WindowsNative {
    fn on_main<R: Send + 'static>(
        &self,
        f: impl FnOnce(DevSurfaceRef<'_>) -> R + Send + 'static,
    ) -> Option<R> {
        let (tx, rx) = mpsc::channel();
        let deadline = Instant::now() + MAIN;
        let available = self.available.clone();
        self.surface.invoke(move |surface| {
            let _ = tx.send(dispatch(Some(&available), deadline, || surface, f));
        });
        match rx.recv_timeout(MAIN) {
            Ok(result) => result,
            Err(error) => {
                let failure = main_failure(error);
                log::warn!("{failure}");
                *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(failure);
                None
            }
        }
    }
}

impl StrictNative for WindowsNative {
    fn available(&self) -> bool {
        self.available.load(Ordering::Acquire)
    }

    fn take_failure(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    fn hit_at(&self, at: Point<Points>) -> Result<Option<NodeId>, String> {
        let (x, y) = (at.x.0, at.y.0);
        let window = *self.window.lock().unwrap_or_else(PoisonError::into_inner);
        self.on_main(move |surface| {
            if window == nsh_core::ROOT_ID {
                surface.pick_node(x, y)
            } else {
                surface.window_node_at(window, x, y)
            }
        })
        .ok_or_else(|| "the window is gone".to_owned())
    }

    fn validate_keys(&self, keys: &str) -> Result<(), String> {
        windows_key(keys).map(|_| ())
    }

    fn focus_node(&self, id: NodeId) -> Result<(), String> {
        require_node(&self.model, id)?;
        self.on_main(move |surface| {
            surface.focus_text_input(id);
            if surface.child_focused() {
                // SAFETY: focusing the surface's own window on its thread.
                let _ = unsafe {
                    windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(surface.hwnd()))
                };
            }
            (surface.focused_text_input() == Some(id) && !surface.child_focused())
                .then_some(())
                .ok_or_else(|| format!("node {id} cannot receive native focus"))
        })
        .unwrap_or_else(|| Err("the window is gone".to_owned()))
    }

    fn fire_menu_keybinding(&self, binding: &SemanticKeybinding) -> Result<(), String> {
        let binding = binding.clone();
        self.on_main(move |surface| {
            let action_id = binding
                .menu_action_id
                .ok_or("menu shortcut has no action id")?;
            let path = binding
                .menu_path
                .as_deref()
                .ok_or("menu shortcut has no action path")?;
            let items = surface
                .app_menu()
                .ok_or("the application menu is unavailable")?;
            let (id, role) = resolve_menu_keybinding(&items, action_id, path, |role| {
                surface.supports_menu_role(role)
            })?;
            surface.choose_menu_action(id, role);
            Ok(())
        })
        .unwrap_or_else(|| Err("the window is gone".into()))
    }

    fn app_focused(&self) -> bool {
        self.on_main(|surface| {
            // SAFETY: GetForegroundWindow is a read-only query.
            (unsafe {
                windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() == surface.hwnd()
            }) || surface
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
        self.on_main(move |surface| surface.key_command(id))
            .ok_or_else(|| "the window is gone".to_owned())
    }

    fn laid_out(&self) -> Result<(), String> {
        self.on_main(|surface| layout_state(surface.pixel_size(), surface.layout_pending()))
            .unwrap_or_else(|| Err("the window is gone".into()))
    }
}

impl Native for WindowsNative {
    fn bounds(&self, id: NodeId) -> Option<Bounds<Points>> {
        self.native.bounds(id)
    }
    fn scroll_room(&self, id: NodeId, horizontal: bool) -> Option<(Points, Points)> {
        self.native.scroll_room(id, horizontal)
    }
    fn target_window(&self, window: NodeId) -> Result<(), String> {
        let result = self.native.target_window(window);
        if result.is_ok() {
            *self.window.lock().unwrap_or_else(PoisonError::into_inner) = window;
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
        let middle = self.viewport();
        if middle.width.0 <= 0.0 || middle.height.0 <= 0.0 {
            return Err(self
                .take_failure()
                .unwrap_or_else(|| "the viewport is not laid out yet".into()));
        }
        self.native.back()
    }
    fn forward(&self) -> Result<(), String> {
        self.native.forward()
    }
    fn answer_dialog(&self, button: Option<&str>) -> Result<(), String> {
        self.native.answer_dialog(button)
    }
    fn animating(&self) -> bool {
        self.native.animating() || self.on_main(|s| s.layout_pending()).unwrap_or(false)
    }
    fn screenshot(&self) -> Result<Vec<u8>, String> {
        self.laid_out()?;
        self.native.screenshot()
    }
    fn resize(&self, width: Points, height: Points) -> Result<(), String> {
        let target = *self.window.lock().unwrap_or_else(PoisonError::into_inner);
        if target != nsh_core::ROOT_ID {
            let (w, h) = (width.0 as i32, height.0 as i32);
            return self
                .on_main(move |surface| surface.resize_window(target, w, h))
                .unwrap_or(false)
                .then_some(())
                .ok_or_else(|| format!("window {target} is not mounted"));
        }
        self.native.resize(width, height)
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

fn require_node(model: &Model, id: NodeId) -> Result<(), String> {
    model
        .screen(|screen| screen.mounted(id).is_some())
        .then_some(())
        .ok_or_else(|| format!("node {id} is missing"))
}

fn windows_key(keys: &str) -> Result<(u16, Modifiers), String> {
    let stroke = nsh_core::keymap::Keystroke::parse(keys, nsh_core::platform::Os::Windows)
        .map_err(|e| e.to_string())?;
    let vk = stroke
        .key
        .virtual_key()
        .ok_or_else(|| format!("{keys:?} has no key on a Windows keyboard"))?;
    Ok((vk, stroke.modifiers))
}

fn layout_state(pixels: (u32, u32), pending: bool) -> Result<(), String> {
    if pixels.0 == 0 || pixels.1 == 0 || pending {
        Err("the viewport is not laid out yet".into())
    } else {
        Ok(())
    }
}

fn dispatch<S, R>(
    available: Option<&AtomicBool>,
    deadline: Instant,
    surface: impl FnOnce() -> Option<S>,
    f: impl FnOnce(S) -> R,
) -> Option<R> {
    if available.is_some_and(|available| !available.load(Ordering::Acquire))
        || Instant::now() >= deadline
    {
        return None;
    }
    let Some(surface) = surface() else {
        if let Some(available) = available {
            available.store(false, Ordering::Release);
        }
        return None;
    };
    Some(f(surface))
}

fn main_failure(error: mpsc::RecvTimeoutError) -> String {
    match error {
        mpsc::RecvTimeoutError::Timeout => {
            format!("the main thread did not answer in {} s", MAIN.as_secs())
        }
        mpsc::RecvTimeoutError::Disconnected => {
            "the main thread dropped the call: its message loop has ended".into()
        }
    }
}

fn find_menu_action<'a, 'p>(
    items: &'a [MenuItem],
    path: &'p [String],
    enabled_only: bool,
    leaf: impl Fn(&MenuAction) -> bool,
) -> Result<Option<&'a MenuAction>, &'p str> {
    let mut level = items;
    for (n, label) in path.iter().enumerate() {
        let last = n + 1 == path.len();
        let item = level
            .iter()
            .find(|item| match item {
                MenuItem::Action(action) => last && action.label == *label && leaf(action),
                MenuItem::Submenu {
                    label: l, enabled, ..
                } => !last && l == label && (*enabled || !enabled_only),
                MenuItem::Separator => false,
            })
            .ok_or(label.as_str())?;
        match item {
            MenuItem::Action(action) => return Ok(Some(action)),
            MenuItem::Submenu { items, .. } => level = items,
            MenuItem::Separator => {}
        }
    }
    Ok(None)
}

fn resolve_menu_keybinding(
    items: &[MenuItem],
    action_id: u32,
    path: &[String],
    supported: impl FnOnce(Option<MenuRole>) -> bool,
) -> Result<(u32, Option<MenuRole>), String> {
    let action = find_menu_action(items, path, true, |action| {
        action.enabled && action.id == action_id
    })
    .map_err(|label| format!("the application menu has no action {label:?}"))?
    .ok_or("no menu action chosen")?;
    let (id, role) = (action.id, action.role);
    if !supported(role) {
        return Err("the menu action is unsupported on Windows".into());
    }
    Ok((id, role))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_requires_a_mounted_node() {
        let model = Model::default();
        assert_eq!(require_node(&model, 2), Err("node 2 is missing".into()));
        model.apply(&nsh_core::MountBatch {
            items: vec![nsh_core::MountItem::Create {
                id: 2,
                kind: nsh_core::ViewKind::View,
            }],
        });
        assert_eq!(require_node(&model, 2), Ok(()));
        model.reset();
        assert_eq!(require_node(&model, 2), Err("node 2 is missing".into()));
    }

    #[test]
    fn a_late_and_a_lost_call_fail_differently() {
        let late = main_failure(mpsc::RecvTimeoutError::Timeout);
        let lost = main_failure(mpsc::RecvTimeoutError::Disconnected);
        assert!(late.contains("did not answer"), "{late}");
        assert!(lost.contains("message loop has ended"), "{lost}");
    }

    #[test]
    fn without_an_attachment_a_late_call_is_skipped() {
        let mut called = false;
        let answer = dispatch(None, Instant::now(), || Some(()), |()| called = true);
        assert!(answer.is_none() && !called);
    }

    #[test]
    fn parses_windows_key_chords() {
        let (vk, modifiers) = windows_key("ctrl+a").expect("Windows key chord");
        assert_eq!(vk, 0x41);
        assert!(modifiers.ctrl);
        assert!(!modifiers.alt);
    }

    #[test]
    fn rejects_unsupported_windows_keys() {
        assert!(windows_key("not-a-windows-key").is_err());
    }

    #[test]
    fn a_window_without_its_layout_is_not_laid_out() {
        assert!(layout_state((0, 0), false).is_err());
        assert!(layout_state((1200, 0), false).is_err());
        assert!(layout_state((1200, 831), true).is_err());
        assert!(layout_state((1200, 831), false).is_ok());
    }

    #[test]
    fn a_late_dispatch_skips_the_call_and_keeps_the_attachment() {
        let available = AtomicBool::new(true);
        let mut called = false;
        let result = dispatch(
            Some(&available),
            Instant::now() - Duration::from_millis(1),
            || Some(()),
            |()| called = true,
        );
        assert!(result.is_none());
        assert!(!called);
        assert!(available.load(Ordering::Acquire));
    }

    #[test]
    fn a_dispatch_to_a_gone_surface_disables_the_attachment() {
        let available = AtomicBool::new(true);
        let mut called = false;
        let result = dispatch(
            Some(&available),
            Instant::now() + MAIN,
            || None::<()>,
            |()| called = true,
        );
        assert!(result.is_none());
        assert!(!called);
        assert!(!available.load(Ordering::Acquire));
    }

    #[test]
    fn a_live_dispatch_runs_the_call() {
        let available = AtomicBool::new(true);
        let result = dispatch(
            Some(&available),
            Instant::now() + MAIN,
            || Some(2),
            |n| n * 3,
        );
        assert_eq!(result, Some(6));
        assert!(available.load(Ordering::Acquire));
    }

    #[test]
    fn an_unavailable_attachment_never_looks_up_the_surface() {
        let available = AtomicBool::new(false);
        let result = dispatch(
            Some(&available),
            Instant::now() + MAIN,
            || panic!("looked up an unavailable surface"),
            |()| panic!("acted on an unavailable surface"),
        );
        assert!(result.is_none());
    }

    #[test]
    fn an_unrelated_surface_is_not_attached() {
        assert!(opened(&OpenedSurface::new(&())).is_none());
    }
}

#[cfg(test)]
mod agent_menu_tests {
    use nsh_core::keymap::When;

    use super::*;

    fn supported(role: Option<MenuRole>) -> bool {
        !matches!(
            role,
            Some(MenuRole::About | MenuRole::Hide | MenuRole::HideOthers | MenuRole::ShowAll)
        )
    }

    fn action(id: u32, label: &str, enabled: bool, role: Option<MenuRole>) -> MenuItem {
        MenuItem::Action(MenuAction {
            id,
            label: label.into(),
            shortcut: None,
            enabled,
            checked: None,
            role,
            when: When::Always,
            icon: None,
            symbol: None,
            destructive: false,
        })
    }

    fn menu(role: Option<MenuRole>, enabled: bool) -> Vec<MenuItem> {
        vec![MenuItem::Submenu {
            label: "File".into(),
            enabled: true,
            items: vec![action(7, "Open", enabled, role)],
            icon: None,
            symbol: None,
            inline: false,
        }]
    }

    fn path() -> Vec<String> {
        vec!["File".into(), "Open".into()]
    }

    #[test]
    fn resolves_enabled_menu_keybinding_by_path_and_id() {
        assert_eq!(
            resolve_menu_keybinding(&menu(None, true), 7, &path(), supported),
            Ok((7, None))
        );
        assert!(resolve_menu_keybinding(&menu(None, true), 8, &path(), supported).is_err());
    }

    #[test]
    fn rejects_missing_and_disabled_menu_keybindings() {
        assert!(
            resolve_menu_keybinding(&menu(None, true), 7, &["Missing".into()], supported).is_err()
        );
        assert!(resolve_menu_keybinding(&menu(None, false), 7, &path(), supported).is_err());
        let disabled_submenu = vec![MenuItem::Submenu {
            label: "File".into(),
            enabled: false,
            items: vec![action(7, "Open", true, None)],
            icon: None,
            symbol: None,
            inline: false,
        }];
        assert!(resolve_menu_keybinding(&disabled_submenu, 7, &path(), supported).is_err());
    }

    #[test]
    fn rejects_windows_unsupported_menu_roles() {
        assert!(
            resolve_menu_keybinding(&menu(Some(MenuRole::About), true), 7, &path(), supported)
                .is_err()
        );
    }

    #[test]
    fn rejects_an_empty_menu_or_path() {
        assert!(resolve_menu_keybinding(&[], 7, &path(), supported).is_err());
        assert_eq!(
            resolve_menu_keybinding(&menu(None, true), 7, &[], supported),
            Err("no menu action chosen".into())
        );
    }
}

#[cfg(test)]
mod attachment_tests {
    use nsh_testing::agent::Scene;

    use super::*;

    struct UncalledNative;

    impl StrictNative for UncalledNative {}

    impl Native for UncalledNative {
        fn bounds(&self, _: NodeId) -> Option<Bounds<Points>> {
            unreachable!()
        }
        fn viewport(&self) -> Size<Points> {
            unreachable!()
        }
        fn press(&self, _: Point<Points>) {
            unreachable!()
        }
        fn long_press(&self, _: Point<Points>, _: Duration) {
            unreachable!()
        }
        fn scroll(&self, _: Point<Points>, _: Point<Points>) {
            unreachable!()
        }
        fn drag(&self, _: Point<Points>, _: Point<Points>) {
            unreachable!()
        }
        fn type_text(&self, _: &str) {
            unreachable!()
        }
        fn key(&self, _: Key) {
            unreachable!()
        }
        fn input_text(&self, _: NodeId) -> Option<String> {
            unreachable!()
        }
        fn back(&self) -> Result<(), String> {
            unreachable!()
        }
        fn answer_dialog(&self, _: Option<&str>) -> Result<(), String> {
            unreachable!()
        }
        fn animating(&self) -> bool {
            unreachable!()
        }
        fn screenshot(&self) -> Result<Vec<u8>, String> {
            unreachable!()
        }
        fn resize(&self, _: Points, _: Points) -> Result<(), String> {
            unreachable!()
        }
        fn set_dark(&self, _: bool) -> Result<(), String> {
            unreachable!()
        }
        fn quit(&self) {
            unreachable!()
        }
    }

    fn attachment(worker: bool) -> Rc<Attachment> {
        let model = Arc::new(Model::default());
        let worker = worker.then(|| {
            let agent = Arc::new(Agent::requiring_native_hit_test(
                model.clone(),
                Box::new(UncalledNative),
            ));
            let host = NshAgentHost::new("test", PlatformKind::Desktop, agent).unwrap();
            nsh_gpui_agent::spawn_worker("127.0.0.1:0".parse().unwrap(), "test".into(), host)
                .unwrap()
        });
        Rc::new(Attachment {
            model,
            available: Arc::new(AtomicBool::new(true)),
            worker: RefCell::new(worker),
        })
    }

    #[test]
    fn closing_the_owner_releases_its_worker_and_resets_its_model() {
        let attachment = attachment(true);
        let guard = Guard(attachment.clone());
        AGENT_OWNER.with(|owner| *owner.borrow_mut() = Rc::downgrade(&attachment));
        attachment
            .model
            .apply(&nsh_core::MountBatch { items: vec![] });
        attachment.model.error("old engine");
        assert!(attachment.agent_running());
        assert_eq!(attachment.model.activity().unwrap().batches, 1);
        let addr = attachment.worker.borrow().as_ref().unwrap().local_addr();
        drop(guard);
        // The handle signals shutdown without joining the serve thread, so
        // poll until the listener is actually gone, not just the flag.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::net::TcpStream::connect(addr).is_ok() {
            assert!(
                std::time::Instant::now() < deadline,
                "worker listener on {addr} still accepts connections after shutdown"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!attachment.agent_running());
        assert!(!attachment.available.load(Ordering::Acquire));
        assert_eq!(attachment.model.activity().unwrap().batches, 0);
        assert!(attachment.model.take_errors().unwrap().is_empty());
        assert!(AGENT_OWNER.with(|owner| owner.borrow().upgrade().is_none()));
    }

    #[test]
    fn an_old_guard_does_not_release_the_new_owner() {
        let old = attachment(true);
        let old_guard = Guard(old.clone());
        AGENT_OWNER.with(|owner| *owner.borrow_mut() = Rc::downgrade(&old));
        old.release_agent();

        let current = attachment(true);
        let current_guard = Guard(current.clone());
        AGENT_OWNER.with(|owner| *owner.borrow_mut() = Rc::downgrade(&current));
        drop(old_guard);
        assert!(current.agent_running());
        assert!(current.available.load(Ordering::Acquire));
        assert!(AGENT_OWNER.with(|owner| Rc::ptr_eq(&owner.borrow().upgrade().unwrap(), &current)));
        drop(current_guard);
    }

    #[test]
    fn closing_an_attachment_without_a_worker_leaves_the_owner_alone() {
        let owner = attachment(true);
        let owner_guard = Guard(owner.clone());
        AGENT_OWNER.with(|slot| *slot.borrow_mut() = Rc::downgrade(&owner));
        let other = attachment(false);
        assert!(!other.agent_running());
        drop(Guard(other.clone()));
        assert!(!other.available.load(Ordering::Acquire));
        assert!(owner.agent_running());
        assert!(AGENT_OWNER.with(|slot| Rc::ptr_eq(&slot.borrow().upgrade().unwrap(), &owner)));
        drop(owner_guard);
    }
}
