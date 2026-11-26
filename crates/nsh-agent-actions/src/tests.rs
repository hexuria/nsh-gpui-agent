use std::sync::{Arc, Mutex};

use nsh_core::{
    EventKind, MountBatch,
    geometry::{Size, point},
    prelude::*,
};
use nsh_dev::protocol::{ClockChange, SpanStyle, TestStep, ViewStyle};
use nsh_testing::agent::{Activity, Model, RemoteScene};

use super::*;

fn pick_value(pick: Pick) -> serde_json::Value {
    match pick {
        Pick::One(id) => serde_json::json!({ "one": id }),
        Pick::None => serde_json::json!("none"),
        Pick::Many(n) => serde_json::json!({ "many": n }),
        Pick::Disabled => serde_json::json!("disabled"),
    }
}

/// A scene asked through a host ([`RemoteScene`]) whose semantic tree
/// is `nodes` and whose keybindings are `bindings`.
struct WithNodes(Vec<SemanticNode>, Vec<SemanticKeybinding>, RemoteScene);

impl SemanticScene for WithNodes {
    fn semantic_tree(&self) -> Result<Vec<SemanticNode>, String> {
        Ok(self.0.clone())
    }

    fn keybindings(&self) -> Result<Vec<SemanticKeybinding>, String> {
        Ok(self.1.clone())
    }
}

impl Scene for WithNodes {
    fn activity(&self) -> Result<Activity, String> {
        self.2.activity()
    }
    fn wait_for_batch(&self, seen: u64, timeout: Duration) {
        self.2.wait_for_batch(seen, timeout)
    }
    fn take_errors(&self) -> Result<Vec<String>, String> {
        self.2.take_errors()
    }
    fn input_value_text(&self, selector: &str) -> Result<Option<String>, String> {
        self.2.input_value_text(selector)
    }
    fn pick(&self, selector: &str, target: PickTarget) -> Result<Pick, String> {
        self.2.pick(selector, target)
    }
    fn scroll_ancestors(&self, id: NodeId) -> Result<Vec<(NodeId, bool)>, String> {
        self.2.scroll_ancestors(id)
    }
    fn drag_takers(&self, id: NodeId) -> Result<Vec<NodeId>, String> {
        self.2.drag_takers(id)
    }
    fn long_press_delay(&self, id: NodeId) -> Result<Option<f32>, String> {
        self.2.long_press_delay(id)
    }
    fn horizontal(&self, id: NodeId) -> Result<bool, String> {
        self.2.horizontal(id)
    }
    fn texts(&self) -> Result<Vec<String>, String> {
        self.2.texts()
    }
    fn count(&self, selector: &str) -> Result<usize, String> {
        self.2.count(selector)
    }
    fn matches(&self, selector: &str) -> Result<bool, String> {
        self.2.matches(selector)
    }
    fn enabled(&self, selector: &str) -> Result<bool, String> {
        self.2.enabled(selector)
    }
    fn span_style(&self, selector: &str) -> Result<SpanStyle, String> {
        self.2.span_style(selector)
    }
    fn view_style(&self, selector: &str) -> Result<Option<ViewStyle>, String> {
        self.2.view_style(selector)
    }
    fn input_id(&self, selector: &str) -> Result<Option<NodeId>, String> {
        self.2.input_id(selector)
    }
    fn dialogs(&self) -> Result<Vec<nsh_core::Dialog>, String> {
        self.2.dialogs()
    }
    fn pending_dialog(&self) -> Result<Option<nsh_core::Dialog>, String> {
        self.2.pending_dialog()
    }
    fn answered(&self) -> Result<(), String> {
        self.2.answered()
    }
    fn menu(&self) -> Result<Option<Vec<nsh_core::desktop::MenuItem>>, String> {
        self.2.menu()
    }
    fn menu_closed(&self) -> Result<(), String> {
        self.2.menu_closed()
    }
    fn clipboard(&self) -> Result<Option<String>, String> {
        self.2.clipboard()
    }
    fn storage(&self) -> Result<std::collections::BTreeMap<String, String>, String> {
        self.2.storage()
    }
    fn set_viewport(&self, size: Size<Points>) -> Result<(), String> {
        self.2.set_viewport(size)
    }
    fn span(&self, id: NodeId) -> Result<Option<(NodeId, u32, u32)>, String> {
        self.2.span(id)
    }
    fn canvas_node(&self, selector: &str) -> Result<Option<(NodeId, Point<Points>)>, String> {
        self.2.canvas_node(selector)
    }
    fn match_inside(&self, selector: &str, id: NodeId) -> Result<Option<NodeId>, String> {
        self.2.match_inside(selector, id)
    }
    fn move_clock(&self, change: ClockChange) -> Result<(), String> {
        self.2.move_clock(change)
    }
    fn manual_clock(&self) -> bool {
        self.2.manual_clock()
    }
    fn send_host_event(&self, name: &str, data: serde_json::Value) -> Result<(), String> {
        self.2.send_host_event(name, data)
    }
    fn opened_urls(&self) -> Result<Vec<String>, String> {
        self.2.opened_urls()
    }
    fn window_root(&self, id: &str) -> Result<Option<NodeId>, String> {
        self.2.window_root(id)
    }
    fn window_of(&self, id: NodeId) -> Result<NodeId, String> {
        self.2.window_of(id)
    }
}

type LongPressRecords = Arc<Mutex<Vec<(Point<Points>, Duration)>>>;
type TextBoundsTable = [((NodeId, u32, u32), Bounds<Points>)];

/// A recorded native action: its name, target window and point.
type WindowAction = (&'static str, NodeId, Option<Point<Points>>);

#[derive(Default)]
struct WindowRecords {
    selected: Mutex<NodeId>,
    targets: Mutex<Vec<NodeId>>,
    actions: Mutex<Vec<WindowAction>>,
    nodes: std::collections::HashMap<NodeId, NodeId>,
    unavailable: Option<NodeId>,
}

struct GateNative {
    windows: Option<Arc<WindowRecords>>,
    bounds: std::collections::HashMap<NodeId, Bounds<Points>>,
    text_bounds: std::collections::HashMap<(NodeId, u32, u32), Bounds<Points>>,
    hit: Result<Option<NodeId>, String>,
    hits: Arc<Mutex<Vec<Point<Points>>>>,
    presses: Arc<std::sync::atomic::AtomicUsize>,
    pressed_at: Arc<Mutex<Vec<Point<Points>>>>,
    long_presses: LongPressRecords,
    typed: Arc<std::sync::atomic::AtomicUsize>,
    viewport: Size<Points>,
}

impl Native for GateNative {
    fn target_window(&self, window: NodeId) -> Result<(), String> {
        if let Some(windows) = &self.windows {
            if windows.unavailable == Some(window) {
                return Err("native window is unavailable".into());
            }
            *windows.selected.lock().unwrap() = window;
            windows.targets.lock().unwrap().push(window);
        } else if window != nsh_core::ROOT_ID {
            return Err("native window is unavailable".into());
        }
        Ok(())
    }
    fn bounds(&self, id: NodeId) -> Option<Bounds<Points>> {
        self.bounds.get(&id).copied()
    }
    fn text_bounds(&self, id: NodeId, start: u32, end: u32) -> Option<Bounds<Points>> {
        self.text_bounds.get(&(id, start, end)).copied()
    }
    fn viewport(&self) -> Size<Points> {
        self.viewport
    }
    fn press(&self, at: Point<Points>) {
        self.record("press", Some(at));
        self.presses
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.pressed_at.lock().unwrap().push(at);
        if let Some(id) = GATE_FOCUS_ON_PRESS.with(std::cell::Cell::get) {
            GATE_FOCUSED.with(|focused| focused.set(Some(id)));
        }
    }
    fn long_press(&self, at: Point<Points>, hold: Duration) {
        self.record("long_press", Some(at));
        self.long_presses.lock().unwrap().push((at, hold));
        self.pressed_at.lock().unwrap().push(at);
    }
    fn scroll(&self, at: Point<Points>, _: Point<Points>) {
        self.record("scroll", Some(at));
    }
    fn drag(&self, at: Point<Points>, _: Point<Points>) {
        self.record("drag", Some(at));
    }
    fn hover(&self, at: Point<Points>) -> Result<(), String> {
        if self.windows.is_none() {
            return Err("this platform has no mouse".into());
        }
        self.record("hover", Some(at));
        Ok(())
    }
    fn press_keys(&self, _: &str) -> Result<(), String> {
        if self.windows.is_none() {
            return Err(
                "this platform's test agent presses no hardware keys: run the test headless".into(),
            );
        }
        self.record("key", None);
        Ok(())
    }
    fn focused(&self) -> Option<NodeId> {
        GATE_FOCUSED.with(std::cell::Cell::get)
    }
    fn type_text(&self, _: &str) {
        self.record("type", None);
        self.typed
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    fn key(&self, _: Key) {}
    fn input_text(&self, _: NodeId) -> Option<String> {
        None
    }
    fn back(&self) -> Result<(), String> {
        Err("unsupported".into())
    }
    fn answer_dialog(&self, _: Option<&str>) -> Result<(), String> {
        Ok(())
    }
    fn animating(&self) -> bool {
        false
    }
    fn screenshot(&self) -> Result<Vec<u8>, String> {
        Err("unsupported".into())
    }
    fn resize(&self, _: Points, _: Points) -> Result<(), String> {
        Ok(())
    }
    fn set_dark(&self, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn quit(&self) {}
}

thread_local! {
    /// The software keyboard's frame [`GateNative`] reports on this test's
    /// thread (the agent asks its native on the caller's thread).
    static GATE_KEYBOARD: std::cell::Cell<Option<Bounds<Points>>> = const { std::cell::Cell::new(None) };
    /// The focused input [`GateNative`] reports on this test's thread.
    static GATE_FOCUSED: std::cell::Cell<Option<NodeId>> = const { std::cell::Cell::new(None) };
    /// The input a press gives the focus to, on this test's thread.
    static GATE_FOCUS_ON_PRESS: std::cell::Cell<Option<NodeId>> = const { std::cell::Cell::new(None) };
}

impl StrictNative for GateNative {
    fn keyboard_frame(&self) -> Result<Option<Bounds<Points>>, String> {
        Ok(GATE_KEYBOARD.with(std::cell::Cell::get))
    }
    fn hit_at(&self, at: Point<Points>) -> Result<Option<NodeId>, String> {
        self.hits.lock().unwrap().push(at);
        if let Some(windows) = &self.windows {
            let selected = *windows.selected.lock().unwrap();
            return Ok(self
                .bounds
                .iter()
                .filter(|(id, bounds)| {
                    windows.nodes.get(id) == Some(&selected) && bounds.contains(&at)
                })
                .map(|(id, _)| *id)
                .max());
        }
        self.hit.clone()
    }
    fn focus_node(&self, _: NodeId) -> Result<(), String> {
        if self.windows.is_none() {
            return Err(NATIVE_FOCUS_UNSUPPORTED.into());
        }
        self.record("focus", None);
        Ok(())
    }
}

#[derive(Default)]
struct SystemActionRecords {
    calls: Mutex<Vec<&'static str>>,
    presses: Mutex<Vec<Point<Points>>>,
    pastes: Mutex<Vec<String>>,
    typed_text: Mutex<Vec<String>>,
    rotations: Mutex<Vec<bool>>,
    keyboards: Mutex<Vec<bool>>,
}

struct SystemActionNative {
    bounds: std::collections::HashMap<NodeId, Bounds<Points>>,
    viewport: Size<Points>,
    focused: Option<NodeId>,
    records: Arc<SystemActionRecords>,
    rotate_error: Option<&'static str>,
    keyboard_error: Option<&'static str>,
}

impl Native for SystemActionNative {
    fn bounds(&self, id: NodeId) -> Option<Bounds<Points>> {
        self.bounds.get(&id).copied()
    }
    fn viewport(&self) -> Size<Points> {
        self.viewport
    }
    fn press(&self, at: Point<Points>) {
        self.records.calls.lock().unwrap().push("press");
        self.records.presses.lock().unwrap().push(at);
    }
    fn long_press(&self, _: Point<Points>, _: Duration) {}
    fn scroll(&self, _: Point<Points>, _: Point<Points>) {}
    fn drag(&self, _: Point<Points>, _: Point<Points>) {}
    fn type_text(&self, text: &str) {
        self.records.calls.lock().unwrap().push("type");
        self.records
            .typed_text
            .lock()
            .unwrap()
            .push(text.to_string());
    }
    fn key(&self, _: Key) {}
    fn input_text(&self, _: NodeId) -> Option<String> {
        None
    }
    fn focused(&self) -> Option<NodeId> {
        self.focused
    }
    fn paste(&self, content: &nsh_core::PasteEvent) -> Result<(), String> {
        self.records.calls.lock().unwrap().push("paste");
        self.records
            .pastes
            .lock()
            .unwrap()
            .push(content.text.clone());
        Ok(())
    }
    fn back(&self) -> Result<(), String> {
        Ok(())
    }
    fn answer_dialog(&self, _: Option<&str>) -> Result<(), String> {
        Ok(())
    }
    fn animating(&self) -> bool {
        false
    }
    fn screenshot(&self) -> Result<Vec<u8>, String> {
        Err("unsupported".into())
    }
    fn resize(&self, _: Points, _: Points) -> Result<(), String> {
        Ok(())
    }
    fn set_dark(&self, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn quit(&self) {}
}

impl StrictNative for SystemActionNative {
    fn rotate(&self, landscape: bool) -> Result<(), String> {
        self.records.calls.lock().unwrap().push("rotate");
        self.records.rotations.lock().unwrap().push(landscape);
        match self.rotate_error {
            Some(error) => Err(error.into()),
            None => Ok(()),
        }
    }
    fn keyboard(&self, shown: bool) -> Result<(), String> {
        self.records.calls.lock().unwrap().push("keyboard");
        self.records.keyboards.lock().unwrap().push(shown);
        match self.keyboard_error {
            Some(error) => Err(error.into()),
            None => Ok(()),
        }
    }
}

impl GateNative {
    fn record(&self, action: &'static str, at: Option<Point<Points>>) {
        if let Some(windows) = &self.windows {
            let selected = *windows.selected.lock().unwrap();
            windows.actions.lock().unwrap().push((action, selected, at));
        }
    }
}

fn gate_scene(kind: nsh_core::ViewKind, disabled: bool, clip_root: bool) -> Arc<Model> {
    use nsh_core::{EventMask, MountItem, ViewProps, ViewStyle};

    let model = Arc::new(Model::default());
    let mut events = EventMask::default();
    events.set(EventKind::Press, true);
    let mut props = ViewProps {
        test_id: Some("target".into()),
        ..ViewProps::default()
    };
    props.accessibility.state.disabled = disabled;
    let mut items = vec![
        MountItem::Create { id: 2, kind },
        MountItem::Frame {
            id: 2,
            frame: Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0))),
        },
        MountItem::Insert {
            parent: 1,
            child: 2,
            before: None,
        },
        MountItem::Events { id: 2, events },
        MountItem::ViewProps {
            id: 2,
            props: Box::new(props),
        },
    ];
    if clip_root {
        items.push(MountItem::ViewStyle {
            id: 1,
            style: Box::new(ViewStyle {
                overflow_hidden: true,
                ..Default::default()
            }),
        });
    }
    model.apply(&MountBatch { items });
    model
}

fn system_action_agent(
    input: bool,
    focused: Option<NodeId>,
    rotate_error: Option<&'static str>,
    keyboard_error: Option<&'static str>,
) -> (Agent, Arc<SystemActionRecords>) {
    use nsh_core::{EventMask, InputConfig, MountItem, ViewKind, ViewProps};

    let model = Arc::new(Model::default());
    let mut events = EventMask::default();
    events.set(EventKind::Press, true);
    let props = ViewProps {
        test_id: Some("target".into()),
        ..ViewProps::default()
    };
    let frame = Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0)));
    let mut items = vec![
        MountItem::Create {
            id: 2,
            kind: if input {
                ViewKind::Input
            } else {
                ViewKind::View
            },
        },
        MountItem::Frame { id: 2, frame },
        MountItem::Insert {
            parent: 1,
            child: 2,
            before: None,
        },
        MountItem::Events { id: 2, events },
        MountItem::ViewProps {
            id: 2,
            props: Box::new(props),
        },
    ];
    if input {
        items.push(MountItem::Input {
            id: 2,
            config: Box::new(InputConfig::default()),
        });
    }
    model.apply(&MountBatch { items });

    let records = Arc::new(SystemActionRecords::default());
    let native = SystemActionNative {
        bounds: [(2, frame)].into(),
        viewport: size(px(100.0), px(100.0)),
        focused,
        records: records.clone(),
        rotate_error,
        keyboard_error,
    };
    (Agent::new(model, Box::new(native)), records)
}

