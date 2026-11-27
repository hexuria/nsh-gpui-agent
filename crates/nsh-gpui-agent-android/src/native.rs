//! The strict native driver: the host's driver for upstream operations,
//! with the plug's Java bridge for what the agent adds.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use jni::{
    JNIEnv,
    objects::{GlobalRef, JClass, JFloatArray, JIntArray, JObject, JValue},
};
use nsh_agent_actions::StrictNative;
use nsh_core::{
    NodeId,
    geometry::{Bounds, Point, Points, Size},
};
use nsh_testing::agent::{Key, Native, SystemName};

use crate::android::SharedBridge;

/// The strict Android driver of one surface.
pub(crate) struct AndroidStrictNative {
    inner: Box<dyn Native>,
    bridge: SharedBridge,
    view: GlobalRef,
    density: f32,
    alive: Arc<AtomicBool>,
}

impl AndroidStrictNative {
    pub(crate) fn new(
        inner: Box<dyn Native>,
        bridge: SharedBridge,
        view: GlobalRef,
        density: f32,
        alive: Arc<AtomicBool>,
    ) -> AndroidStrictNative {
        AndroidStrictNative {
            inner,
            bridge,
            view,
            density,
            alive,
        }
    }

    /// `f` with the JNI env, the bridge class and the `NshView`. Errors and
    /// a closed agent become `R::default`, as they did in the host's driver.
    fn call<R: Default>(
        &self,
        f: impl FnOnce(&mut JNIEnv<'_>, &JClass<'_>, &JObject<'_>) -> jni::errors::Result<R>,
    ) -> R {
        if !self.alive.load(Ordering::Acquire) {
            return R::default();
        }
        self.bridge.with_env(|env| {
            let class: &JClass<'_> = self.bridge.class().as_obj().into();
            f(env, class, self.view.as_obj())
        })
    }

    fn px(&self, p: f32) -> f32 {
        p * self.density
    }

    /// Calls a bridge method returning a Java float array with only the
    /// view as its argument.
    fn floats(&self, method: &str) -> Option<Vec<f32>> {
        self.call(|env, class, view| {
            let array = env
                .call_static_method(
                    class,
                    method,
                    "(Landroid/view/View;)[F",
                    &[JValue::Object(view)],
                )?
                .l()?;
            if array.is_null() {
                return Ok(None);
            }
            let array = JFloatArray::from(array);
            let len = env.get_array_length(&array)? as usize;
            let mut out = vec![0.0; len];
            env.get_float_array_region(&array, 0, &mut out)?;
            Ok(Some(out))
        })
    }
}

impl Native for AndroidStrictNative {
    fn target_window(&self, window: NodeId) -> Result<(), String> {
        self.inner.target_window(window)
    }

    fn window_state(
        &self,
        window: NodeId,
    ) -> Result<Option<nsh_core::desktop::WindowState>, String> {
        self.inner.window_state(window)
    }

    fn press_window_close(&self, window: NodeId) -> Result<(), String> {
        self.inner.press_window_close(window)
    }

    fn window_screenshot(&self, window: NodeId) -> Result<Vec<u8>, String> {
        self.inner.window_screenshot(window)
    }

    fn bounds(&self, id: NodeId) -> Option<Bounds<Points>> {
        self.inner.bounds(id)
    }

    fn text_bounds(&self, text: NodeId, start: u32, end: u32) -> Option<Bounds<Points>> {
        self.inner.text_bounds(text, start, end)
    }

    fn viewport(&self) -> Size<Points> {
        // The agent resizes the NshView only (`resize`), which the
        // development host shows from the window's top left: its viewport
        // is the view's size, in the same window coordinates.
        let v = self.floats("contentSize").unwrap_or_else(|| vec![0.0, 0.0]);
        Size::new(Points(v[0] / self.density), Points(v[1] / self.density))
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
        if !self.available() {
            return Err("the app is gone".into());
        }
        let delivered: bool = self.call(|env, class, view| {
            env.call_static_method(
                class,
                "hover",
                "(Landroid/view/View;FF)Z",
                &[
                    JValue::Object(view),
                    JValue::Float(self.px(at.x.0)),
                    JValue::Float(self.px(at.y.0)),
                ],
            )?
            .z()
        });
        delivered
            .then_some(())
            .ok_or_else(|| "Android hover dispatch failed".into())
    }

