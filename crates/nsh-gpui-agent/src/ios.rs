//! iOS Simulator attachment for a development host.
//!
//! Call [`install`] before the host opens any development surfaces. The host
//! must link this crate with feature `ios` and the same NSH as the app.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use nsh_agent_actions::{Agent, NATIVE_KEYBINDING_UNSUPPORTED, SemanticKeybinding, StrictNative};
use nsh_core::{
    NodeId,
    geometry::{Bounds, Point, Points, Size},
};
use nsh_platform_ios::dev::DevelopmentSurface;
use nsh_testing::agent::{Key, Model, Native, OpenedSurface, SurfaceAttachment, SystemName};
use objc2_ui_kit::{UIApplication, UIApplicationState, UIView};

/// Registers the iOS surface hook before the development host starts.
///
/// An error means another tool already installed a surface hook.
pub fn install() -> Result<(), String> {
    nsh_testing::agent::set_surface_hook(opened)
        .map_err(|_| "a development surface hook is already installed".into())
}

fn opened(opened: &OpenedSurface<'_>) -> Option<SurfaceAttachment> {
    let surface = opened.surface.downcast_ref::<DevelopmentSurface>()?;
    match Attachment::start(surface) {
        Ok(attachment) => attachment,
        Err(error) => {
            log::error!("cannot start GPUI agent: {error}");
            surface.on_connected(move |client, show_error| {
                client.log(
                    log::Level::Error.into(),
                    "nsh_gpui_agent",
                    &format!("cannot start GPUI agent; continuing development without it: {error}"),
                );
                show_error("GPUI agent unavailable", &error);
            });
            None
        }
    }
}

struct Attachment {
    alive: Arc<AtomicBool>,
    _worker: crate::WorkerHandle,
}

impl Attachment {
    fn start(surface: &DevelopmentSurface) -> Result<Option<SurfaceAttachment>, String> {
        if !matches!(std::env::var("GPUI_AGENT").as_deref(), Ok("1" | "true")) {
            return Ok(None);
        }
        if !cfg!(target_abi = "sim") {
            return Err("GPUI agent attach requires the iOS Simulator".into());
        }
        if !cfg!(debug_assertions) {
            return Err("GPUI agent attach is unavailable in release builds".into());
        }
        let home = std::env::var_os("HOME").ok_or("app data container is unavailable")?;
        let token_path = std::path::PathBuf::from(home).join("Library/Caches/gpui-agent-token");
        let token = std::fs::read_to_string(&token_path);
        let removed = std::fs::remove_file(&token_path);
        let token = token.map_err(|_| "cannot read the GPUI-agent token file")?;
        removed.map_err(|_| "cannot remove the GPUI-agent token file")?;
        let address = std::env::var("GPUI_AGENT_ADDR").unwrap_or_else(|_| "127.0.0.1:17421".into());
        let address = address
            .parse()
            .map_err(|_| "GPUI-agent address is invalid")?;
        crate::validate_worker_config(address, &token)?;
        let model = Arc::new(Model::default());
        let alive = Arc::new(AtomicBool::new(true));
        let native = IosNative {
            native: surface.native(),
            surface: surface.clone(),
            alive: alive.clone(),
        };
        let agent = Arc::new(Agent::requiring_native_hit_test(
            model.clone(),
            Box::new(native),
        ));
        let screenshots = std::path::PathBuf::from(
            std::env::var_os("HOME").ok_or("app data container is unavailable")?,
        )
        .join("Library/Caches/gpui-agent");
        std::fs::create_dir_all(&screenshots)
            .map_err(|_| "cannot create GPUI-agent screenshot directory")?;
        let host = crate::NshAgentHost::new("Native Shell", crate::PlatformKind::Mobile, agent)?
            .with_screenshot_directory(screenshots);
        let worker = crate::spawn_worker(address, token, host)?;
        Ok(Some(SurfaceAttachment::new(
            Some(model),
            Box::new(Self {
                alive,
                _worker: worker,
            }),
        )))
    }
}

impl Drop for Attachment {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

struct IosNative {
    native: Box<dyn Native>,
    surface: DevelopmentSurface,
    alive: Arc<AtomicBool>,
}

impl StrictNative for IosNative {
    fn available(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    fn app_focused(&self) -> bool {
        self.surface
            .with_views(|_, window, _| {
                UIApplication::sharedApplication(objc2::MainThreadMarker::from(window))
                    .applicationState()
                    == UIApplicationState::Active
                    && window.isKeyWindow()
            })
            .unwrap_or(false)
    }

    fn hit_at(&self, at: Point<Points>) -> Result<Option<NodeId>, String> {
        self.surface
            .with_views(move |_, window, mounted| {
                let point =
                    objc2_core_foundation::CGPoint::new(f64::from(at.x.0), f64::from(at.y.0));
                let Some(hit) = window.hitTest_withEvent(point, None) else {
                    return Ok(None);
                };
                if !mounted.is_available() {
                    return Err("native surface is unavailable".to_owned());
                }
                let mut current = Some(hit);
                while let Some(native) = current {
                    let tag = native.tag();
                    if tag >= 0
                        && let Some(owner) = mounted.get(tag as NodeId)
                        && std::ptr::eq::<UIView>(&*owner, &*native)
                    {
                        return Ok(Some(tag as NodeId));
                    }
                    current = native.superview();
                }
                Ok(None)
            })
            .unwrap_or_else(|| Err("the window is gone".to_owned()))
    }

    fn fire_keybinding(&self, binding: &SemanticKeybinding) -> Result<(), String> {
        let Some(id) = binding.keymap_id() else {
            return Err(NATIVE_KEYBINDING_UNSUPPORTED.into());
        };
        self.surface
            .with_views(move |_, _, mounted| mounted.key_command(id))
            .unwrap_or(false)
            .then_some(())
            .ok_or_else(|| "the window is gone".to_owned())
    }
}

impl Native for IosNative {
    fn bounds(&self, id: NodeId) -> Option<Bounds<Points>> {
        self.native.bounds(id)
    }
    fn text_bounds(&self, text: NodeId, start: u32, end: u32) -> Option<Bounds<Points>> {
        self.native.text_bounds(text, start, end)
    }
    fn scroll_room(&self, id: NodeId, horizontal: bool) -> Option<(Points, Points)> {
        self.native.scroll_room(id, horizontal)
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
    fn paste(&self, content: &nsh_core::PasteEvent) -> Result<(), String> {
        self.native.paste(content)
    }
    fn type_text(&self, text: &str) {
        self.native.type_text(text);
    }
    fn key(&self, key: Key) {
        self.native.key(key);
    }
    fn press_keys(&self, keys: &str) -> Result<(), String> {
        self.native.press_keys(keys)
    }
    fn input_text(&self, id: NodeId) -> Option<String> {
        self.native.input_text(id)
    }
    fn back(&self) -> Result<(), String> {
        self.native.back()
    }
    fn system_control(&self, name: SystemName<'_>) -> Option<Bounds<Points>> {
        self.native.system_control(name)
    }
    fn system_menu_open(&self) -> bool {
        self.native.system_menu_open()
    }
    fn choose_menu_item(&self, path: &[String]) -> Result<(), String> {
        self.native.choose_menu_item(path)
    }
    fn close_menu(&self) -> Result<(), String> {
        self.native.close_menu()
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
        self.native.resize(width, height)
    }
    fn set_dark(&self, dark: bool) -> Result<(), String> {
        self.native.set_dark(dark)
    }
    fn quit(&self) {
        self.native.quit();
    }
}