#[test]
fn paste_node_preserves_focus_and_passes_text_to_native() {
    let (agent, records) = system_action_agent(true, Some(2), None, None);

    agent.paste_node(2, "replacement").unwrap();

    assert_eq!(*records.calls.lock().unwrap(), vec!["paste"]);
    assert!(records.presses.lock().unwrap().is_empty());
    assert_eq!(
        *records.pastes.lock().unwrap(),
        vec!["replacement".to_string()]
    );
}

#[test]
fn paste_node_focuses_an_unfocused_input_before_pasting() {
    let (agent, records) = system_action_agent(true, None, None, None);

    agent.paste_node(2, "replacement").unwrap();

    assert_eq!(*records.calls.lock().unwrap(), vec!["press", "paste"]);
    assert_eq!(
        *records.presses.lock().unwrap(),
        vec![point(px(50.0), px(50.0))]
    );
    assert_eq!(
        *records.pastes.lock().unwrap(),
        vec!["replacement".to_string()]
    );
}

#[test]
fn type_node_keeps_focus_on_a_focused_input() {
    let (agent, records) = system_action_agent(true, Some(2), None, None);

    agent.type_node(2, "typed text").unwrap();

    assert_eq!(*records.calls.lock().unwrap(), vec!["type"]);
    assert!(records.presses.lock().unwrap().is_empty());
    assert_eq!(
        *records.typed_text.lock().unwrap(),
        vec!["typed text".to_string()]
    );
}

#[test]
fn type_node_focuses_an_unfocused_input_before_typing() {
    let (agent, records) = system_action_agent(true, None, None, None);

    agent.type_node(2, "typed text").unwrap();

    assert_eq!(*records.calls.lock().unwrap(), vec!["press", "type"]);
    assert_eq!(
        *records.presses.lock().unwrap(),
        vec![point(px(50.0), px(50.0))]
    );
    assert_eq!(
        *records.typed_text.lock().unwrap(),
        vec!["typed text".to_string()]
    );
}

#[test]
fn paste_node_rejects_non_input_without_pasting() {
    let (agent, records) = system_action_agent(false, None, None, None);

    assert!(agent.paste_node(2, "replacement").is_err());

    assert!(records.calls.lock().unwrap().is_empty());
    assert!(records.pastes.lock().unwrap().is_empty());
}

#[test]
fn rotate_and_keyboard_forward_arguments_and_errors() {
    let (agent, records) = system_action_agent(
        false,
        None,
        Some("rotation failed"),
        Some("keyboard failed"),
    );

    assert_eq!(agent.rotate(true).unwrap_err(), "rotation failed");
    assert_eq!(agent.keyboard(false).unwrap_err(), "keyboard failed");

    assert_eq!(*records.rotations.lock().unwrap(), vec![true]);
    assert_eq!(*records.keyboards.lock().unwrap(), vec![false]);
    assert_eq!(*records.calls.lock().unwrap(), vec!["rotate", "keyboard"]);
}

fn two_window_agent() -> (Agent, Arc<WindowRecords>) {
    use nsh_core::{
        Command, EventMask, InputConfig, MountItem, ViewKind,
        desktop::{DesktopCommand, OpenWindow, WindowOptions},
    };

    let model = Arc::new(Model::default());
    let mut open = OpenWindow::default();
    open.options = WindowOptions::new("preferences");
    open.parent = Some(1);
    let mut items = vec![
        MountItem::Create {
            id: 10,
            kind: ViewKind::View,
        },
        MountItem::Frame {
            id: 10,
            frame: Bounds::new(Point::default(), size(px(100.0), px(100.0))),
        },
        MountItem::Insert {
            parent: 1,
            child: 10,
            before: None,
        },
        MountItem::Command {
            id: 10,
            command: Command::Desktop(Box::new(DesktopCommand::OpenWindow(Box::new(open)))),
        },
    ];
    let mut bounds = std::collections::HashMap::new();
    for (id, parent, input) in [(2, 1, false), (3, 1, true), (12, 10, false), (13, 10, true)] {
        let frame = Bounds::new(
            point(px(if input { 60.0 } else { 10.0 }), px(10.0)),
            size(px(20.0), px(20.0)),
        );
        let mut events = EventMask::default();
        events.set(EventKind::Press, true);
        events.set(EventKind::LongPress, true);
        items.extend([
            MountItem::Create {
                id,
                kind: if input {
                    ViewKind::Input
                } else {
                    ViewKind::View
                },
            },
            MountItem::Frame { id, frame },
            MountItem::Insert {
                parent,
                child: id,
                before: None,
            },
            MountItem::Events { id, events },
        ]);
        if input {
            items.push(MountItem::Input {
                id,
                config: Box::new(InputConfig::default()),
            });
        }
        bounds.insert(id, frame);
    }
    model.apply(&MountBatch { items });
    let windows = Arc::new(WindowRecords {
        selected: Mutex::new(1),
        nodes: [(2, 1), (3, 1), (12, 10), (13, 10)].into(),
        ..Default::default()
    });
    let native = GateNative {
        windows: Some(windows.clone()),
        bounds,
        text_bounds: Default::default(),
        hit: Ok(None),
        hits: Arc::default(),
        presses: Arc::default(),
        pressed_at: Arc::default(),
        long_presses: Arc::default(),
        typed: Arc::default(),
        viewport: size(px(100.0), px(100.0)),
    };
    (
        Agent::requiring_native_hit_test(model, Box::new(native)),
        windows,
    )
}

#[test]
fn node_actions_target_their_window_and_switch_back_to_main() {
    for action in [
        "press",
        "hover",
        "scroll",
        "drag",
        "drag_to",
        "drag_by",
        "long_press",
        "type",
        "set_value",
        "key",
    ] {
        let (agent, windows) = two_window_agent();
        for (window, id, input) in [(10, 12, 13), (1, 2, 3)] {
            let before = windows.actions.lock().unwrap().len();
            let delta = point(px(5.0), px(5.0));
            match action {
                "press" => agent.press_node(id),
                "hover" => agent.hover_node(id),
                "scroll" => agent.scroll_node(id, delta),
                "drag" => agent.drag_nodes(id, id),
                "drag_to" => agent.drag_to(id, point(px(25.0), px(25.0))),
                "drag_by" => agent.drag_node_by(id, None, delta),
                "long_press" => agent.long_press_node(id, Some(Duration::ZERO)),
                "type" => agent.type_node(input, "hello"),
                "set_value" => agent.set_value_node(input, "hello"),
                "key" => agent.key_node(id, "enter"),
                _ => unreachable!(),
            }
            .unwrap_or_else(|error| panic!("{action} in window {window}: {error}"));
            let actions = windows.actions.lock().unwrap();
            assert!(actions.len() > before, "{action}");
            assert!(
                actions[before..]
                    .iter()
                    .all(|(_, selected, _)| *selected == window),
                "{action}: {actions:?}"
            );
            let expected = point(
                px(if matches!(action, "type" | "set_value") {
                    70.0
                } else {
                    20.0
                }),
                px(20.0),
            );
            for (_, _, at) in &actions[before..] {
                if let Some(at) = at {
                    assert_eq!(*at, expected, "{action}");
                }
            }
            assert_eq!(
                windows.targets.lock().unwrap().last(),
                Some(&window),
                "{action}"
            );
        }
    }
}

#[test]
fn point_actions_keep_the_explicitly_selected_window() {
    let (agent, windows) = two_window_agent();
    agent.native.target_window(10).unwrap();
    windows.targets.lock().unwrap().clear();
    let at = point(px(20.0), px(20.0));
    agent.press_at(at).unwrap();
    agent.hover_at(at).unwrap();
    agent.scroll_at(at, point(px(0.0), px(5.0))).unwrap();
    agent.drag_at(at, point(px(25.0), px(25.0))).unwrap();
    agent.long_press_at(at, Some(Duration::ZERO)).unwrap();
    assert!(windows.targets.lock().unwrap().is_empty());
    let actions = windows.actions.lock().unwrap();
    assert_eq!(actions.len(), 5);
    assert!(
        actions
            .iter()
            .all(|(_, window, at)| *window == 10 && *at == Some(point(px(20.0), px(20.0))))
    );
}

#[test]
fn missing_nodes_and_cross_window_drags_dispatch_no_input() {
    let (agent, windows) = two_window_agent();
    let missing_node = "`node 999` is offscreen (node unavailable)";
    assert_eq!(agent.press_node(999).unwrap_err(), missing_node);
    assert_eq!(agent.drag_nodes(12, 999).unwrap_err(), missing_node);
    assert_eq!(agent.drag_nodes(999, 12).unwrap_err(), missing_node);
    assert_eq!(
        agent.drag_nodes(12, 2).unwrap_err(),
        "drag endpoints are in different windows"
    );
    assert!(windows.actions.lock().unwrap().is_empty());
    assert!(windows.targets.lock().unwrap().is_empty());
}