    fn right_click(&self, at: Point<Points>) -> Result<(), String> {
        self.inner.right_click(at)
    }

    fn focused(&self) -> Option<NodeId> {
        let focused: Option<i32> = self.call(|env, class, view| {
            env.call_static_method(
                class,
                "focusedNode",
                "(Landroid/view/View;)I",
                &[JValue::Object(view)],
            )?
            .i()
            .map(Some)
        });
        focused.filter(|id| *id >= 0).map(|id| id as NodeId)
    }

    fn paste(&self, content: &nsh_core::PasteEvent) -> Result<(), String> {
        if !content.html.is_empty() || !content.files.is_empty() || !content.uris.is_empty() {
            return Err("Android paste supports plain text only".into());
        }
        let done = self.call(|env, class, view| {
            let text = env.new_string(&content.text)?;
            env.call_static_method(
                class,
                "paste",
                "(Landroid/view/View;Ljava/lang/String;)Z",
                &[JValue::Object(view), JValue::Object(&text)],
            )?
            .z()
        });
        done.then_some(())
            .ok_or_else(|| "Android paste dispatch failed".into())
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
        self.inner.drop_files(at, uris)
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
        self.inner.back()
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
        if !width.0.is_finite() || !height.0.is_finite() || width.0 <= 0.0 || height.0 <= 0.0 {
            return Err("viewport dimensions must be finite and positive".into());
        }
        let (width_px, height_px) = (
            self.px(width.0).round() as i32,
            self.px(height.0).round() as i32,
        );
        let resized = self.call(|env, class, view| {
            env.call_static_method(
                class,
                "resizeViewport",
                "(Landroid/view/View;II)Z",
                &[
                    JValue::Object(view),
                    JValue::Int(width_px),
                    JValue::Int(height_px),
                ],
            )?
            .z()
        });
        if !resized {
            return Err("requested viewport does not fit the Android window".into());
        }
        for _ in 0..50 {
            let actual = self.viewport();
            if (actual.width.0 * self.density - width_px as f32).abs() < 1.0
                && (actual.height.0 * self.density - height_px as f32).abs() < 1.0
            {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err("Android did not lay out the requested viewport".into())
    }

    fn set_dark(&self, dark: bool) -> Result<(), String> {
        self.inner.set_dark(dark)
    }

    fn quit(&self) {
        self.inner.quit();
    }
}

impl StrictNative for AndroidStrictNative {
    fn available(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    fn hit_at(&self, at: Point<Points>) -> Result<Option<NodeId>, String> {
        let hit: Option<i32> = self.call(|env, class, view| {
            env.call_static_method(
                class,
                "hitAt",
                "(Landroid/view/View;FF)I",
                &[
                    JValue::Object(view),
                    JValue::Float(self.px(at.x.0)),
                    JValue::Float(self.px(at.y.0)),
                ],
            )?
            .i()
            .map(Some)
        });
        let hit = hit.ok_or_else(|| "Android hit testing failed".to_string())?;
        // `AgentBridge.HIT_*`.
        match hit {
            -1 => Ok(None),
            // A view that is not one of the app's (a system dialog's)
            // occludes the point; it has no node id.
            -2 => Ok(Some(NodeId::MAX)),
            i32::MIN => Err(
                "Android hit testing failed: the main thread did not answer \
                             within 10 s, or the hit walk threw"
                    .to_owned(),
            ),
            id if id < 0 => Err(format!("Android hit testing failed: unknown result {id}")),
            id => Ok(Some(id as NodeId)),
        }
    }

    fn validate_keys(&self, keys: &str) -> Result<(), String> {
        use nsh_core::keymap::Keystroke;
        let stroke =
            Keystroke::parse(keys, nsh_core::platform::Os::Android).map_err(|e| e.to_string())?;
        stroke
            .key
            .android_keycode()
            .map(|_| ())
            .ok_or_else(|| format!("an Android keyboard has no key {keys:?}"))
    }

    fn focus_node(&self, id: NodeId) -> Result<(), String> {
        let focused = self.call(|env, class, view| {
            env.call_static_method(
                class,
                "focusNode",
                "(Landroid/view/View;I)Z",
                &[JValue::Object(view), JValue::Int(id as i32)],
            )?
            .z()
        });
        focused
            .then_some(())
            .ok_or_else(|| "native focus is only supported for an editable input".to_string())
    }

    fn app_focused(&self) -> bool {
        self.call(|env, class, view| {
            env.call_static_method(
                class,
                "appFocused",
                "(Landroid/view/View;)Z",
                &[JValue::Object(view)],
            )?
            .z()
        })
    }

    fn rotate(&self, landscape: bool) -> Result<(), String> {
        let done = self.call(|env, class, view| {
            env.call_static_method(
                class,
                "rotate",
                "(Landroid/view/View;Z)Z",
                &[
                    JValue::Object(view),
                    JValue::Bool(if landscape { 1 } else { 0 }),
                ],
            )?
            .z()
        });
        done.then_some(())
            .ok_or_else(|| "Android could not change the requested orientation".into())
    }

    fn input_selections(&self, ids: &[NodeId]) -> std::collections::HashMap<NodeId, (u32, u32)> {
        let ids: Vec<i32> = ids
            .iter()
            .filter_map(|&id| i32::try_from(id).ok())
            .collect();
        // One main-thread hop for every input of the snapshot: the bridge
        // answers (id, start, end) triples for those with a selection.
        let triples: Vec<i32> = self.call(|env, class, view| {
            let wanted = env.new_int_array(ids.len() as i32)?;
            env.set_int_array_region(&wanted, 0, &ids)?;
            let array = env
                .call_static_method(
                    class,
                    "inputSelections",
                    "(Landroid/view/View;[I)[I",
                    &[JValue::Object(view), JValue::Object(&wanted)],
                )?
                .l()?;
            if array.is_null() {
                return Ok(Vec::new());
            }
            let array = JIntArray::from(array);
            let len = env.get_array_length(&array)? as usize;
            let mut out = vec![0; len];
            env.get_int_array_region(&array, 0, &mut out)?;
            Ok(out)
        });
        triples
            .chunks_exact(3)
            .filter_map(|triple| match *triple {
                [id, start, end] if id >= 0 && start >= 0 && end >= start => {
                    Some((id as NodeId, (start as u32, end as u32)))
                }
                _ => None,
            })
            .collect()
    }

    fn keyboard_frame(&self) -> Result<Option<Bounds<Points>>, String> {
        let frame = self
            .floats("keyboardFrame")
            .ok_or_else(|| "Android cannot tell where the keyboard is".to_string())?;
        match frame.as_slice() {
            [] => Ok(None),
            [x, y, w, h] => Ok(Some(Bounds::new(
                Point::new(Points(x / self.density), Points(y / self.density)),
                Size::new(Points(w / self.density), Points(h / self.density)),
            ))),
            other => Err(format!(
                "Android keyboard frame has {} numbers, not 4",
                other.len()
            )),
        }
    }

    fn keyboard(&self, shown: bool) -> Result<(), String> {
        let done = self.call(|env, class, view| {
            env.call_static_method(
                class,
                "keyboard",
                "(Landroid/view/View;Z)Z",
                &[
                    JValue::Object(view),
                    JValue::Bool(if shown { 1 } else { 0 }),
                ],
            )?
            .z()
        });
        if !done {
            return if shown {
                Err("no input has the focus".into())
            } else {
                Err("Android could not hide the keyboard".into())
            };
        }
        let expected = if shown { 1 } else { 0 };
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let state = self.call(|env, class, view| {
                env.call_static_method(
                    class,
                    "imeState",
                    "(Landroid/view/View;)I",
                    &[JValue::Object(view)],
                )?
                .i()
            });
            if state == -1 || state == expected {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(if shown {
                    "the keyboard did not show within 2 s".into()
                } else {
                    "the keyboard did not hide within 2 s".into()
                });
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
