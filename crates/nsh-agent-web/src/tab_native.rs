use std::{
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use nsh_core::{
    NodeId, PasteEvent,
    geometry::{Bounds, Point, Points, Size},
};
use nsh_dev::{DevServer, protocol::TestStep};
use nsh_testing::agent::{Key, Native, drag_path};
use serde_json::Value;

use crate::synth::{SynthEvent, SynthKeyPhase, SynthPointerPhase};

use crate::web_native::{ASK, WebScene, browser_keys};

const FRAME: Duration = Duration::from_millis(16);
const CLICK: Duration = Duration::from_millis(60);
const SCROLL_STEP: f32 = 20.0;
const UNSUPPORTED: &str = "is not available in a tab nsh does not control";

pub(crate) struct TabNative {
    server: Arc<DevServer>,
    scene: WebScene,
    failure: Mutex<Option<String>>,
}

impl TabNative {
    pub(crate) fn new(server: Arc<DevServer>) -> Self {
        Self {
            scene: WebScene::new(server.clone(), true),
            server,
            failure: Mutex::new(None),
        }
    }

    fn send(&self, event: SynthEvent) -> Result<(), String> {
        self.step(event).map(drop)
    }

    fn step(&self, event: SynthEvent) -> Result<Value, String> {
        self.server
            .step(
                TestStep::Page {
                    name: "synth".into(),
                    args: serde_json::to_value(event).map_err(|e| e.to_string())?,
                },
                ASK,
            )
            .map_err(|error| {
                if !self.server.has_tester() {
                    "no tab has opened the agent's page yet".into()
                } else {
                    error
                }
            })
    }

    fn record(&self, result: Result<(), String>) {
        if let Err(error) = result {
            *self.failure.lock().unwrap_or_else(|e| e.into_inner()) = Some(error);
        }
    }

    fn pointer(
        &self,
        phase: SynthPointerPhase,
        at: Point<Points>,
        buttons: u32,
    ) -> Result<(), String> {
        self.send(SynthEvent::Pointer {
            phase,
            x: at.x.0,
            y: at.y.0,
            buttons,
        })
    }

    fn click(&self, at: Point<Points>, hold: Duration) -> Result<(), String> {
        self.pointer(SynthPointerPhase::Move, at, 0)?;
        self.pointer(SynthPointerPhase::Down, at, 1)?;
        thread::sleep(hold);
        self.pointer(SynthPointerPhase::Up, at, 0)
    }

    fn key_event(
        &self,
        phase: SynthKeyPhase,
        key: String,
        code: String,
        modifiers: nsh_core::keymap::Modifiers,
    ) -> Result<Value, String> {
        self.step(SynthEvent::Key {
            phase,
            key,
            code,
            shift: modifiers.shift,
            ctrl: modifiers.ctrl,
            alt: modifiers.alt,
            meta: modifiers.cmd,
        })
    }
}

impl Native for TabNative {
    fn bounds(&self, id: NodeId) -> Option<Bounds<Points>> {
        self.scene.bounds(id)
    }

    fn text_bounds(&self, id: NodeId, start: u32, end: u32) -> Option<Bounds<Points>> {
        self.scene.text_bounds(id, start, end)
    }

    fn viewport(&self) -> Size<Points> {
        self.scene.viewport()
    }

    fn press(&self, at: Point<Points>) {
        self.record(self.click(at, CLICK));
    }

    fn long_press(&self, at: Point<Points>, hold: Duration) {
        self.record(self.click(at, hold));
    }

    fn scroll(&self, at: Point<Points>, delta: Point<Points>) {
        self.record((|| {
            self.pointer(SynthPointerPhase::Move, at, 0)?;
            let n = (delta.x.0.abs().max(delta.y.0.abs()) / SCROLL_STEP)
                .ceil()
                .max(1.0) as u32;
            for _ in 0..n {
                self.send(SynthEvent::Wheel {
                    x: at.x.0,
                    y: at.y.0,
                    dx: delta.x.0 / n as f32,
                    dy: delta.y.0 / n as f32,
                })?;
                thread::sleep(FRAME);
            }
            Ok(())
        })());
    }

    fn drag(&self, from: Point<Points>, to: Point<Points>) {
        let mut last = from;
        let result = (|| {
            self.pointer(SynthPointerPhase::Move, from, 0)?;
            self.pointer(SynthPointerPhase::Down, from, 1)?;
            thread::sleep(nsh_testing::agent::LANDING);
            for at in drag_path(from, to) {
                thread::sleep(FRAME);
                self.pointer(SynthPointerPhase::Move, at, 1)?;
                last = at;
            }
            self.pointer(SynthPointerPhase::Up, last, 0)
        })();
        if result.is_err() {
            let _ = self.pointer(SynthPointerPhase::Up, last, 0);
        }
        self.record(result);
    }

    fn mouse(&self) -> bool {
        true
    }

    fn hover(&self, at: Point<Points>) -> Result<(), String> {
        self.pointer(SynthPointerPhase::Move, at, 0)
    }

    fn focused(&self) -> Option<NodeId> {
        self.scene.focused()
    }

    fn paste(&self, content: &PasteEvent) -> Result<(), String> {
        if !content.html.is_empty() || !content.files.is_empty() || !content.uris.is_empty() {
            return Err("synthetic paste supports plain text only".into());
        }
        self.send(SynthEvent::InsertText {
            text: content.text.clone(),
        })
    }

    fn press_keys(&self, keys: &str) -> Result<(), String> {
        let (stroke, (key, code)) = browser_keys(keys)?;
        key_pair(|phase| self.key_event(phase, key.clone(), code.clone(), stroke.modifiers))
    }

    fn type_text(&self, text: &str) {
        self.record(self.send(SynthEvent::InsertText {
            text: text.to_owned(),
        }));
    }

    fn key(&self, key: Key) {
        let command = if cfg!(target_os = "macos") {
            "mod"
        } else {
            "ctrl"
        };
        let result = match key {
            Key::Return => self.press_keys("enter"),
            Key::Backspace => self.press_keys("backspace"),
            Key::SelectAll => self.press_keys(&format!("{command}+a")),
        };
        self.record(result);
    }

    fn input_text(&self, id: NodeId) -> Option<String> {
        self.scene.input_text(id)
    }

    fn back(&self) -> Result<(), String> {
        self.send(SynthEvent::Back)
    }

    fn forward(&self) -> Result<(), String> {
        Err(format!("forward {UNSUPPORTED}"))
    }

    fn right_click(&self, _: Point<Points>) -> Result<(), String> {
        Err(format!("right_click {UNSUPPORTED}"))
    }

    fn answer_dialog(&self, _: Option<&str>) -> Result<(), String> {
        Err(format!("answer_dialog {UNSUPPORTED}"))
    }

    fn animating(&self) -> bool {
        self.scene.animating()
    }

    fn screenshot(&self) -> Result<Vec<u8>, String> {
        Err(format!("screenshot {UNSUPPORTED}"))
    }

    fn resize(&self, _: Points, _: Points) -> Result<(), String> {
        Err(format!("resize {UNSUPPORTED}"))
    }

    fn set_dark(&self, _: bool) -> Result<(), String> {
        Err(format!("set_dark {UNSUPPORTED}"))
    }

    fn quit(&self) {}
}

impl nsh_agent_actions::StrictNative for TabNative {
    fn available(&self) -> bool {
        self.server.has_tester()
    }

    /// The page reports its focus ([`Native::focused`], the scene's): `None` there means nothing has it, so a press
    /// that leaves nothing focused is refused rather than typed into nowhere.
    fn focus_known(&self) -> bool {
        true
    }

    fn hit_at(&self, at: Point<Points>) -> Result<Option<NodeId>, String> {
        self.scene.hit_at(at)
    }

    fn validate_keys(&self, keys: &str) -> Result<(), String> {
        browser_keys(keys).map(drop)
    }

    fn focus_node(&self, id: NodeId) -> Result<(), String> {
        self.send(SynthEvent::Focus { node: id })
    }

    fn app_focused(&self) -> bool {
        self.scene.app_focused()
    }

    fn take_failure(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }
}

fn key_pair(
    mut dispatch: impl FnMut(SynthKeyPhase) -> Result<Value, String>,
) -> Result<(), String> {
    let down = dispatch(SynthKeyPhase::Down)?;
    let up = dispatch(SynthKeyPhase::Up);
    if let Some(error) = down.get("error").and_then(Value::as_str) {
        return Err(error.to_owned());
    }
    up.map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_tab_reports_its_focus() {
        use nsh_agent_actions::StrictNative as _;
        let server = std::sync::Arc::new(
            DevServer::start("127.0.0.1:0".parse().unwrap(), "test", None, |_| {}).unwrap(),
        );
        assert!(TabNative::new(server).focus_known());
    }

    #[test]
    fn dispatched_keys_release_even_when_the_default_is_unsupported() {
        for release in [Ok(Value::Null), Err("release failed".to_owned())] {
            let mut phases = Vec::new();
            let result = key_pair(|phase| {
                phases.push(phase);
                if phase == SynthKeyPhase::Down {
                    Ok(json!({"dispatched": true, "error": "unsupported default"}))
                } else {
                    release.clone()
                }
            });
            assert_eq!(result, Err("unsupported default".into()));
            assert_eq!(phases, [SynthKeyPhase::Down, SynthKeyPhase::Up]);
        }
    }

    #[test]
    fn unsupported_email_home_releases_before_returning_the_page_error() {
        let error = "key \"Home\" cannot be synthesized; use type for text";
        let response: Value = serde_json::from_str(
            r#"{"dispatched":true,"error":"key \"Home\" cannot be synthesized; use type for text"}"#,
        ).unwrap();
        let mut phases = Vec::new();
        let result = key_pair(|phase| {
            phases.push(phase);
            Ok(if phase == SynthKeyPhase::Down {
                response.clone()
            } else {
                Value::Null
            })
        });
        assert_eq!(phases, [SynthKeyPhase::Down, SynthKeyPhase::Up]);
        assert_eq!(result, Err(error.into()));
    }

    #[test]
    fn dispatch_failures_do_not_release_a_key_that_was_not_pressed() {
        let mut phases = Vec::new();
        assert_eq!(
            key_pair(|phase| {
                phases.push(phase);
                Err("dispatch failed".into())
            }),
            Err("dispatch failed".into())
        );
        assert_eq!(phases, [SynthKeyPhase::Down]);
    }

    #[test]
    fn supported_keys_report_the_release_result() {
        for release in [Ok(Value::Null), Err("release failed".to_owned())] {
            let mut phases = Vec::new();
            assert_eq!(
                key_pair(|phase| {
                    phases.push(phase);
                    if phase == SynthKeyPhase::Down {
                        Ok(Value::Null)
                    } else {
                        release.clone()
                    }
                }),
                release.map(drop)
            );
            assert_eq!(phases, [SynthKeyPhase::Down, SynthKeyPhase::Up]);
        }
    }
}