#[test]
fn unavailable_node_window_dispatches_no_input() {
    let (mut agent, windows) = two_window_agent();
    let closed = Arc::new(WindowRecords {
        selected: Mutex::new(1),
        unavailable: Some(10),
        ..Default::default()
    });
    agent.native = Box::new(GateNative {
        windows: Some(closed.clone()),
        bounds: Default::default(),
        text_bounds: Default::default(),
        hit: Ok(None),
        hits: Arc::default(),
        presses: Arc::default(),
        pressed_at: Arc::default(),
        long_presses: Arc::default(),
        typed: Arc::default(),
        viewport: size(px(100.0), px(100.0)),
    });
    assert_eq!(
        agent.press_node(12).unwrap_err(),
        "native window is unavailable"
    );
    assert!(closed.actions.lock().unwrap().is_empty());
    assert!(windows.actions.lock().unwrap().is_empty());
}

fn long_press_only_scene(disabled: bool) -> Arc<Model> {
    use nsh_core::{EventMask, MountItem, ViewKind, ViewProps};

    let model = gate_scene(ViewKind::View, disabled, false);
    let mut events = EventMask::default();
    events.set(EventKind::LongPress, true);
    let mut props = ViewProps {
        test_id: Some("target".into()),
        ..ViewProps::default()
    };
    props.accessibility.state.disabled = disabled;
    props.long_press_delay = Some(325.0);
    model.apply(&MountBatch {
        items: vec![
            MountItem::Events { id: 2, events },
            MountItem::ViewProps {
                id: 2,
                props: Box::new(props),
            },
        ],
    });
    model
}

fn gate_agent(
    model: Arc<Model>,
    hit: Result<Option<NodeId>, String>,
    bounds: &[(NodeId, Bounds<Points>)],
    viewport: Size<Points>,
    hits: Arc<Mutex<Vec<Point<Points>>>>,
    presses: Arc<std::sync::atomic::AtomicUsize>,
    typed: Arc<std::sync::atomic::AtomicUsize>,
) -> Agent {
    Agent::requiring_native_hit_test(
        model,
        Box::new(GateNative {
            windows: None,
            bounds: bounds.iter().copied().collect(),
            text_bounds: Default::default(),
            hit,
            hits,
            presses,
            pressed_at: Arc::new(Mutex::new(Vec::new())),
            long_presses: Arc::new(Mutex::new(Vec::new())),
            typed,
            viewport,
        }),
    )
}

fn gate_long_press_agent(model: Arc<Model>, long_presses: LongPressRecords) -> Agent {
    let bounds = [(
        2,
        Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0))),
    )];
    gate_long_press_agent_with(model, Ok(Some(2)), &bounds, &[], long_presses)
}

fn gate_long_press_agent_with(
    model: Arc<Model>,
    hit: Result<Option<NodeId>, String>,
    bounds: &[(NodeId, Bounds<Points>)],
    text_bounds: &TextBoundsTable,
    long_presses: LongPressRecords,
) -> Agent {
    Agent::requiring_native_hit_test(
        model,
        Box::new(GateNative {
            windows: None,
            bounds: bounds.iter().copied().collect(),
            text_bounds: text_bounds.iter().copied().collect(),
            hit,
            hits: Arc::default(),
            presses: Arc::default(),
            pressed_at: Arc::default(),
            long_presses,
            typed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            viewport: size(px(100.0), px(100.0)),
        }),
    )
}

#[test]
fn long_press_only_nodes_are_held_by_node_and_point_actions() {
    let long_presses = Arc::new(Mutex::new(Vec::new()));
    let scene = long_press_only_scene(false);
    assert!(
        !scene
            .semantic_tree()
            .unwrap()
            .iter()
            .find(|node| node.id == 2)
            .unwrap()
            .pressable
    );
    let agent = gate_long_press_agent(scene, long_presses.clone());
    let center = point(px(50.0), px(50.0));
    let explicit = Duration::from_millis(800);

    agent.long_press_node(2, None).unwrap();
    agent.long_press_node(2, Some(explicit)).unwrap();
    agent.long_press_at(center, None).unwrap();
    agent.long_press_at(center, Some(explicit)).unwrap();

    assert_eq!(
        *long_presses.lock().unwrap(),
        vec![
            (center, Duration::from_millis(425)),
            (center, explicit),
            (center, Duration::from_millis(425)),
            (center, explicit),
        ]
    );
}

#[test]
fn disabled_long_press_only_nodes_are_refused() {
    let long_presses = Arc::new(Mutex::new(Vec::new()));
    let agent = gate_long_press_agent(long_press_only_scene(true), long_presses.clone());
    let center = point(px(50.0), px(50.0));

    assert!(
        agent
            .long_press_node(2, None)
            .unwrap_err()
            .contains("disabled")
    );
    assert!(
        agent
            .long_press_at(center, None)
            .unwrap_err()
            .contains("disabled")
    );
    assert!(long_presses.lock().unwrap().is_empty());
}

#[test]
fn long_press_refuses_hidden_receivers_but_accepts_decorative_children() {
    use nsh_core::{EventMask, MountItem, ViewKind, ViewProps};

    let bounds = [(
        2,
        Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0))),
    )];
    let center = point(px(50.0), px(50.0));
    for takes_gestures in [true, false] {
        let model = long_press_only_scene(false);
        let mut hidden = ViewProps::default();
        hidden.accessibility.hidden = true;
        let mut items = vec![
            MountItem::Create {
                id: 3,
                kind: ViewKind::View,
            },
            MountItem::ViewProps {
                id: 3,
                props: Box::new(hidden),
            },
            MountItem::Insert {
                parent: 2,
                child: 3,
                before: None,
            },
        ];
        if takes_gestures {
            let mut events = EventMask::default();
            events.set(EventKind::Press, true);
            events.set(EventKind::LongPress, true);
            items.push(MountItem::Events { id: 3, events });
        }
        model.apply(&MountBatch { items });

        let long_presses = Arc::new(Mutex::new(Vec::new()));
        let agent =
            gate_long_press_agent_with(model, Ok(Some(3)), &bounds, &[], long_presses.clone());
        let result = agent.long_press_node(2, None);
        if takes_gestures {
            let error = result.unwrap_err();
            assert!(error.contains("goes to node 3"), "{error}");
            assert!(error.contains("accessibility-hidden"), "{error}");
            assert!(long_presses.lock().unwrap().is_empty());
        } else {
            result.unwrap();
            assert_eq!(
                *long_presses.lock().unwrap(),
                vec![(center, Duration::from_millis(425))]
            );
        }
    }
}

#[test]
fn a_hold_through_a_touch_observer_still_reaches_the_parents_recognizer() {
    use nsh_core::{EventMask, MountItem, ViewKind};

    // A raw `Touch` listener installs no recognizer: a hold on the child
    // still fires the parent's long-press recognizer.
    let model = long_press_only_scene(false);
    let mut touch = EventMask::default();
    touch.set(EventKind::Touch, true);
    model.apply(&MountBatch {
        items: vec![
            MountItem::Create {
                id: 3,
                kind: ViewKind::View,
            },
            MountItem::Frame {
                id: 3,
                frame: Bounds::new(point(px(30.0), px(40.0)), size(px(20.0), px(20.0))),
            },
            MountItem::Insert {
                parent: 2,
                child: 3,
                before: None,
            },
            MountItem::Events {
                id: 3,
                events: touch,
            },
        ],
    });
    let bounds = [(
        2,
        Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0))),
    )];
    let long_presses = Arc::new(Mutex::new(Vec::new()));
    let agent = gate_long_press_agent_with(model, Ok(Some(3)), &bounds, &[], long_presses.clone());
    let center = point(px(50.0), px(50.0));

    agent.long_press_node(2, None).unwrap();

    assert_eq!(
        *long_presses.lock().unwrap(),
        vec![(center, Duration::from_millis(425))]
    );
}

#[test]
fn a_hold_refuses_a_touch_observer_that_also_takes_presses() {
    use nsh_core::{EventMask, MountItem, ViewKind};

    // Touch + PressDown takes the press in native routing (`has_press`
    // covers the down/up listeners), so the parent's recognizer never
    // fires: the hold must fail rather than report success.
    let model = long_press_only_scene(false);
    let mut events = EventMask::default();
    events.set(EventKind::Touch, true);
    events.set(EventKind::PressDown, true);
    model.apply(&MountBatch {
        items: vec![
            MountItem::Create {
                id: 3,
                kind: ViewKind::View,
            },
            MountItem::Frame {
                id: 3,
                frame: Bounds::new(point(px(30.0), px(40.0)), size(px(20.0), px(20.0))),
            },
            MountItem::Insert {
                parent: 2,
                child: 3,
                before: None,
            },
            MountItem::Events { id: 3, events },
        ],
    });
    let bounds = [(
        2,
        Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0))),
    )];
    let long_presses = Arc::new(Mutex::new(Vec::new()));
    let agent = gate_long_press_agent_with(model, Ok(Some(3)), &bounds, &[], long_presses.clone());

    let error = agent.long_press_node(2, None).unwrap_err();

    assert!(error.contains("occluded"), "{error}");
    assert!(long_presses.lock().unwrap().is_empty());
}

#[test]
fn a_hold_refuses_a_press_down_child_even_without_touch() {
    use nsh_core::{EventMask, MountItem, ViewKind};

    // PressDown alone also takes the press natively: the model's pick
    // steps over it to the long-pressable parent, but the recognizer
    // there can never fire.
    let model = long_press_only_scene(false);
    let mut events = EventMask::default();
    events.set(EventKind::PressDown, true);
    model.apply(&MountBatch {
        items: vec![
            MountItem::Create {
                id: 3,
                kind: ViewKind::View,
            },
            MountItem::Frame {
                id: 3,
                frame: Bounds::new(point(px(30.0), px(40.0)), size(px(20.0), px(20.0))),
            },
            MountItem::Insert {
                parent: 2,
                child: 3,
                before: None,
            },
            MountItem::Events { id: 3, events },
        ],
    });
    let bounds = [(
        2,
        Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0))),
    )];
    let long_presses = Arc::new(Mutex::new(Vec::new()));
    let agent = gate_long_press_agent_with(model, Ok(Some(3)), &bounds, &[], long_presses.clone());

    let error = agent.long_press_node(2, None).unwrap_err();

    assert!(error.contains("occluded"), "{error}");
    assert!(long_presses.lock().unwrap().is_empty());
}

#[test]
fn a_hold_over_a_press_down_child_fails_when_the_parent_also_takes_presses() {
    use nsh_core::{EventMask, MountItem, ViewKind};

    // A card whose long-press callback is native keeps nothing when a
    // press-down child owns the gesture: touches still reach its touch
    // listener, but the native callback can never run.
    let model = long_press_only_scene(false);
    let mut events = EventMask::default();
    events.set(EventKind::LongPress, true);
    events.set(EventKind::Touch, true);
    let mut press_down = EventMask::default();
    press_down.set(EventKind::PressDown, true);
    model.apply(&MountBatch {
        items: vec![
            MountItem::Events { id: 2, events },
            MountItem::Create {
                id: 3,
                kind: ViewKind::View,
            },
            MountItem::Frame {
                id: 3,
                frame: Bounds::new(point(px(30.0), px(40.0)), size(px(20.0), px(20.0))),
            },
            MountItem::Insert {
                parent: 2,
                child: 3,
                before: None,
            },
            MountItem::Events {
                id: 3,
                events: press_down,
            },
        ],
    });
    let bounds = [(
        2,
        Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0))),
    )];
    let long_presses = Arc::new(Mutex::new(Vec::new()));
    let agent = gate_long_press_agent_with(model, Ok(Some(3)), &bounds, &[], long_presses.clone());

    let error = agent.long_press_node(2, None).unwrap_err();

    assert!(error.contains("occluded"), "{error}");
    assert!(long_presses.lock().unwrap().is_empty());
}

#[test]
fn a_hold_through_a_press_down_child_reaches_a_touch_observer() {
    use nsh_core::{EventMask, MountItem, ViewKind, ViewProps};

    // A card holding from raw touches keeps its hold when the press lands
    // on an enabled child that owns the native press (press-down only):
    // touches follow the raw touch path beside the recognizers.
    let model = gate_scene(ViewKind::View, false, false);
    let mut touch = EventMask::default();
    touch.set(EventKind::Touch, true);
    let mut press_down = EventMask::default();
    press_down.set(EventKind::PressDown, true);
    model.apply(&MountBatch {
        items: vec![
            MountItem::Events {
                id: 2,
                events: touch,
            },
            MountItem::Create {
                id: 3,
                kind: ViewKind::View,
            },
            MountItem::Frame {
                id: 3,
                frame: Bounds::new(point(px(30.0), px(40.0)), size(px(20.0), px(20.0))),
            },
            MountItem::Insert {
                parent: 2,
                child: 3,
                before: None,
            },
            MountItem::Events {
                id: 3,
                events: press_down,
            },
            MountItem::ViewProps {
                id: 3,
                props: Box::new(ViewProps {
                    test_id: Some("child".into()),
                    ..ViewProps::default()
                }),
            },
        ],
    });
    let bounds = [(
        2,
        Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0))),
    )];
    let long_presses = Arc::new(Mutex::new(Vec::new()));
    let agent = gate_long_press_agent_with(model, Ok(Some(3)), &bounds, &[], long_presses.clone());

    agent.long_press_node(2, None).unwrap();

    assert_eq!(
        *long_presses.lock().unwrap(),
        vec![(point(px(50.0), px(50.0)), Duration::from_millis(600))]
    );
}

#[test]
fn a_touch_observer_takes_a_hold_targeted_at_it() {
    use nsh_core::{EventMask, MountItem, ViewKind};

    // An app can hold from raw touches, so a hold aimed at the observer
    // itself still goes through even under an ancestor's recognizer.
    let model = long_press_only_scene(false);
    let mut touch = EventMask::default();
    touch.set(EventKind::Touch, true);
    let child_bounds = Bounds::new(point(px(30.0), px(40.0)), size(px(20.0), px(20.0)));
    model.apply(&MountBatch {
        items: vec![
            MountItem::Create {
                id: 3,
                kind: ViewKind::View,
            },
            MountItem::Frame {
                id: 3,
                frame: child_bounds,
            },
            MountItem::Insert {
                parent: 2,
                child: 3,
                before: None,
            },
            MountItem::Events {
                id: 3,
                events: touch,
            },
        ],
    });
    let bounds = [
        (
            2,
            Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0))),
        ),
        (3, child_bounds),
    ];
    let long_presses = Arc::new(Mutex::new(Vec::new()));
    let agent = gate_long_press_agent_with(model, Ok(Some(3)), &bounds, &[], long_presses.clone());

    agent.long_press_node(3, None).unwrap();

    assert_eq!(
        *long_presses.lock().unwrap(),
        vec![(child_bounds.center(), Duration::from_millis(600))]
    );
}

#[test]
fn a_targeted_hold_refuses_a_nested_input_hit() {
    use nsh_core::{InputConfig, MountItem, ViewKind};

    // A hold on a field inside a long-pressable card goes to the field's
    // text selection, not the card's recognizer.
    let model = long_press_only_scene(false);
    model.apply(&MountBatch {
        items: vec![
            MountItem::Create {
                id: 3,
                kind: ViewKind::Input,
            },
            MountItem::Frame {
                id: 3,
                frame: Bounds::new(point(px(30.0), px(40.0)), size(px(20.0), px(20.0))),
            },
            MountItem::Insert {
                parent: 2,
                child: 3,
                before: None,
            },
            MountItem::Input {
                id: 3,
                config: Box::new(InputConfig::default()),
            },
        ],
    });
    let bounds = [(
        2,
        Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0))),
    )];
    let long_presses = Arc::new(Mutex::new(Vec::new()));
    let agent = gate_long_press_agent_with(model, Ok(Some(3)), &bounds, &[], long_presses.clone());

    let error = agent.long_press_node(2, None).unwrap_err();
    assert!(error.contains("occluded_by"), "{error}");
    assert!(long_presses.lock().unwrap().is_empty());
}

#[test]
fn inputs_without_listeners_accept_node_and_point_long_presses() {
    use nsh_core::{EventMask, MountItem, ViewKind};

    let model = gate_scene(ViewKind::Input, false, false);
    model.apply(&MountBatch {
        items: vec![MountItem::Events {
            id: 2,
            events: EventMask::default(),
        }],
    });
    let long_presses = Arc::new(Mutex::new(Vec::new()));
    let agent = gate_long_press_agent(model, long_presses.clone());
    let center = point(px(50.0), px(50.0));
    let hold = Duration::from_millis(800);

    agent.long_press_node(2, Some(hold)).unwrap();
    agent.long_press_at(center, Some(hold)).unwrap();

    assert_eq!(
        *long_presses.lock().unwrap(),
        vec![(center, hold), (center, hold)]
    );
}

#[test]
fn long_press_at_targets_an_inline_span_under_its_text_owner_hit() {
    use nsh_core::{AttributedText, EventMask, MountItem, TextLayout, TextLink, ViewKind};

    let text_bounds = Bounds::new(point(px(10.0), px(10.0)), size(px(80.0), px(40.0)));
    let link_bounds = Bounds::new(point(px(30.0), px(20.0)), size(px(40.0), px(20.0)));
    let mut events = EventMask::default();
    events.set(EventKind::Press, true);
    events.set(EventKind::LongPress, true);
    let model = Arc::new(Model::default());
    model.apply(&MountBatch {
        items: vec![
            MountItem::Create {
                id: 2,
                kind: ViewKind::View,
            },
            MountItem::Frame {
                id: 2,
                frame: text_bounds,
            },
            MountItem::Insert {
                parent: 1,
                child: 2,
                before: None,
            },
            MountItem::Create {
                id: 3,
                kind: ViewKind::Text,
            },
            MountItem::Frame {
                id: 3,
                frame: Bounds::new(Point::default(), text_bounds.size),
            },
            MountItem::Insert {
                parent: 2,
                child: 3,
                before: None,
            },
            MountItem::Text {
                id: 3,
                text: Arc::new(AttributedText {
                    text: "link".into(),
                    links: vec![TextLink {
                        start: 0,
                        end: 4,
                        node: 4,
                        events,
                        focus_stop: true,
                    }],
                    ..Default::default()
                }),
                layout: TextLayout(Arc::new(())),
                content: Bounds::new(Point::default(), text_bounds.size),
            },
        ],
    });

    let long_presses = Arc::new(Mutex::new(Vec::new()));
    let agent = gate_long_press_agent_with(
        model,
        Ok(Some(3)),
        &[(2, text_bounds), (3, text_bounds)],
        &[((3, 0, 4), link_bounds)],
        long_presses.clone(),
    );
    let at = link_bounds.center();
    let hold = Duration::from_millis(800);

    agent.long_press_at(at, Some(hold)).unwrap();

    assert_eq!(*long_presses.lock().unwrap(), vec![(at, hold)]);
}

fn target_scene(nodes: Vec<SemanticNode>) -> Arc<WithNodes> {
    Arc::new(WithNodes(
        nodes,
        Vec::new(),
        RemoteScene::new(move |step| match step {
            TestStep::Scene {
                query: nsh_dev::protocol::SceneQuery::Activity,
            } => Ok(serde_json::json!({"batches": 1, "since_last_ms": 1000.0})),
            TestStep::Scene {
                query: nsh_dev::protocol::SceneQuery::TakeErrors,
            } => Ok(serde_json::json!([])),
            TestStep::Scene {
                query: nsh_dev::protocol::SceneQuery::ScrollAncestors { .. },
            } => Ok(serde_json::json!([])),
            _ => Err("unexpected scene query".into()),
        }),
    ))
}

fn manual_target_scene(
    nodes: Vec<SemanticNode>,
    clock_changes: Arc<Mutex<Vec<nsh_dev::protocol::ClockChange>>>,
) -> Arc<WithNodes> {
    use nsh_dev::protocol::SceneQuery;

    Arc::new(WithNodes(
        nodes,
        Vec::new(),
        RemoteScene::new(move |step| match step {
            TestStep::Scene {
                query: SceneQuery::Activity,
            } => Ok(serde_json::json!({"batches": 1, "since_last_ms": 1000.0})),
            TestStep::Scene {
                query: SceneQuery::TakeErrors,
            } => Ok(serde_json::json!([])),
            TestStep::Scene {
                query: SceneQuery::ScrollAncestors { .. },
            } => Ok(serde_json::json!([])),
            TestStep::Scene {
                query: SceneQuery::Pick { selector, target },
            } => {
                let pick = if selector == "view:2" && matches!(target, PickTarget::LongPressable) {
                    Pick::One(2)
                } else {
                    Pick::None
                };
                Ok(pick_value(pick))
            }
            TestStep::Scene {
                query:
                    SceneQuery::CanvasNode { .. }
                    | SceneQuery::MatchInside { .. }
                    | SceneQuery::Span { .. }
                    | SceneQuery::LongPressDelay { .. },
            } => Ok(serde_json::Value::Null),
            TestStep::Scene {
                query: SceneQuery::Clock { change },
            } => {
                clock_changes.lock().unwrap().push(change);
                Ok(serde_json::Value::Null)
            }
            _ => Err("unexpected scene query".into()),
        })
        .with_manual_clock(true),
    ))
}

#[test]
fn manual_clock_long_press_advances_the_remaining_hold() {
    use nsh_dev::protocol::ClockChange;

    let clock_changes = Arc::new(Mutex::new(Vec::new()));
    let long_presses = Arc::new(Mutex::new(Vec::new()));
    let bounds = Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0)));
    let scene = manual_target_scene(
        vec![semantic_node(2, Some(1), false, SemanticTarget::View)],
        clock_changes.clone(),
    );
    let agent = Agent::new(
        scene,
        Box::new(GateNative {
            windows: None,
            bounds: [(2, bounds)].into(),
            text_bounds: Default::default(),
            hit: Ok(Some(2)),
            hits: Arc::default(),
            presses: Arc::default(),
            pressed_at: Arc::default(),
            long_presses: long_presses.clone(),
            typed: Arc::default(),
            viewport: size(px(100.0), px(100.0)),
        }),
    );
    let hold = Duration::from_millis(800);

    agent.long_press_node(2, Some(hold)).unwrap();

    assert_eq!(*long_presses.lock().unwrap(), vec![(bounds.center(), hold)]);
    let mut changes = clock_changes.lock().unwrap();
    assert_eq!(changes.len(), 1);
    assert!(matches!(
        changes.pop().unwrap(),
        ClockChange::RunFor { ms }
            if ms == hold.saturating_sub(LANDING).as_millis() as u64
    ));
}

fn semantic_node(
    id: NodeId,
    parent: Option<NodeId>,
    pressable: bool,
    native_target: SemanticTarget,
) -> SemanticNode {
    SemanticNode {
        id,
        parent,
        enabled: true,
        pressable,
        native_target,
        ..SemanticNode::default()
    }
}

#[test]
fn button_label_press_rejects_siblings_and_accepts_only_its_ancestry() {
    let label_bounds = Bounds::new(point(px(20.0), px(10.0)), size(px(40.0), px(20.0)));
    for hit in [2, 3, 4, 5, 6, 99] {
        let nodes = vec![
            semantic_node(1, None, false, SemanticTarget::View),
            semantic_node(2, Some(1), true, SemanticTarget::View),
            semantic_node(6, Some(2), false, SemanticTarget::View),
            semantic_node(3, Some(6), false, SemanticTarget::View),
            semantic_node(4, Some(2), true, SemanticTarget::View),
            semantic_node(5, Some(4), false, SemanticTarget::View),
        ];
        let presses = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let pressed_at = Arc::new(Mutex::new(Vec::new()));
        let agent = Agent::requiring_native_hit_test(
            target_scene(nodes),
            Box::new(GateNative {
                windows: None,
                bounds: [
                    (2, Bounds::new(Point::default(), size(px(80.0), px(40.0)))),
                    (3, label_bounds),
                ]
                .into(),
                text_bounds: Default::default(),
                hit: Ok(Some(hit)),
                hits: Arc::new(Mutex::new(Vec::new())),
                presses: presses.clone(),
                pressed_at: pressed_at.clone(),
                long_presses: Arc::new(Mutex::new(Vec::new())),
                typed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                viewport: size(px(100.0), px(100.0)),
            }),
        );
        let result = agent.press_node(3);
        if matches!(hit, 4 | 5 | 99) {
            assert!(result.unwrap_err().contains(&format!("occluded_by: {hit}")));
            assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 0);
            assert!(pressed_at.lock().unwrap().is_empty());
        } else {
            result.unwrap();
            assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 1);
            assert_eq!(*pressed_at.lock().unwrap(), vec![label_bounds.center()]);
        }
    }
}

#[test]
fn button_label_press_rejects_a_sibling_sharing_its_receiver() {
    // Node 4 is not pressable itself: a hit on it (or its child 5)
    // routes to node 2 — the same receiver the target's press takes.
    // It still covers the label, so the press must not pass.
    let label_bounds = Bounds::new(point(px(20.0), px(10.0)), size(px(40.0), px(20.0)));
    for hit in [4, 5] {
        let nodes = vec![
            semantic_node(1, None, false, SemanticTarget::View),
            semantic_node(2, Some(1), true, SemanticTarget::View),
            semantic_node(6, Some(2), false, SemanticTarget::View),
            semantic_node(3, Some(6), false, SemanticTarget::View),
            semantic_node(4, Some(2), false, SemanticTarget::View),
            semantic_node(5, Some(4), false, SemanticTarget::View),
        ];
        let agent = Agent::requiring_native_hit_test(
            target_scene(nodes),
            Box::new(GateNative {
                windows: None,
                bounds: [
                    (2, Bounds::new(Point::default(), size(px(80.0), px(40.0)))),
                    (3, label_bounds),
                ]
                .into(),
                text_bounds: Default::default(),
                hit: Ok(Some(hit)),
                hits: Arc::new(Mutex::new(Vec::new())),
                presses: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                pressed_at: Arc::new(Mutex::new(Vec::new())),
                long_presses: Arc::new(Mutex::new(Vec::new())),
                typed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                viewport: size(px(100.0), px(100.0)),
            }),
        );
        let result = agent.press_node(3);
        assert!(result.unwrap_err().contains(&format!("occluded_by: {hit}")));
    }
}

#[test]
fn a_hit_on_an_accessibility_hidden_child_is_a_hit_on_its_owner() {
    use nsh_core::{MountItem, ViewKind, ViewProps, geometry::Size};

    let bounds = [(
        2,
        Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0))),
    )];
    for hit in [3, 4] {
        let model = gate_scene(ViewKind::View, false, false);
        let mut icon = ViewProps::default();
        icon.accessibility.hidden = true;
        model.apply(&MountBatch {
            items: vec![
                // A decorative icon inside the button, and an overlay
                // beside it.
                MountItem::Create {
                    id: 3,
                    kind: ViewKind::View,
                },
                MountItem::ViewProps {
                    id: 3,
                    props: Box::new(icon),
                },
                MountItem::Insert {
                    parent: 2,
                    child: 3,
                    before: None,
                },
                MountItem::Create {
                    id: 4,
                    kind: ViewKind::View,
                },
                MountItem::Insert {
                    parent: 1,
                    child: 4,
                    before: None,
                },
            ],
        });
        let presses = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let agent = gate_agent(
            model,
            Ok(Some(hit)),
            &bounds,
            Size::new(px(100.0), px(100.0)),
            Arc::new(Mutex::new(Vec::new())),
            presses.clone(),
            Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        );
        let result = agent.press_node(2);
        if hit == 3 {
            result.unwrap();
            assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 1);
        } else {
            assert!(result.unwrap_err().contains("occluded_by: 4"));
            assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 0);
        }
    }
}

/// A press on a button whose point lands on an accessibility-hidden
/// child (or a view inside it) that takes presses itself is refused:
/// the child would get it.
#[test]
fn a_press_refuses_a_hidden_child_that_takes_presses_itself() {
    use nsh_core::{EventMask, MountItem, ViewKind, ViewProps, geometry::Size};

    let bounds = [(
        2,
        Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0))),
    )];
    for hit in [3, 5] {
        let model = gate_scene(ViewKind::View, false, false);
        let mut hidden = ViewProps::default();
        hidden.accessibility.hidden = true;
        let mut events = EventMask::default();
        events.set(EventKind::Press, true);
        model.apply(&MountBatch {
            items: vec![
                // A hidden child of the button with its own press
                // handler, and a view inside it.
                MountItem::Create {
                    id: 3,
                    kind: ViewKind::View,
                },
                MountItem::ViewProps {
                    id: 3,
                    props: Box::new(hidden),
                },
                MountItem::Events { id: 3, events },
                MountItem::Insert {
                    parent: 2,
                    child: 3,
                    before: None,
                },
                MountItem::Create {
                    id: 5,
                    kind: ViewKind::View,
                },
                MountItem::Insert {
                    parent: 3,
                    child: 5,
                    before: None,
                },
            ],
        });
        let target = model
            .semantic_tree()
            .unwrap()
            .into_iter()
            .find(|node| node.id == 2)
            .unwrap();
        assert_eq!(target.hidden_descendants, [3, 5]);
        assert_eq!(target.hidden_press_owners, [(3, 3), (5, 3)]);
        let presses = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let agent = gate_agent(
            model,
            Ok(Some(hit)),
            &bounds,
            Size::new(px(100.0), px(100.0)),
            Arc::new(Mutex::new(Vec::new())),
            presses.clone(),
            Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        );
        let error = agent.press_node(2).unwrap_err();
        assert!(
            error.contains("goes to node 3") && error.contains("accessibility-hidden"),
            "{error}"
        );
        assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 0);
    }
}

#[test]
fn press_at_rejects_a_label_nothing_pressable_owns() {
    let label_bounds = Bounds::new(point(px(20.0), px(10.0)), size(px(40.0), px(20.0)));
    for button in [true, false] {
        let nodes = vec![
            semantic_node(1, None, false, SemanticTarget::View),
            semantic_node(2, Some(1), button, SemanticTarget::View),
            semantic_node(3, Some(2), false, SemanticTarget::View),
        ];
        let presses = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let agent = Agent::requiring_native_hit_test(
            target_scene(nodes),
            Box::new(GateNative {
                windows: None,
                bounds: [
                    (2, Bounds::new(Point::default(), size(px(80.0), px(40.0)))),
                    (3, label_bounds),
                ]
                .into(),
                text_bounds: Default::default(),
                hit: Ok(Some(3)),
                hits: Arc::new(Mutex::new(Vec::new())),
                presses: presses.clone(),
                pressed_at: Arc::new(Mutex::new(Vec::new())),
                long_presses: Arc::new(Mutex::new(Vec::new())),
                typed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                viewport: size(px(100.0), px(100.0)),
            }),
        );
        let result = agent.press_at(label_bounds.center());
        if button {
            result.unwrap();
            assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 1);
        } else {
            assert_eq!(result.unwrap_err(), "node 3 is not pressable");
            assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 0);
        }
    }
}

/// A coordinate press on a link in a text nothing pressable owns goes
/// to the link; elsewhere in the text it is refused.
#[test]
fn press_at_takes_a_link_in_text_nothing_pressable_owns() {
    let text_bounds = Bounds::new(point(px(10.0), px(10.0)), size(px(200.0), px(40.0)));
    let link_bounds = Bounds::new(point(px(40.0), px(15.0)), size(px(30.0), px(10.0)));
    let link_at = link_bounds.center();
    let plain_at = point(px(150.0), px(30.0));
    for (hit, at, enabled, expected) in [
        (4, link_at, true, Ok(())),
        (4, plain_at, true, Err("node 4 is not pressable")),
        (4, link_at, false, Err("node 11 is disabled")),
        (
            9,
            link_at,
            true,
            Err("target_gone: coordinate has no semantic target"),
        ),
    ] {
        let mut link = semantic_node(
            11,
            Some(4),
            true,
            SemanticTarget::Text {
                owner: 4,
                start: 6,
                end: 10,
            },
        );
        link.enabled = enabled;
        let nodes = vec![
            semantic_node(1, None, false, SemanticTarget::View),
            semantic_node(4, Some(1), false, SemanticTarget::View),
            link,
        ];
        let presses = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let pressed_at = Arc::new(Mutex::new(Vec::new()));
        let agent = Agent::requiring_native_hit_test(
            target_scene(nodes),
            Box::new(GateNative {
                windows: None,
                bounds: [(4, text_bounds)].into(),
                text_bounds: [((4, 6, 10), link_bounds)].into(),
                hit: Ok(Some(hit)),
                hits: Arc::new(Mutex::new(Vec::new())),
                presses: presses.clone(),
                pressed_at: pressed_at.clone(),
                long_presses: Arc::new(Mutex::new(Vec::new())),
                typed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                viewport: size(px(300.0), px(200.0)),
            }),
        );
        let result = agent.press_at(at);
        match expected {
            Ok(()) => {
                result.unwrap();
                assert_eq!(*pressed_at.lock().unwrap(), vec![link_at]);
            }
            Err(message) => {
                assert_eq!(result.unwrap_err(), message);
                assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 0);
            }
        }
    }
}

#[test]
fn canvas_max_id_does_not_make_a_missing_native_hit_actionable() {
    use nsh_core::geometry::Size;

    let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(20.0), px(20.0)));
    let nodes = vec![
        semantic_node(1, None, false, SemanticTarget::View),
        semantic_node(4, Some(1), true, SemanticTarget::View),
        semantic_node(
            NodeId::MAX,
            Some(4),
            false,
            SemanticTarget::Canvas { owner: 4, bounds },
        ),
    ];
    assert!(is_descendant_or_same(
        NodeId::MAX,
        4,
        &mounted_parents(&nodes)
    ));
    for hit in [None, Some(4)] {
        let presses = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let agent = Agent::requiring_native_hit_test(
            target_scene(nodes.clone()),
            Box::new(GateNative {
                windows: None,
                bounds: [(4, bounds)].into(),
                text_bounds: Default::default(),
                hit: Ok(hit),
                hits: Arc::new(Mutex::new(Vec::new())),
                presses: presses.clone(),
                pressed_at: Arc::new(Mutex::new(Vec::new())),
                long_presses: Arc::new(Mutex::new(Vec::new())),
                typed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                viewport: Size::new(px(100.0), px(100.0)),
            }),
        );
        let result = agent.press_node(NodeId::MAX);
        if hit.is_none() {
            assert!(result.unwrap_err().contains("has no native hit"));
            assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 0);
        } else {
            result.unwrap();
            assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 1);
        }
    }
}

#[test]
fn synthetic_canvas_and_text_targets_use_precise_native_geometry() {
    let canvas_bounds = Bounds::new(point(px(60.0), px(20.0)), size(px(10.0), px(10.0)));
    let canvas_node = semantic_node(
        10,
        Some(4),
        false,
        SemanticTarget::Canvas {
            owner: 4,
            bounds: canvas_bounds,
        },
    );
    let canvas_nodes = vec![
        semantic_node(1, None, false, SemanticTarget::View),
        semantic_node(4, Some(1), true, SemanticTarget::View),
        canvas_node,
    ];
    let owner_bounds = Bounds::new(point(px(10.0), px(10.0)), size(px(200.0), px(100.0)));
    let expected_canvas = Bounds::new(point(px(70.0), px(30.0)), size(px(10.0), px(10.0)));
    let hits = Arc::new(Mutex::new(Vec::new()));
    let presses = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let pressed_at = Arc::new(Mutex::new(Vec::new()));
    let canvas = Agent::requiring_native_hit_test(
        target_scene(canvas_nodes),
        Box::new(GateNative {
            windows: None,
            bounds: [(4, owner_bounds)].into(),
            text_bounds: Default::default(),
            hit: Ok(Some(4)),
            hits: hits.clone(),
            presses: presses.clone(),
            pressed_at: pressed_at.clone(),
            long_presses: Arc::new(Mutex::new(Vec::new())),
            typed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            viewport: Size::new(px(300.0), px(200.0)),
        }),
    );
    assert_eq!(canvas.native_bounds(10), Some(expected_canvas));
    assert_eq!(canvas.native.bounds(10), None);
    canvas.press_node(10).unwrap();
    assert_eq!(*hits.lock().unwrap(), vec![expected_canvas.center()]);
    assert_eq!(*pressed_at.lock().unwrap(), vec![expected_canvas.center()]);
    assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 1);

    let span_bounds = Bounds::new(point(px(40.0), px(25.0)), size(px(20.0), px(10.0)));
    let span_nodes = vec![
        semantic_node(1, None, false, SemanticTarget::View),
        semantic_node(4, Some(1), false, SemanticTarget::View),
        semantic_node(
            11,
            Some(4),
            true,
            SemanticTarget::Text {
                owner: 4,
                start: 0,
                end: 4,
            },
        ),
    ];
    let hits = Arc::new(Mutex::new(Vec::new()));
    let presses = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let pressed_at = Arc::new(Mutex::new(Vec::new()));
    let span = Agent::requiring_native_hit_test(
        target_scene(span_nodes),
        Box::new(GateNative {
            windows: None,
            bounds: [(4, owner_bounds)].into(),
            text_bounds: [((4, 0, 4), span_bounds)].into(),
            hit: Ok(Some(4)),
            hits: hits.clone(),
            presses: presses.clone(),
            pressed_at: pressed_at.clone(),
            long_presses: Arc::new(Mutex::new(Vec::new())),
            typed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            viewport: Size::new(px(300.0), px(200.0)),
        }),
    );
    assert!(span.native_bounds(11).is_some());
    assert_eq!(span.native_bounds(11), Some(span_bounds));
    assert_eq!(span.native.bounds(11), None);
    span.press_node(11).unwrap();
    assert_eq!(*hits.lock().unwrap(), vec![span_bounds.center()]);
    assert_eq!(*pressed_at.lock().unwrap(), vec![span_bounds.center()]);
    assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 1);
}

#[test]
fn synthetic_canvas_targets_fail_closed_when_disabled_clipped_or_foreign_hit() {
    let local = Bounds::new(point(px(60.0), px(20.0)), size(px(10.0), px(10.0)));
    let target = || {
        semantic_node(
            10,
            Some(4),
            false,
            SemanticTarget::Canvas {
                owner: 4,
                bounds: local,
            },
        )
    };
    let owner_bounds = Bounds::new(point(px(10.0), px(10.0)), size(px(200.0), px(100.0)));
    let bounds = [
        (1, Bounds::new(Point::default(), size(px(50.0), px(50.0)))),
        (4, owner_bounds),
    ];
    let make_agent = |nodes: Vec<SemanticNode>, hit| {
        let presses = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let agent = Agent::requiring_native_hit_test(
            target_scene(nodes),
            Box::new(GateNative {
                windows: None,
                bounds: bounds.into(),
                text_bounds: Default::default(),
                hit,
                hits: Arc::new(Mutex::new(Vec::new())),
                presses: presses.clone(),
                pressed_at: Arc::new(Mutex::new(Vec::new())),
                long_presses: Arc::new(Mutex::new(Vec::new())),
                typed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                viewport: Size::new(px(300.0), px(200.0)),
            }),
        );
        (agent, presses)
    };

    let mut disabled = target();
    disabled.enabled = false;
    let (agent, presses) = make_agent(
        vec![
            semantic_node(1, None, false, SemanticTarget::View),
            semantic_node(4, Some(1), true, SemanticTarget::View),
            disabled,
        ],
        Ok(Some(4)),
    );
    assert!(agent.press_node(10).unwrap_err().contains("disabled"));
    assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 0);

    let mut clipping_root = semantic_node(1, None, false, SemanticTarget::View);
    clipping_root.clipping = true;
    let (agent, presses) = make_agent(
        vec![
            clipping_root,
            semantic_node(4, Some(1), true, SemanticTarget::View),
            target(),
        ],
        Ok(Some(4)),
    );
    assert!(agent.press_node(10).unwrap_err().contains("offscreen"));
    assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 0);

    let (agent, presses) = make_agent(
        vec![
            semantic_node(1, None, false, SemanticTarget::View),
            semantic_node(4, Some(1), true, SemanticTarget::View),
            target(),
        ],
        Ok(Some(99)),
    );
    assert!(
        agent
            .press_node(10)
            .unwrap_err()
            .contains("occluded_by: 99")
    );
    assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[test]
fn disabled_press_and_type_fail_before_native_hit_or_input() {
    use nsh_core::{ViewKind, geometry::Size};

    let model = gate_scene(ViewKind::Input, true, false);
    let hits = Arc::new(Mutex::new(Vec::new()));
    let presses = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let typed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let agent = gate_agent(
        model,
        Ok(Some(2)),
        &[(
            2,
            Bounds::new(point(px(0.0), px(0.0)), size(px(80.0), px(40.0))),
        )],
        Size::new(px(100.0), px(100.0)),
        hits.clone(),
        presses.clone(),
        typed.clone(),
    );
    assert!(agent.press_node(2).unwrap_err().contains("disabled"));
    assert!(
        agent
            .type_node(2, "forbidden")
            .unwrap_err()
            .contains("disabled")
    );
    assert!(hits.lock().unwrap().is_empty());
    assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(typed.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[test]
fn strict_actionability_reports_occlusion_offscreen_and_accepts_descendants() {
    use nsh_core::{MountItem, geometry::Size};

    let model = gate_scene(nsh_core::ViewKind::View, false, false);
    model.apply(&MountBatch {
        items: vec![
            MountItem::Create {
                id: 3,
                kind: nsh_core::ViewKind::View,
            },
            MountItem::Insert {
                parent: 2,
                child: 3,
                before: None,
            },
        ],
    });
    let bounds = [(
        2,
        Bounds::new(point(px(0.0), px(0.0)), size(px(80.0), px(40.0))),
    )];
    let viewport = Size::new(px(100.0), px(100.0));
    let hits = Arc::new(Mutex::new(Vec::new()));
    let presses = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let typed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let occluded = gate_agent(
        model.clone(),
        Ok(Some(99)),
        &bounds,
        viewport,
        hits.clone(),
        presses.clone(),
        typed.clone(),
    );
    assert!(
        occluded
            .actionable_point(2, 2, point(px(40.0), px(20.0)), "target")
            .unwrap_err()
            .contains("occluded_by: 99")
    );
    let offscreen = gate_agent(
        model.clone(),
        Ok(None),
        &[(
            2,
            Bounds::new(point(px(120.0), px(0.0)), size(px(20.0), px(20.0))),
        )],
        viewport,
        hits.clone(),
        presses.clone(),
        typed.clone(),
    );
    assert!(
        offscreen
            .actionable_point(2, 2, point(px(125.0), px(5.0)), "target")
            .unwrap_err()
            .contains("offscreen")
    );
    let descendant = gate_agent(
        model,
        Ok(Some(3)),
        &bounds,
        viewport,
        hits.clone(),
        presses.clone(),
        typed.clone(),
    );
    let accepted = descendant.actionable_point(2, 2, point(px(40.0), px(20.0)), "target");
    assert!(accepted.is_ok(), "{accepted:?}");
    assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(typed.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[test]
fn clipping_ancestor_without_native_bounds_fails_closed() {
    use nsh_core::{ViewKind, geometry::Size};

    let agent = gate_agent(
        gate_scene(ViewKind::View, false, true),
        Ok(Some(2)),
        &[(
            2,
            Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0))),
        )],
        Size::new(px(100.0), px(100.0)),
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    );
    assert!(
        agent
            .actionable_point(2, 2, point(px(40.0), px(20.0)), "target")
            .unwrap_err()
            .contains("offscreen")
    );
}

#[test]
fn strict_actionability_preserves_a_selected_point_inside_partial_clip() {
    use nsh_core::{ViewKind, geometry::Size};

    let model = gate_scene(ViewKind::View, false, true);
    let clip_bounds = Bounds::new(point(px(40.0), px(0.0)), size(px(60.0), px(100.0)));
    let target_bounds = Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0)));
    let hits = Arc::new(Mutex::new(Vec::new()));
    let agent = gate_agent(
        model,
        Ok(Some(2)),
        &[(1, clip_bounds), (2, target_bounds)],
        Size::new(px(100.0), px(100.0)),
        hits,
        Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    );
    let selected = point(px(45.0), px(30.0));
    assert_eq!(
        agent.actionable_point(2, 2, selected, "target"),
        Ok(selected)
    );
}

#[test]
fn a_view_without_native_bounds_is_not_on_screen_unless_strict() {
    use nsh_core::{ViewKind, geometry::Size};

    for strict in [false, true] {
        let native = GateNative {
            windows: None,
            bounds: Default::default(),
            text_bounds: Default::default(),
            hit: Ok(None),
            hits: Arc::new(Mutex::new(Vec::new())),
            presses: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            pressed_at: Arc::new(Mutex::new(Vec::new())),
            long_presses: Arc::new(Mutex::new(Vec::new())),
            typed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            viewport: Size::new(px(100.0), px(100.0)),
        };
        let scene = gate_scene(ViewKind::View, false, false);
        let agent = if strict {
            Agent::requiring_native_hit_test(scene, Box::new(native))
        } else {
            Agent::new(scene, Box::new(native))
        };
        let error = agent.scroll_into_view(2, "id:target").unwrap_err();
        let expected = if strict {
            "`id:target` is offscreen (it has no native view)"
        } else {
            "`id:target` is not on screen (it has no native view)"
        };
        assert!(error.starts_with(expected), "{error}");
    }
}

#[test]
fn strict_explicit_point_outside_the_visible_target_is_an_error() {
    use nsh_core::{ViewKind, geometry::Size};

    // The clip hides the target's left part (x 20..40).
    let clip_bounds = Bounds::new(point(px(40.0), px(0.0)), size(px(60.0), px(100.0)));
    let target_bounds = Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0)));
    let agent = gate_agent(
        gate_scene(ViewKind::View, false, true),
        Ok(Some(2)),
        &[(1, clip_bounds), (2, target_bounds)],
        Size::new(px(100.0), px(100.0)),
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    );
    let hidden = point(px(25.0), px(30.0));
    let visible_center = point(px(60.0), px(50.0));
    let error = agent
        .explicit_actionable_point(2, 2, hidden, "target")
        .unwrap_err();
    assert!(
        error.contains("`target`") && error.contains("(25, 30)"),
        "{error}"
    );
    assert_eq!(
        agent.actionable_point(2, 2, hidden, "target"),
        Ok(visible_center)
    );
    let delta = point(px(10.0), px(0.0));
    let error = agent.drag_node_by(2, Some(hidden), delta).unwrap_err();
    assert!(
        error.contains("`node 2`") && error.contains("(25, 30)"),
        "{error}"
    );
    agent.drag_node_by(2, None, delta).unwrap();
}

#[test]
fn press_refuses_an_independently_pressable_nested_control() {
    // A targeted press on node 2 must not land on the independently
    // pressable child 4 under the same point: the platform would run
    // 4's action, not 2's.
    let nodes = vec![
        semantic_node(1, None, false, SemanticTarget::View),
        semantic_node(2, Some(1), true, SemanticTarget::View),
        semantic_node(4, Some(2), true, SemanticTarget::View),
    ];
    let presses = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let agent = Agent::requiring_native_hit_test(
        target_scene(nodes),
        Box::new(GateNative {
            windows: None,
            bounds: [
                (
                    2,
                    Bounds::new(point(px(20.0), px(10.0)), size(px(60.0), px(80.0))),
                ),
                (
                    4,
                    Bounds::new(point(px(30.0), px(40.0)), size(px(20.0), px(20.0))),
                ),
            ]
            .into(),
            text_bounds: Default::default(),
            hit: Ok(Some(4)),
            hits: Arc::new(Mutex::new(Vec::new())),
            presses: presses.clone(),
            pressed_at: Arc::new(Mutex::new(Vec::new())),
            long_presses: Arc::new(Mutex::new(Vec::new())),
            typed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            viewport: size(px(100.0), px(100.0)),
        }),
    );

    let error = agent.press_node(2).unwrap_err();
    assert!(error.contains("occluded_by: 4"), "{error}");
    assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 0);

    agent.press_node(4).unwrap();
    assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 1);
}

fn keybinding(id: &str, chord: &str) -> SemanticKeybinding {
    SemanticKeybinding {
        id: id.into(),
        chord: chord.into(),
        label: None,
        menu_path: None,
        menu_action_id: None,
    }
}

fn keybinding_scene(bindings: Vec<SemanticKeybinding>) -> Arc<WithNodes> {
    Arc::new(WithNodes(
        Vec::new(),
        bindings,
        RemoteScene::new(move |step| match step {
            TestStep::Scene {
                query: nsh_dev::protocol::SceneQuery::Activity,
            } => Ok(serde_json::json!({"batches": 1, "since_last_ms": 1000.0})),
            TestStep::Scene {
                query: nsh_dev::protocol::SceneQuery::TakeErrors,
            } => Ok(serde_json::json!([])),
            _ => Err("unexpected scene query".into()),
        }),
    ))
}

#[derive(Default)]
struct KeybindingRecords {
    sent: Mutex<Vec<String>>,
    fired: Mutex<Vec<String>>,
}

/// A strict native that dispatches keybindings by identity only when
/// `identity` is set, like a platform with a real binding registry.
struct KeybindingNative {
    identity: bool,
    records: Arc<KeybindingRecords>,
}

impl Native for KeybindingNative {
    fn bounds(&self, _: NodeId) -> Option<Bounds<Points>> {
        None
    }
    fn viewport(&self) -> Size<Points> {
        size(px(100.0), px(100.0))
    }
    fn press(&self, _: Point<Points>) {}
    fn long_press(&self, _: Point<Points>, _: Duration) {}
    fn scroll(&self, _: Point<Points>, _: Point<Points>) {}
    fn drag(&self, _: Point<Points>, _: Point<Points>) {}
    fn press_keys(&self, keys: &str) -> Result<(), String> {
        self.records.sent.lock().unwrap().push(keys.to_string());
        Ok(())
    }
    fn type_text(&self, _: &str) {}
    fn key(&self, _: Key) {}
    fn input_text(&self, _: NodeId) -> Option<String> {
        None
    }
    fn back(&self) -> Result<(), String> {
        Ok(())
    }
    fn answer_dialog(&self, _: Option<&str>) -> Result<(), String> {
        Ok(())
    }
    fn animating(&self) -> bool {
        false
    }
    fn screenshot(&self) -> Result<Vec<u8>, String> {
        Err("unsupported".into())
    }
    fn resize(&self, _: Points, _: Points) -> Result<(), String> {
        Ok(())
    }
    fn set_dark(&self, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn quit(&self) {}
}

impl StrictNative for KeybindingNative {
    fn fire_keybinding(&self, binding: &SemanticKeybinding) -> Result<(), String> {
        if self.identity {
            self.records.fired.lock().unwrap().push(binding.id.clone());
            Ok(())
        } else {
            Err(NATIVE_KEYBINDING_UNSUPPORTED.into())
        }
    }
}

#[test]
fn fire_keybinding_sends_a_unique_chord_as_keys() {
    let records = Arc::new(KeybindingRecords::default());
    let agent = Agent::new(
        keybinding_scene(vec![keybinding("keymap.quit", "ctrl+q")]),
        Box::new(KeybindingNative {
            identity: false,
            records: records.clone(),
        }),
    );

    agent.fire_keybinding("keymap.quit").unwrap();

    assert_eq!(*records.sent.lock().unwrap(), vec!["ctrl+q".to_string()]);
    assert!(records.fired.lock().unwrap().is_empty());
}

#[test]
fn fire_keybinding_dispatches_a_shared_chord_by_identity() {
    // Two bindings on `ctrl+s`: sending the chord alone could run the
    // other command, so the platform must dispatch by identity.
    let records = Arc::new(KeybindingRecords::default());
    let agent = Agent::new(
        keybinding_scene(vec![
            keybinding("keymap.save", "ctrl+s"),
            keybinding("keymap.save_all", "ctrl+s"),
        ]),
        Box::new(KeybindingNative {
            identity: true,
            records: records.clone(),
        }),
    );

    agent.fire_keybinding("keymap.save_all").unwrap();

    assert_eq!(
        *records.fired.lock().unwrap(),
        vec!["keymap.save_all".to_string()]
    );
    assert!(records.sent.lock().unwrap().is_empty());
}

#[test]
fn fire_keybinding_fails_closed_on_a_shared_chord_without_identity_dispatch() {
    let records = Arc::new(KeybindingRecords::default());
    let agent = Agent::new(
        keybinding_scene(vec![
            keybinding("keymap.save", "ctrl+s"),
            keybinding("keymap.save_all", "ctrl+s"),
        ]),
        Box::new(KeybindingNative {
            identity: false,
            records: records.clone(),
        }),
    );

    assert_eq!(
        agent.fire_keybinding("keymap.save_all").unwrap_err(),
        NATIVE_KEYBINDING_UNSUPPORTED
    );
    assert!(records.sent.lock().unwrap().is_empty());
}

#[test]
fn scroll_node_only_scrolls_a_view_the_screen_marks_scrollable() {
    use nsh_dev::protocol::SceneQuery;

    let pick = Arc::new(Mutex::new(Pick::Disabled));
    let scene_pick = pick.clone();
    let scene = Arc::new(WithNodes(
        vec![
            semantic_node(1, None, false, SemanticTarget::View),
            semantic_node(2, Some(1), false, SemanticTarget::View),
        ],
        Vec::new(),
        RemoteScene::new(move |step| match step {
            TestStep::Scene {
                query: SceneQuery::Activity,
            } => Ok(serde_json::json!({"batches": 1, "since_last_ms": 1000.0})),
            TestStep::Scene {
                query: SceneQuery::TakeErrors,
            } => Ok(serde_json::json!([])),
            TestStep::Scene {
                query: SceneQuery::Pick { selector, target },
            } => Ok(pick_value(
                if selector == "view:2" && matches!(target, PickTarget::ScrollView) {
                    scene_pick.lock().unwrap().clone()
                } else {
                    Pick::None
                },
            )),
            TestStep::Scene {
                query: SceneQuery::ScrollAncestors { .. },
            } => Ok(serde_json::json!([])),
            _ => Err("unexpected scene query".into()),
        }),
    ));
    let agent = Agent::requiring_native_hit_test(
        scene,
        Box::new(GateNative {
            windows: None,
            bounds: [(
                2,
                Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0))),
            )]
            .into(),
            text_bounds: Default::default(),
            hit: Ok(Some(2)),
            hits: Arc::new(Mutex::new(Vec::new())),
            presses: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            pressed_at: Arc::new(Mutex::new(Vec::new())),
            long_presses: Arc::new(Mutex::new(Vec::new())),
            typed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            viewport: size(px(100.0), px(100.0)),
        }),
    );
    let delta = point(px(0.0), px(5.0));

    // A locked scroller would silently scroll an eligible ancestor.
    assert_eq!(
        agent.scroll_node(2, delta).unwrap_err(),
        "`view:2` is a scroll view that does not scroll"
    );
    *pick.lock().unwrap() = Pick::One(3);
    let error = agent.scroll_node(2, delta).unwrap_err();
    assert!(error.contains("`view:2` does not scroll"), "{error}");
    *pick.lock().unwrap() = Pick::One(2);
    agent.scroll_node(2, delta).unwrap();
}

#[test]
fn drag_node_by_takes_an_explicit_origin_when_the_center_is_covered() {
    use nsh_core::{EventMask, MountItem, ViewKind};

    // Node 9 covers node 2's center: the default point is occluded,
    // but a drag from an explicit edge point still works.
    let model = Arc::new(Model::default());
    let mut events = EventMask::default();
    events.set(EventKind::Press, true);
    let target = Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0)));
    let cover = Bounds::new(point(px(40.0), px(40.0)), size(px(20.0), px(20.0)));
    model.apply(&MountBatch {
        items: vec![
            MountItem::Create {
                id: 2,
                kind: ViewKind::View,
            },
            MountItem::Frame {
                id: 2,
                frame: target,
            },
            MountItem::Insert {
                parent: 1,
                child: 2,
                before: None,
            },
            MountItem::Events { id: 2, events },
            MountItem::Create {
                id: 9,
                kind: ViewKind::View,
            },
            MountItem::Frame {
                id: 9,
                frame: cover,
            },
            MountItem::Insert {
                parent: 1,
                child: 9,
                before: None,
            },
        ],
    });
    let windows = Arc::new(WindowRecords {
        nodes: [(1, 1), (2, 1), (9, 1)].into(),
        ..WindowRecords::default()
    });
    *windows.selected.lock().unwrap() = 1;
    let agent = Agent::requiring_native_hit_test(
        model,
        Box::new(GateNative {
            windows: Some(windows.clone()),
            bounds: [(2, target), (9, cover)].into(),
            text_bounds: Default::default(),
            hit: Ok(None),
            hits: Arc::new(Mutex::new(Vec::new())),
            presses: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            pressed_at: Arc::new(Mutex::new(Vec::new())),
            long_presses: Arc::new(Mutex::new(Vec::new())),
            typed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            viewport: size(px(100.0), px(100.0)),
        }),
    );

    assert!(agent.node_point(2).is_err());
    let from = point(px(25.0), px(30.0));
    agent
        .drag_node_by(2, Some(from), point(px(10.0), px(0.0)))
        .unwrap();
    assert_eq!(
        windows.actions.lock().unwrap().as_slice(),
        &[("drag", 1, Some(from))]
    );
}

#[test]
fn press_at_on_a_decorative_hidden_icon_presses_its_ancestor_button() {
    // A decorative accessibility-hidden icon (3) carries no action of
    // its own: the press goes to the button (2) it decorates.
    let mut owner = semantic_node(2, Some(1), true, SemanticTarget::View);
    owner.hidden_descendants = vec![3];
    let nodes = vec![semantic_node(1, None, false, SemanticTarget::View), owner];
    let presses = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let pressed_at = Arc::new(Mutex::new(Vec::new()));
    let agent = Agent::requiring_native_hit_test(
        target_scene(nodes),
        Box::new(GateNative {
            windows: None,
            bounds: [(
                3,
                Bounds::new(point(px(20.0), px(20.0)), size(px(10.0), px(10.0))),
            )]
            .into(),
            text_bounds: Default::default(),
            hit: Ok(Some(3)),
            hits: Arc::new(Mutex::new(Vec::new())),
            presses: presses.clone(),
            pressed_at: pressed_at.clone(),
            long_presses: Arc::new(Mutex::new(Vec::new())),
            typed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            viewport: size(px(100.0), px(100.0)),
        }),
    );
    let at = point(px(25.0), px(25.0));

    agent.press_at(at).unwrap();

    assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(*pressed_at.lock().unwrap(), vec![at]);
}

#[test]
fn press_at_refuses_a_hidden_view_that_takes_its_own_presses() {
    // The hidden view 3 has its own press handler: a semantic press
    // there must not be rerouted to its visible ancestor.
    let mut owner = semantic_node(2, Some(1), true, SemanticTarget::View);
    owner.hidden_descendants = vec![3];
    owner.hidden_press_owners = vec![(3, 3)];
    let nodes = vec![semantic_node(1, None, false, SemanticTarget::View), owner];
    let presses = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let agent = Agent::requiring_native_hit_test(
        target_scene(nodes),
        Box::new(GateNative {
            windows: None,
            bounds: Default::default(),
            text_bounds: Default::default(),
            hit: Ok(Some(3)),
            hits: Arc::new(Mutex::new(Vec::new())),
            presses: presses.clone(),
            pressed_at: Arc::new(Mutex::new(Vec::new())),
            long_presses: Arc::new(Mutex::new(Vec::new())),
            typed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            viewport: size(px(100.0), px(100.0)),
        }),
    );
    let at = point(px(25.0), px(25.0));

    let error = agent.press_at(at).unwrap_err();

    assert!(error.contains("accessibility-hidden"), "{error}");
    assert_eq!(presses.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[test]
fn a_hold_reaches_a_long_press_only_child_inside_a_pressable_parent() {
    use nsh_core::{EventMask, MountItem, ViewKind};

    // The parent takes presses, the child only holds: the hold's hit check
    // must resolve receivers by the long-press rules, not refuse the child
    // as a press receiver mismatch.
    let model = gate_scene(ViewKind::View, false, false);
    let mut events = EventMask::default();
    events.set(EventKind::LongPress, true);
    model.apply(&MountBatch {
        items: vec![
            MountItem::Create {
                id: 3,
                kind: ViewKind::View,
            },
            MountItem::Insert {
                parent: 2,
                child: 3,
                before: None,
            },
            MountItem::Events { id: 3, events },
            MountItem::Frame {
                id: 3,
                frame: Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0))),
            },
        ],
    });
    let frame = Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0)));
    let bounds = [(2, frame), (3, frame)];
    let center = point(px(50.0), px(50.0));
    let long_presses = Arc::new(Mutex::new(Vec::new()));
    let agent = gate_long_press_agent_with(model, Ok(Some(3)), &bounds, &[], long_presses.clone());
    let hold = Duration::from_millis(800);

    agent.long_press_node(3, Some(hold)).unwrap();
    agent.long_press_at(center, Some(hold)).unwrap();

    assert_eq!(
        *long_presses.lock().unwrap(),
        vec![(center, hold), (center, hold)]
    );
}

#[test]
fn a_swipe_default_distance_stays_inside_when_clipping_moves_the_origin() {
    use nsh_core::ViewKind;

    // Clipping moves the drag origin to the visible sliver's middle: the
    // default distance must fit from that origin, not the node's own
    // center (which sits lower and would send the endpoint offscreen).
    let model = gate_scene(ViewKind::View, false, true);
    let bounds = [
        (
            1,
            Bounds::new(point(px(0.0), px(45.0)), size(px(100.0), px(55.0))),
        ),
        (
            2,
            Bounds::new(point(px(20.0), px(0.0)), size(px(60.0), px(100.0))),
        ),
    ];
    let agent = gate_agent(
        model,
        Ok(Some(2)),
        &bounds,
        size(px(100.0), px(100.0)),
        Arc::default(),
        Arc::default(),
        Arc::default(),
    );

    agent.swipe_node(2, SwipeDirection::Down, None).unwrap();
}

#[test]
fn fire_keybinding_dispatches_an_aliased_shared_chord_by_identity() {
    // `control-s` and `ctrl+s` are the same chord: alias spellings must
    // not hide the share and let the chord go out as keys.
    let records = Arc::new(KeybindingRecords::default());
    let agent = Agent::new(
        keybinding_scene(vec![
            keybinding("keymap.save", "control-s"),
            keybinding("keymap.save_all", "ctrl+s"),
        ]),
        Box::new(KeybindingNative {
            identity: true,
            records: records.clone(),
        }),
    );

    agent.fire_keybinding("keymap.save_all").unwrap();

    assert_eq!(
        *records.fired.lock().unwrap(),
        vec!["keymap.save_all".to_string()]
    );
    assert!(records.sent.lock().unwrap().is_empty());
}

#[test]
fn fire_keybinding_dispatches_a_short_spelled_shared_chord_by_identity() {
    // The menu catalogue spells these keys short (its `Key::label()` gives
    // `pgup`, `pgdn`, `ins`): a keymap `ctrl-pageup` and a catalogue
    // `ctrl+pgup` are one chord, so the keymap entry must dispatch by
    // identity or it could fire the menu's action.
    let records = Arc::new(KeybindingRecords::default());
    let agent = Agent::new(
        keybinding_scene(vec![
            keybinding("keymap.prev_tab", "ctrl-pageup"),
            SemanticKeybinding {
                id: "menu.File/#7".into(),
                chord: "ctrl+pgup".into(),
                label: Some("Previous".into()),
                menu_path: Some(vec!["File".into(), "Previous".into()]),
                menu_action_id: Some(7),
            },
        ]),
        Box::new(KeybindingNative {
            identity: true,
            records: records.clone(),
        }),
    );

    agent.fire_keybinding("keymap.prev_tab").unwrap();

    assert_eq!(
        *records.fired.lock().unwrap(),
        vec!["keymap.prev_tab".to_string()]
    );
    assert!(records.sent.lock().unwrap().is_empty());
}

#[test]
fn a_swipe_without_room_to_the_edge_errors_instead_of_overshooting() {
    // An origin a half point inside the left edge cannot host a default
    // swipe: refuse rather than asking for a 1pt move past the edge.
    let model = gate_scene(nsh_core::ViewKind::View, false, false);
    let agent = gate_agent(
        model,
        Ok(Some(2)),
        &[(
            2,
            Bounds::new(point(px(0.5), px(50.0)), size(px(1.0), px(1.0))),
        )],
        size(px(100.0), px(100.0)),
        Arc::default(),
        Arc::default(),
        Arc::default(),
    );

    let error = agent
        .swipe_at(point(px(0.5), px(50.5)), SwipeDirection::Left, None)
        .unwrap_err();

    assert!(error.contains("no room"), "{error}");
}

/// An agent over one pressable button (node 2) at `button`, in a 100x100
/// viewport, with the software keyboard at `keyboard`; every hit is the
/// button. Returns the agent and where its presses landed.
fn keyboard_agent(
    button: Bounds<Points>,
    keyboard: Option<Bounds<Points>>,
) -> (Agent, Arc<Mutex<Vec<Point<Points>>>>) {
    GATE_KEYBOARD.with(|frame| frame.set(keyboard));
    let pressed_at = Arc::new(Mutex::new(Vec::new()));
    let agent = Agent::requiring_native_hit_test(
        target_scene(vec![
            semantic_node(1, None, false, SemanticTarget::View),
            semantic_node(2, Some(1), true, SemanticTarget::View),
        ]),
        Box::new(GateNative {
            windows: None,
            bounds: [(2, button)].into(),
            text_bounds: Default::default(),
            hit: Ok(Some(2)),
            hits: Arc::new(Mutex::new(Vec::new())),
            presses: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            pressed_at: pressed_at.clone(),
            long_presses: Arc::new(Mutex::new(Vec::new())),
            typed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            viewport: size(px(100.0), px(100.0)),
        }),
    );
    (agent, pressed_at)
}

#[test]
fn a_press_clear_of_the_keyboard_lands_in_the_middle() {
    let button = Bounds::new(point(px(0.0), px(40.0)), size(px(80.0), px(40.0)));
    let keyboard = Bounds::new(point(px(0.0), px(85.0)), size(px(100.0), px(15.0)));
    for frame in [None, Some(keyboard)] {
        let (agent, pressed_at) = keyboard_agent(button, frame);
        agent.press_node(2).unwrap();
        assert_eq!(
            *pressed_at.lock().unwrap(),
            vec![button.center()],
            "{frame:?}"
        );
    }
}

#[test]
fn a_press_on_a_target_half_under_the_keyboard_lands_on_the_part_above_it() {
    // The button is y 40..80, the keyboard starts at y 50: the middle
    // (y 60) is on a key, the showing part is y 40..50.
    let button = Bounds::new(point(px(0.0), px(40.0)), size(px(80.0), px(40.0)));
    let keyboard = Bounds::new(point(px(0.0), px(50.0)), size(px(100.0), px(50.0)));
    let (agent, pressed_at) = keyboard_agent(button, Some(keyboard));
    agent.press_node(2).unwrap();
    assert_eq!(*pressed_at.lock().unwrap(), vec![point(px(40.0), px(45.0))]);
}

#[test]
fn a_press_on_a_target_under_the_keyboard_is_refused() {
    let button = Bounds::new(point(px(0.0), px(80.0)), size(px(80.0), px(20.0)));
    let keyboard = Bounds::new(point(px(0.0), px(70.0)), size(px(100.0), px(30.0)));
    let (agent, pressed_at) = keyboard_agent(button, Some(keyboard));
    let error = agent.press_node(2).unwrap_err();
    assert!(
        error.starts_with("occluded_by: software keyboard"),
        "{error}"
    );
    assert!(pressed_at.lock().unwrap().is_empty(), "nothing was pressed");
}

#[test]
fn a_press_at_a_point_on_the_keyboard_is_refused_and_not_moved() {
    let button = Bounds::new(point(px(0.0), px(40.0)), size(px(80.0), px(40.0)));
    let keyboard = Bounds::new(point(px(0.0), px(50.0)), size(px(100.0), px(50.0)));
    let (agent, pressed_at) = keyboard_agent(button, Some(keyboard));
    let error = agent.press_at(point(px(40.0), px(70.0))).unwrap_err();
    assert!(
        error.starts_with("occluded_by: software keyboard"),
        "{error}"
    );
    assert!(pressed_at.lock().unwrap().is_empty(), "nothing was pressed");
    // Above the keyboard the same button takes the press where it was asked.
    agent.press_at(point(px(40.0), px(45.0))).unwrap();
    assert_eq!(*pressed_at.lock().unwrap(), vec![point(px(40.0), px(45.0))]);
    GATE_KEYBOARD.with(|frame| frame.set(None));
}

/// [`keyboard_agent`] whose node 2 is a text input, focused when `focused`.
fn keyboard_input_agent(
    input: Bounds<Points>,
    keyboard: Option<Bounds<Points>>,
    focused: bool,
) -> (
    Agent,
    Arc<Mutex<Vec<Point<Points>>>>,
    Arc<std::sync::atomic::AtomicUsize>,
) {
    GATE_KEYBOARD.with(|frame| frame.set(keyboard));
    GATE_FOCUSED.with(|id| id.set(focused.then_some(2)));
    let pressed_at = Arc::new(Mutex::new(Vec::new()));
    let typed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut node = semantic_node(2, Some(1), false, SemanticTarget::View);
    node.input = true;
    let agent = Agent::requiring_native_hit_test(
        target_scene(vec![
            semantic_node(1, None, false, SemanticTarget::View),
            node,
        ]),
        Box::new(GateNative {
            windows: None,
            bounds: [(2, input)].into(),
            text_bounds: Default::default(),
            hit: Ok(Some(2)),
            hits: Arc::new(Mutex::new(Vec::new())),
            presses: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            pressed_at: pressed_at.clone(),
            long_presses: Arc::new(Mutex::new(Vec::new())),
            typed: typed.clone(),
            viewport: size(px(100.0), px(100.0)),
        }),
    );
    (agent, pressed_at, typed)
}

#[test]
fn a_focused_input_under_the_keyboard_takes_typing_without_a_press() {
    let input = Bounds::new(point(px(0.0), px(80.0)), size(px(80.0), px(20.0)));
    let keyboard = Bounds::new(point(px(0.0), px(70.0)), size(px(100.0), px(30.0)));
    let (agent, pressed_at, typed) = keyboard_input_agent(input, Some(keyboard), true);
    agent.type_node(2, "12").unwrap();
    assert!(
        pressed_at.lock().unwrap().is_empty(),
        "no press on the keyboard"
    );
    assert_eq!(typed.load(std::sync::atomic::Ordering::Relaxed), 1);

    // The same input without the focus needs a press, and the keyboard is
    // where it would land.
    let (agent, pressed_at, typed) = keyboard_input_agent(input, Some(keyboard), false);
    let error = agent.type_node(2, "12").unwrap_err();
    assert!(
        error.starts_with("occluded_by: software keyboard"),
        "{error}"
    );
    assert!(pressed_at.lock().unwrap().is_empty());
    assert_eq!(typed.load(std::sync::atomic::Ordering::Relaxed), 0);
    GATE_KEYBOARD.with(|frame| frame.set(None));
    GATE_FOCUSED.with(|id| id.set(None));
}

#[test]
fn a_sliver_above_the_keyboard_is_not_pressed() {
    // The button is y 40..80 and the keyboard starts at y 42: 2 points
    // show, less than a finger needs.
    let button = Bounds::new(point(px(0.0), px(40.0)), size(px(80.0), px(40.0)));
    let keyboard = Bounds::new(point(px(0.0), px(42.0)), size(px(100.0), px(58.0)));
    let (agent, pressed_at) = keyboard_agent(button, Some(keyboard));
    let error = agent.press_node(2).unwrap_err();
    assert!(
        error.starts_with("occluded_by: software keyboard"),
        "{error}"
    );
    assert!(pressed_at.lock().unwrap().is_empty());
    GATE_KEYBOARD.with(|frame| frame.set(None));
}

#[test]
fn a_keyboard_frame_counts_once_it_has_stayed_put() {
    let start = Instant::now();
    let shown = Some(Bounds::new(
        point(px(0.0), px(50.0)),
        size(px(100.0), px(50.0)),
    ));
    let mut track = KeyboardTrack::default();
    // The frame jumps to its end as the slide starts: not yet counted.
    assert_eq!(track.observe(shown, start), Some(start + KEYBOARD_SETTLE));
    assert_eq!(track.settled, None);
    assert!(track.observe(shown, start + KEYBOARD_SETTLE / 2).is_some());
    assert_eq!(track.observe(shown, start + KEYBOARD_SETTLE), None);
    assert_eq!(track.settled, shown);
    // Hiding: the old frame still counts until the new one has settled.
    let hide = start + KEYBOARD_SETTLE * 2;
    assert!(track.observe(None, hide).is_some());
    assert_eq!(track.settled, shown);
    assert_eq!(track.observe(None, hide + KEYBOARD_SETTLE), None);
    assert_eq!(track.settled, None);
}

/// An OTP-like component: group 2 holds input 3 and cell 4 over it, and
/// node 5 sits beside the group; every hit is `hit`, and a press gives the
/// focus to `focus_on_press`.
fn overlay_agent(
    hit: NodeId,
    focus_on_press: Option<NodeId>,
) -> (
    Agent,
    Arc<Mutex<Vec<Point<Points>>>>,
    Arc<std::sync::atomic::AtomicUsize>,
) {
    GATE_KEYBOARD.with(|frame| frame.set(None));
    GATE_FOCUSED.with(|id| id.set(None));
    GATE_FOCUS_ON_PRESS.with(|id| id.set(focus_on_press));
    let field = Bounds::new(point(px(10.0), px(10.0)), size(px(60.0), px(20.0)));
    let pressed_at = Arc::new(Mutex::new(Vec::new()));
    let typed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut input = semantic_node(3, Some(2), false, SemanticTarget::View);
    input.input = true;
    let agent = Agent::requiring_native_hit_test(
        target_scene(vec![
            semantic_node(1, None, false, SemanticTarget::View),
            semantic_node(2, Some(1), false, SemanticTarget::View),
            input,
            semantic_node(4, Some(2), false, SemanticTarget::View),
            semantic_node(5, Some(1), false, SemanticTarget::View),
        ]),
        Box::new(GateNative {
            windows: None,
            bounds: [(2, field), (3, field), (4, field), (5, field)].into(),
            text_bounds: Default::default(),
            hit: Ok(Some(hit)),
            hits: Arc::new(Mutex::new(Vec::new())),
            presses: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            pressed_at: pressed_at.clone(),
            long_presses: Arc::new(Mutex::new(Vec::new())),
            typed: typed.clone(),
            viewport: size(px(100.0), px(100.0)),
        }),
    );
    (agent, pressed_at, typed)
}

#[test]
fn an_input_under_its_own_overlay_is_focused_through_the_overlay() {
    let (agent, pressed_at, typed) = overlay_agent(4, Some(3));
    agent.type_node(3, "123456").unwrap();
    assert_eq!(*pressed_at.lock().unwrap(), vec![point(px(40.0), px(20.0))]);
    assert_eq!(typed.load(std::sync::atomic::Ordering::Relaxed), 1);
    GATE_FOCUS_ON_PRESS.with(|id| id.set(None));
    GATE_FOCUSED.with(|id| id.set(None));
}

#[test]
fn an_input_under_another_view_is_not_pressed() {
    let (agent, pressed_at, typed) = overlay_agent(5, Some(3));
    let error = agent.type_node(3, "1").unwrap_err();
    assert!(error.starts_with("occluded_by: 5"), "{error}");
    assert!(pressed_at.lock().unwrap().is_empty());
    assert_eq!(typed.load(std::sync::atomic::Ordering::Relaxed), 0);
    GATE_FOCUS_ON_PRESS.with(|id| id.set(None));
}

#[test]
fn an_overlay_press_that_does_not_focus_the_input_fails_and_types_nothing() {
    let (agent, _, typed) = overlay_agent(4, None);
    let error = agent.type_node(3, "1").unwrap_err();
    assert!(error.contains("did not give node 3 the focus"), "{error}");
    assert_eq!(typed.load(std::sync::atomic::Ordering::Relaxed), 0);
}
