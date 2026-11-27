//! Adapter from the shared Native Shell test agent to the GPUI-agent protocol.

#[cfg(all(target_os = "ios", feature = "ios"))]
pub mod ios;

#[cfg(feature = "dev-agent")]
use std::sync::Mutex;
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub use gpui_agent::PlatformKind;
use gpui_agent::{
    AgentHost, DispatchResult, HelloAuth, HelloInfo, KeybindingInfo, KeybindingScope, Op,
    PROTOCOL_VERSION, ScreenshotMode, UiNode, UiTree, role, scroll_capture::ScreenshotSpec,
};
use nsh_agent_actions::{Agent, SemanticKeybinding, SemanticNode};
use nsh_core::{
    NodeId,
    geometry::{Point, Points},
};

/// The id of the node a snapshot ends with while the software keyboard
/// shows: role `keyboard`, its bounds the keyboard's frame. Absent while
/// the keyboard is hidden, so `wait_until system-keyboard` waits for the
/// keyboard and `wait_until system-keyboard --absent` for it to go.
pub const SOFTWARE_KEYBOARD_ID: &str = "system-keyboard";

/// The id of the node a snapshot ends with while the platform's own text
/// menu shows over an input (Android's floating toolbar): role `menu`, its
/// kind a state (`selection`, `insertion`, `edit`), its visible items
/// children of role `menuitem`, all `system` and taking no action. Absent
/// while no such menu shows.
pub const TEXT_MENU_ID: &str = "system-text-menu";

/// The `hello.features` entry of a host that reports the software keyboard
/// ([`SOFTWARE_KEYBOARD_ID`]) and refuses presses on it.
pub const SOFTWARE_KEYBOARD_FEATURE: &str = "software_keyboard";

/// Native Shell host exposed through GPUI-agent.
#[non_exhaustive]
pub struct NshAgentHost {
    app: String,
    platform: PlatformKind,
    agent: Arc<Agent>,
    token_required: bool,
    screenshot_directory: Option<PathBuf>,
    tab_mode: bool,
}

impl NshAgentHost {
    /// Creates a host. The shared `Agent` must use native-hit-test enforcement.
    pub fn new(
        app: impl Into<String>,
        platform: PlatformKind,
        agent: Arc<Agent>,
    ) -> Result<Self, String> {
        if !agent.requires_native_hit_test() {
            return Err("GPUI-agent requires native hit-test enforcement".into());
        }
        Ok(Self {
            app: app.into(),
            platform,
            agent,
            token_required: false,
            screenshot_directory: None,
            tab_mode: false,
        })
    }

    /// Advertises only the operations supported by a user-opened web tab.
    pub fn with_tab_mode(mut self) -> Self {
        self.tab_mode = true;
        self
    }

    /// Confines screenshot output to a host-owned writable directory.
    pub fn with_screenshot_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.screenshot_directory = Some(directory.into());
        self
    }

    fn node_id(&self, target: &str) -> Result<NodeId, String> {
        self.agent.settle()?;
        let nodes = self.agent.semantic_nodes()?;
        let ids = stable_ids(&nodes);
        ids.iter()
            .find(|(_, id)| id.as_str() == target)
            .map(|(node, _)| *node)
            .ok_or_else(|| format!("target_gone: {target}"))
    }

    fn require_available(&self) -> Result<(), String> {
        if self.agent.native_available() {
            Ok(())
        } else if self.tab_mode {
            Err("no tab has opened the agent's page yet".into())
        } else {
            Err("attached native surface is unavailable".into())
        }
    }

    fn dispatch_available(&mut self, op: &Op) -> Result<DispatchResult, String> {
        match op {
            Op::Click { target, .. } => self
                .agent
                .press_node(self.node_id(target)?)
                .map(|()| DispatchResult::empty()),
            Op::Type { target, text, .. } => self
                .agent
                .type_node(self.node_id(target)?, text)
                .map(|()| DispatchResult::empty()),
            Op::SetValue { target, value, .. } => self
                .agent
                .set_value_node(self.node_id(target)?, value)
                .map(|()| DispatchResult::empty()),
            Op::Shutdown => Err(
                "shutdown is unsupported for an attached app; stop the host process instead".into(),
            ),
            Op::Key { target, key, .. } => self
                .agent
                .key_node(self.node_id(target)?, key)
                .map(|()| DispatchResult::empty()),
            Op::Invoke { name, args } => self.invoke(name, args),
            Op::Press { .. }
            | Op::Scroll { .. }
            | Op::Hover { .. }
            | Op::Drag { .. }
            | Op::Viewport { .. } => {
                // The dispatcher has already checked a press's fingerprint;
                // the remaining fields are the `invoke` arguments.
                let op = match op {
                    Op::Press { target, x, y, .. } => Op::Press {
                        target: target.clone(),
                        x: *x,
                        y: *y,
                        expect_fingerprint: None,
                    },
                    other => other.clone(),
                };
                match op.legacy_invoke() {
                    Some(Op::Invoke { name, args }) => self.invoke(&name, &args),
                    _ => Err(gpui_agent::unsupported_op("this operation")),
                }
            }
            Op::LongPress {
                target,
                x,
                y,
                duration_ms,
            } => {
                let hold = duration_ms.map(|ms| std::time::Duration::from_millis(ms.min(10_000)));
                match (target, x, y) {
                    (Some(target), _, _) => {
                        self.agent.long_press_node(self.node_id(target)?, hold)?
                    }
                    (None, Some(x), Some(y)) => self
                        .agent
                        .long_press_at(Point::new(Points(*x as f32), Points(*y as f32)), hold)?,
                    _ => return Err("long_press needs target or x and y".into()),
                }
                Ok(DispatchResult::empty())
            }
            Op::Paste { target, text } => self
                .agent
                .paste_node(self.node_id(target)?, text)
                .map(|()| DispatchResult::empty()),
            Op::System {
                action: gpui_agent::SystemAction::Back,
                ..
            } => self.agent.back().map(|()| DispatchResult::empty()),
            Op::System {
                action: gpui_agent::SystemAction::Rotate,
                value,
            } => {
                let landscape = match value.as_deref() {
                    Some("landscape") => true,
                    Some("portrait") => false,
                    _ => {
                        return Err("system rotate value must be `portrait` or `landscape`".into());
                    }
                };
                self.agent
                    .rotate(landscape)
                    .map(|()| DispatchResult::empty())
            }
            Op::System {
                action: gpui_agent::SystemAction::KeyboardShow,
                ..
            } => self.agent.keyboard(true).map(|()| DispatchResult::empty()),
            Op::System {
                action: gpui_agent::SystemAction::KeyboardHide,
                ..
            } => self.agent.keyboard(false).map(|()| DispatchResult::empty()),
            Op::Swipe {
                target,
                direction,
                distance,
            } => self.swipe(target.as_deref(), *direction, *distance),
            Op::Keybinding { .. } => {
                let bindings = self.keybindings();
                let entry =
                    gpui_agent::authorize_keybinding_op(op, &bindings, self.is_app_focused())?;
                self.agent
                    .fire_keybinding(&entry.id)
                    .map(|()| DispatchResult::empty())
            }
            _ => Err("operation is handled by the GPUI-agent dispatcher".into()),
        }
    }
}

impl AgentHost for NshAgentHost {
    fn hello(&self) -> HelloInfo {
        HelloInfo {
            protocol: PROTOCOL_VERSION,
            app: self.app.clone(),
            platform: self.platform,
            os: std::env::consts::OS.into(),
            ready: self.agent.native_available()
                && self.agent.has_rendered_batch().unwrap_or(false)
                && self.agent.semantic_nodes().is_ok()
                && self.agent.native_available()
                && self.agent.native_laid_out().is_ok(),
            deliveries: vec![gpui_agent::DeliveryMode::Semantic],
            auth: if self.token_required {
                HelloAuth::Required
            } else {
                HelloAuth::None
            },
            bundle_id: None,
            executable: None,
            features: self.features(),
            framework: Some("nsh".into()),
            device: Some(self.device()),
        }
    }

    fn snapshot(&self) -> UiTree {
        let _ = self.agent.take_native_failure();
        if !self.agent.native_available()
            || !matches!(self.agent.has_rendered_batch(), Ok(true))
            || self.agent.native_laid_out().is_err()
        {
            return UiTree {
                app: self.app.clone(),
                platform: self.platform,
                ready: false,
                nodes: Vec::new(),
                rev: None,
            };
        }
        // The keyboard first: once it has settled, the tree is laid out for it.
        let keyboard = self.agent.keyboard_frame();
        let Ok(nodes) = self.agent.semantic_nodes() else {
            return UiTree {
                app: self.app.clone(),
                platform: self.platform,
                ready: false,
                nodes: Vec::new(),
                rev: None,
            };
        };
        let index = SnapshotIndex::new(&nodes, self.agent.as_ref());
        let mut roots: Vec<UiNode> = nodes
            .iter()
            .filter(|node| {
                node.parent
                    .is_none_or(|parent| !index.known.contains_key(&parent))
            })
            .map(|node| ui_node(node, &index, self.agent.as_ref()))
            .collect();
        if let Ok(Some(frame)) = keyboard {
            roots.push(keyboard_node(frame));
        }
        if let Some(menu) = self.agent.native_text_menu() {
            roots.push(text_menu_node(&menu));
        }
        // A resize or a failed call while the nodes were read.
        if !self.agent.native_available()
            || self.agent.native_laid_out().is_err()
            || self.agent.take_native_failure().is_some()
        {
            return UiTree {
                app: self.app.clone(),
                platform: self.platform,
                ready: false,
                nodes: Vec::new(),
                rev: None,
            };
        }
        UiTree {
            app: self.app.clone(),
            platform: self.platform,
            ready: true,
            nodes: roots,
            rev: None,
        }
    }

    fn dispatch(&mut self, op: &Op) -> Result<DispatchResult, String> {
        self.require_available()?;
        let _ = self.agent.take_native_failure();
        let result = self.dispatch_available(op);
        if let Some(failure) = self.agent.take_native_failure() {
            return Err(failure);
        }
        if self.agent.native_available() {
            result
        } else {
            Err("attached native surface is unavailable".into())
        }
    }

    fn screenshot(&self, spec: ScreenshotSpec<'_>) -> Result<DispatchResult, String> {
        self.require_available()?;
        spec.validate_request()?;
        if self.tab_mode {
            return Err(gpui_agent::screenshot_unavailable(
                "a tab nsh does not control cannot be captured",
            ));
        }
        if spec.mode != ScreenshotMode::Viewport {
            return Err(gpui_agent::screenshot_unavailable(
                "scrolled capture is unsupported",
            ));
        }
        let path = gpui_agent::require_screenshot_path(spec.path)?;
        let png = self.agent.native_screenshot()?;
        self.require_available()?;
        match &self.screenshot_directory {
            Some(directory) => gpui_agent::write_png_in(path, &png, directory),
            None => gpui_agent::write_png(path, &png),
        }
        .map(DispatchResult::json)
    }

    fn revision(&self) -> Option<u64> {
        None
    }

    fn keybindings(&self) -> Vec<KeybindingInfo> {
        if !self.agent.native_available() {
            return Vec::new();
        }
        self.agent
            .semantic_keybindings()
            .unwrap_or_default()
            .iter()
            .map(keybinding_info)
            .collect()
    }

    fn is_app_focused(&self) -> bool {
        self.agent.app_focused()
    }
}

impl NshAgentHost {
    /// The feature-gated operations this platform's agent delivers. A
    /// phone has no pointer to hover and no window to resize; a swipe is a
    /// touch gesture.
    fn features(&self) -> Vec<String> {
        let mut names: Vec<&str> = if self.tab_mode {
            vec![
                "press",
                "scroll",
                "hover",
                "drag",
                "long_press",
                "system.back",
            ]
        } else {
            match self.platform {
                PlatformKind::Mobile => vec![
                    "press",
                    "scroll",
                    "drag",
                    "swipe",
                    "long_press",
                    "system.back",
                ],
                PlatformKind::Web => vec![
                    "press",
                    "scroll",
                    "hover",
                    "drag",
                    "long_press",
                    "paste",
                    "viewport",
                ],
                _ => vec!["press", "scroll", "hover", "drag", "viewport"],
            }
        };
        if !self.tab_mode
            && matches!(self.platform, PlatformKind::Mobile)
            && self.agent.keyboard_known()
        {
            names.push(SOFTWARE_KEYBOARD_FEATURE);
        }
        if !self.tab_mode && matches!(self.platform, PlatformKind::Mobile) {
            // Every mobile driver supports paste; the OS gates below are
            // about the driver's own platform — the iOS driver runs on the
            // macOS host, so cfg!(ios) is never true here.
            names.push("paste");
            if cfg!(target_os = "android") {
                names.extend([
                    "system.rotate",
                    "system.keyboard_show",
                    "system.keyboard_hide",
                ]);
            }
        }
        names.iter().map(|name| (*name).to_string()).collect()
    }

    fn device(&self) -> gpui_agent::DeviceInfo {
        let viewport = self.agent.native_viewport();
        let (width, height) = (f64::from(viewport.width.0), f64::from(viewport.height.0));
        let known = width > 0.0 && height > 0.0;
        let form_factor = match self.platform {
            PlatformKind::Mobile => "phone",
            PlatformKind::Web => "browser",
            _ => "desktop",
        };
        let orientation = if width > height {
            "landscape"
        } else {
            "portrait"
        };
        let input_source = if self.tab_mode {
            "synthetic"
        } else {
            "trusted"
        };
        gpui_agent::DeviceInfo {
            form_factor: Some(form_factor.into()),
            width: known.then_some(width),
            height: known.then_some(height),
            orientation: known.then(|| orientation.into()),
            input: Some(input_source.into()),
            ..Default::default()
        }
    }

    /// A swipe is a touch drag: across the node, or from the centre of the
    /// surface when no node is named.
    fn swipe(
        &self,
        target: Option<&str>,
        direction: gpui_agent::SwipeDirection,
        distance: Option<f64>,
    ) -> Result<DispatchResult, String> {
        let explicit = match distance {
            Some(distance) if distance.is_finite() && distance > 0.0 => Some(distance as f32),
            Some(_) => return Err("swipe distance must be finite and positive".into()),
            None => None,
        };
        let direction = match direction {
            gpui_agent::SwipeDirection::Up => nsh_agent_actions::SwipeDirection::Up,
            gpui_agent::SwipeDirection::Down => nsh_agent_actions::SwipeDirection::Down,
            gpui_agent::SwipeDirection::Left => nsh_agent_actions::SwipeDirection::Left,
            gpui_agent::SwipeDirection::Right => nsh_agent_actions::SwipeDirection::Right,
        };
        match target {
            Some(target) => {
                let id = self.node_id(target)?;
                self.agent.swipe_node(id, direction, explicit)?;
            }
            // The default distance is picked inside the agent, from the same
            // checked origin and viewport the drag itself uses.
            None => {
                let viewport = self.agent.native_viewport();
                let (width, height) = (viewport.width.0, viewport.height.0);
                self.agent.swipe_at(
                    Point::new(Points(width / 2.0), Points(height / 2.0)),
                    direction,
                    explicit,
                )?;
            }
        }
        Ok(DispatchResult::empty())
    }

    fn invoke(&self, name: &str, args: &serde_json::Value) -> Result<DispatchResult, String> {
        let target = |name: &str| {
            args.get(name)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| format!("invoke `{name}` requires a string target"))
                .and_then(|target| self.node_id(target))
        };
        match name {
            "press" => {
                if args.get("target").is_some() {
                    self.agent.press_node(target("target")?)?;
                } else {
                    self.agent.press_at(
                        optional_point(args)?.ok_or("invoke `press` needs x and y or target")?,
                    )?;
                }
                Ok(DispatchResult::empty())
            }
            "scroll" => {
                let dx = number_arg(args, "dx")?;
                let dy = number_arg(args, "dy")?;
                if number_arg(args, "momentum")? != 0.0 {
                    return Err("scroll momentum is unsupported".into());
                }
                let delta = Point::new(Points(dx), Points(dy));
                if args.get("target").is_some() {
                    let id = target("target")?;
                    if !self
                        .agent
                        .semantic_nodes()?
                        .iter()
                        .any(|node| node.id == id && node.scroll)
                    {
                        return Err(format!("node {id} is not scrollable"));
                    }
                    self.agent.scroll_node(id, delta)?;
                } else {
                    self.agent.scroll_at(
                        optional_point(args)?.ok_or("invoke `scroll` needs x and y or target")?,
                        delta,
                    )?;
                }
                Ok(DispatchResult::empty())
            }
            "hover" => {
                if args.get("target").is_some() {
                    self.agent.hover_node(target("target")?)?;
                } else {
                    self.agent.hover_at(
                        optional_point(args)?.ok_or("invoke `hover` needs x and y or target")?,
                    )?;
                }
                Ok(DispatchResult::empty())
            }
            "drag" => {
                if args.get("from").is_some() || args.get("to").is_some() {
                    self.agent.drag_nodes(target("from")?, target("to")?)?;
                } else if args.get("target").is_some() {
                    let id = target("target")?;
                    let dx = number_arg(args, "dx")?;
                    let dy = number_arg(args, "dy")?;
                    self.agent.drag_node_by(
                        id,
                        optional_point(args)?,
                        Point::new(Points(dx), Points(dy)),
                    )?;
                } else {
                    let origin =
                        optional_point(args)?.ok_or("invoke `drag` needs x and y or target")?;
                    let dx = number_arg(args, "dx")?;
                    let dy = number_arg(args, "dy")?;
                    self.agent.drag_at(
                        origin,
                        Point::new(Points(origin.x.0 + dx), Points(origin.y.0 + dy)),
                    )?;
                }
                Ok(DispatchResult::empty())
            }
            "viewport" => {
                let (width, height) = viewport_size(args)?;
                self.agent.resize_viewport(Points(width), Points(height))?;
                Ok(DispatchResult::json(serde_json::json!({
                    "width": self.agent.native_viewport().width.0,
                    "height": self.agent.native_viewport().height.0,
                })))
            }
            _ => Err(format!("invoke `{name}` is unsupported")),
        }
    }
}

fn viewport_size(args: &serde_json::Value) -> Result<(f32, f32), String> {
    let preset = args.get("preset").and_then(serde_json::Value::as_str);
    let size = match preset {
        Some("phone") => Some((390.0, 844.0)),
        Some("tablet") => Some((820.0, 1180.0)),
        Some("laptop") => Some((1280.0, 800.0)),
        Some("desktop") => Some((1440.0, 900.0)),
        Some(other) => return Err(format!("unknown viewport preset `{other}`")),
        None => None,
    };
    let (width, height) = size.unwrap_or_else(|| {
        (
            args.get("width")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or_default() as f32,
            args.get("height")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or_default() as f32,
        )
    });
    if !width.is_finite() || width <= 0.0 || !height.is_finite() || height <= 0.0 {
        return Err("viewport width and height must be finite and positive".into());
    }
    Ok((width, height))
}

fn number_arg(args: &serde_json::Value, name: &str) -> Result<f32, String> {
    let Some(value) = args.get(name) else {
        return Ok(0.0);
    };
    let number = value
        .as_f64()
        .ok_or_else(|| format!("invoke `{name}` must be a finite number"))?;
    let number = number as f32;
    if !number.is_finite() {
        return Err(format!("invoke `{name}` must be a finite number"));
    }
    Ok(number)
}

fn optional_point(args: &serde_json::Value) -> Result<Option<Point<Points>>, String> {
    match (args.get("x"), args.get("y")) {
        (None, None) => Ok(None),
        (Some(_), Some(_)) => Ok(Some(Point::new(
            Points(number_arg(args, "x")?),
            Points(number_arg(args, "y")?),
        ))),
        _ => Err("invoke coordinates require both x and y".into()),
    }
}

fn stable_ids(nodes: &[SemanticNode]) -> HashMap<NodeId, String> {
    let bases: Vec<String> = nodes
        .iter()
        .map(|node| {
            node.test_id
                .as_deref()
                .filter(|test_id| !test_id.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("n{}", node.id))
        })
        .collect();
    let mut counts = HashMap::new();
    for base in &bases {
        *counts.entry(base.clone()).or_insert(0usize) += 1;
    }
    // The `base#id` form of every node: a base equal to another node's
    // gets the form too. (Never a node's own: `base#id` is longer than
    // `base`, so the other node's base differs as well.)
    let generated: HashSet<String> = nodes
        .iter()
        .zip(&bases)
        .map(|(node, base)| format!("{base}#{}", node.id))
        .collect();
    let mut used = HashSet::new();
    nodes
        .iter()
        .zip(bases.iter().cloned())
        .map(|(node, base)| {
            let generated_by_other = generated.contains(&base);
            let mut id = if counts[&base] == 1 && !generated_by_other {
                base.clone()
            } else {
                format!("{base}#{}", node.id)
            };
            if !used.insert(id.clone()) {
                id = format!("{id}#{}", node.id);
                let mut suffix = 2;
                while !used.insert(id.clone()) {
                    id = format!("{id}#{suffix}");
                    suffix += 1;
                }
            }
            (node.id, id)
        })
        .collect()
}

/// One semantic tree's lookups for a snapshot, built once: nodes,
/// children and native bounds by id, so that the snapshot neither asks the
/// scene for the tree again nor searches it per node.
struct SnapshotIndex<'a> {
    known: HashMap<NodeId, &'a SemanticNode>,
    children: HashMap<NodeId, Vec<&'a SemanticNode>>,
    bounds: HashMap<NodeId, nsh_core::geometry::Bounds<Points>>,
    ids: HashMap<NodeId, String>,
    viewport: nsh_core::geometry::Size<Points>,
}

impl<'a> SnapshotIndex<'a> {
    fn new(nodes: &'a [SemanticNode], agent: &Agent) -> Self {
        let mut children: HashMap<NodeId, Vec<&SemanticNode>> = HashMap::new();
        for node in nodes {
            if let Some(parent) = node.parent {
                children.entry(parent).or_default().push(node);
            }
        }
        Self {
            known: nodes.iter().map(|node| (node.id, node)).collect(),
            children,
            bounds: nodes
                .iter()
                .filter_map(|node| Some((node.id, agent.node_native_bounds(node)?)))
                .collect(),
            ids: stable_ids(nodes),
            viewport: agent.native_viewport(),
        }
    }
}

/// The snapshot's node for the platform's text menu ([`TEXT_MENU_ID`]):
/// not one of the app's views, so it and its items take no action. Its
/// bounds stay zero: the platform does not say where it floats.
fn text_menu_node(menu: &nsh_agent_actions::TextMenu) -> UiNode {
    let mut node = UiNode::new(TEXT_MENU_ID, "menu", "Text menu");
    node.enabled = false;
    node.states
        .extend(["system".to_owned(), menu.kind.to_owned()]);
    node.children = menu
        .items
        .iter()
        .enumerate()
        .map(|(n, title)| {
            let mut item = UiNode::new(format!("{TEXT_MENU_ID}-{}", n + 1), "menuitem", title);
            item.enabled = false;
            item.states.push("system".into());
            item
        })
        .collect();
    node
}

/// The snapshot's node for the software keyboard showing at `frame`: not
/// one of the app's views, so it takes no action (`enabled: false`).
fn keyboard_node(frame: nsh_core::geometry::Bounds<Points>) -> UiNode {
    let mut node = UiNode::new(SOFTWARE_KEYBOARD_ID, "keyboard", "Software keyboard");
    node.enabled = false;
    node.bounds = gpui_agent::Bounds {
        x: frame.origin.x.0,
        y: frame.origin.y.0,
        w: frame.size.width.0,
        h: frame.size.height.0,
    };
    node.states.push("system".into());
    node
}

fn ui_node(node: &SemanticNode, index: &SnapshotIndex<'_>, agent: &Agent) -> UiNode {
    let mut name = node
        .label
        .clone()
        .or_else(|| node.value.clone())
        .or_else(|| node.test_id.clone())
        .unwrap_or_default();
    if node.grouped {
        name.push_str(" [grouped]");
    }
    if let Some(hint) = &node.hint {
        if !name.is_empty() {
            name.push_str(" — ");
        }
        name.push_str("hint: ");
        name.push_str(hint);
    }
    let role = match node.role.as_str() {
        "button" => role::BUTTON,
        "textbox" => role::TEXTBOX,
        "scroll" => role::SCROLL,
        "text" => role::NOTE,
        _ => node.role.as_str(),
    };
    let mut result = UiNode::new(
        index
            .ids
            .get(&node.id)
            .cloned()
            .unwrap_or_else(|| format!("n{}", node.id)),
        role,
        name,
    );
    result.enabled = node.enabled;
    // The scene already masks a masked input's value; the native text is
    // masked the same way: a bullet per character, never the text.
    let masked = node.states.iter().any(|state| state == "masked");
    result.value = agent
        .native_input_text(node.id)
        .map(|text| {
            if masked {
                "\u{2022}".repeat(text.chars().count())
            } else {
                text
            }
        })
        .or_else(|| node.value.clone());
    if let Some(bounds) = index.bounds.get(&node.id) {
        result.bounds = gpui_agent::Bounds {
            x: bounds.origin.x.0,
            y: bounds.origin.y.0,
            w: bounds.size.width.0,
            h: bounds.size.height.0,
        };
    }
    result.visible = visible_in_viewport(node, index);
    result.checked = node.states.iter().find_map(|state| match state.as_str() {
        "checked" => Some(true),
        "unchecked" => Some(false),
        _ => None,
    });
    result.states.clone_from(&node.states);
    if let Some(hint) = &node.hint {
        result.states.push(format!("hint: {hint}"));
    }
    if node.grouped {
        result.states.push("grouped".into());
    }
    if !node.enabled && !result.states.iter().any(|state| state == "disabled") {
        result.states.push("disabled".into());
    }
    if node.selected && !result.states.iter().any(|state| state == "selected") {
        result.states.push("selected".into());
    }
    result.children = index
        .children
        .get(&node.id)
        .into_iter()
        .flatten()
        .map(|child| ui_node(child, index, agent))
        .collect();
    result
}

fn visible_in_viewport(node: &SemanticNode, index: &SnapshotIndex<'_>) -> bool {
    let Some(bounds) = index.bounds.get(&node.id) else {
        return false;
    };
    let viewport = index.viewport;
    let (mut x0, mut y0, mut x1, mut y1) = (0.0_f32, 0.0_f32, viewport.width.0, viewport.height.0);
    let mut parent = node.parent;
    while let Some(id) = parent {
        let Some(ancestor) = index.known.get(&id) else {
            return false;
        };
        if ancestor.clipping {
            let Some(clip) = index.bounds.get(&id) else {
                return false;
            };
            x0 = x0.max(clip.origin.x.0);
            y0 = y0.max(clip.origin.y.0);
            x1 = x1.min(clip.origin.x.0 + clip.size.width.0);
            y1 = y1.min(clip.origin.y.0 + clip.size.height.0);
        }
        parent = ancestor.parent;
    }
    x0 < x1
        && y0 < y1
        && bounds.origin.x.0 < x1
        && x0 < bounds.origin.x.0 + bounds.size.width.0
        && bounds.origin.y.0 < y1
        && y0 < bounds.origin.y.0 + bounds.size.height.0
}

fn keybinding_info(binding: &SemanticKeybinding) -> KeybindingInfo {
    let chord = binding.chord.replace('+', "-");
    let scope = if binding.menu_path.is_some() {
        KeybindingScope::Global
    } else {
        KeybindingScope::Focused
    };
    let label = binding
        .label
        .as_deref()
        .unwrap_or(&binding.id)
        .to_ascii_lowercase();
    let dangerous_label = [
        "quit", "exit", "delete", "remove", "reset", "restart", "sign out",
    ]
    .iter()
    .any(|word| label.contains(word));
    let mut entry = KeybindingInfo::new(&binding.id, chord, scope, dangerous_label);
    entry.dangerous |= gpui_agent::binding_is_dangerous(&entry);
    entry
}

/// Validates the worker's bind address and token without opening a socket.
pub fn validate_worker_config(addr: SocketAddr, token: &str) -> Result<(), String> {
    gpui_agent::ensure_loopback(addr).map_err(|error| error.to_string())?;
    if token.trim().is_empty() {
        return Err("worker token must not be empty".into());
    }
    Ok(())
}

/// Bound worker lifecycle. Dropping it requests shutdown without blocking.
#[non_exhaustive]
pub struct WorkerHandle {
    addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

impl WorkerHandle {
    /// The address selected by the operating system.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Whether it still serves: not once its accept loop ended or a
    /// shutdown was requested (a client's `shutdown` request sets that too).
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire) && !self.shutdown.load(Ordering::Acquire)
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

/// Starts a loopback-only worker with upstream GPUI-agent authentication.
#[cfg(feature = "dev-agent")]
pub fn spawn_worker(
    addr: SocketAddr,
    token: String,
    mut host: NshAgentHost,
) -> Result<WorkerHandle, String> {
    validate_worker_config(addr, &token)?;
    host.token_required = true;
    let listener = std::net::TcpListener::bind(addr).map_err(|error| error.to_string())?;
    let local_addr = listener.local_addr().map_err(|error| error.to_string())?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let running = Arc::new(AtomicBool::new(true));
    let serving = running.clone();
    let serve_shutdown = shutdown.clone();
    let thread = std::thread::Builder::new()
        .name("nsh-gpui-agent".into())
        .spawn(move || {
            serve_worker(listener, token, Arc::new(Mutex::new(host)), serve_shutdown);
            serving.store(false, Ordering::Release);
        })
        .map_err(|error| error.to_string())?;
    drop(thread);
    Ok(WorkerHandle {
        addr: local_addr,
        shutdown,
        running,
    })
}

/// Serves `host` on `listener` the way `AgentServer::serve_host` does — the
/// same auth challenge, one dispatch thread per connection, `Op::Shutdown`
/// and confirmed quit handling — except shutdown also stops established
/// connections: each connection loop re-checks the flag between requests,
/// and the accept loop closes every live stream when it ends. gpui-agent
/// 0.3.0's `serve_host` checks the flag only while accepting, so a client
/// connected before a `WorkerHandle` drop kept dispatching.
#[cfg(feature = "dev-agent")]
fn serve_worker(
    listener: std::net::TcpListener,
    token: String,
    host: Arc<Mutex<NshAgentHost>>,
    shutdown: Arc<AtomicBool>,
) {
    use std::net::TcpStream;
    use std::sync::atomic::AtomicU64;

    let _published = listener.local_addr().ok().and_then(|addr| {
        let app = host
            .lock()
            .map(|guard| guard.hello().app)
            .unwrap_or_default();
        gpui_agent::discovery::publish_current(addr, &app).ok()
    });
    let limits = gpui_agent::ServerLimits::default();
    listener.set_nonblocking(true).ok();
    let inflight = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let next_connection = Arc::new(AtomicU64::new(0));
    let live: Arc<Mutex<HashMap<u64, TcpStream>>> = Arc::new(Mutex::new(HashMap::new()));
    while !shutdown.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                if inflight.fetch_add(1, Ordering::SeqCst) >= limits.max_connections {
                    inflight.fetch_sub(1, Ordering::SeqCst);
                    drop(stream);
                    continue;
                }
                let connection = next_connection.fetch_add(1, Ordering::SeqCst);
                if let Ok(clone) = stream.try_clone() {
                    live.lock().expect("live streams").insert(connection, clone);
                }
                let host = host.clone();
                let token = token.clone();
                let shutdown = shutdown.clone();
                let inflight = inflight.clone();
                let live = live.clone();
                let limits = limits.clone();
                std::thread::spawn(move || {
                    // The slot frees on unwind too: a panicking handler must
                    // not pin the connection or its slot forever.
                    struct Guard {
                        live: Arc<Mutex<HashMap<u64, TcpStream>>>,
                        inflight: Arc<std::sync::atomic::AtomicUsize>,
                        connection: u64,
                    }
                    impl Drop for Guard {
                        fn drop(&mut self) {
                            self.live
                                .lock()
                                .expect("live streams")
                                .remove(&self.connection);
                            self.inflight.fetch_sub(1, Ordering::SeqCst);
                        }
                    }
                    let _guard = Guard {
                        live,
                        inflight,
                        connection,
                    };
                    serve_worker_stream(stream, host, &token, &shutdown, &limits);
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(_) => break,
        }
    }
    // Established connections end now rather than at their next read
    // timeout; their threads exit as their blocked reads fail.
    for stream in live.lock().expect("live streams").values() {
        let _ = stream.shutdown(std::net::Shutdown::Both);
    }
}

/// One worker connection's request loop: identical to gpui-agent's
/// `handle_stream_host` but it stops as soon as shutdown is requested.
#[cfg(feature = "dev-agent")]
fn serve_worker_stream(
    stream: std::net::TcpStream,
    host: Arc<Mutex<NshAgentHost>>,
    token: &str,
    shutdown: &Arc<AtomicBool>,
    limits: &gpui_agent::ServerLimits,
) {
    use std::io::{BufReader, Write};

    // Accepted sockets inherit nonblocking from the listener on macOS/BSD;
    // timeouts only work on blocking sockets.
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    let _ = stream.set_nodelay(true);
    if stream.set_read_timeout(Some(limits.idle_timeout)).is_err()
        || stream.set_write_timeout(Some(limits.idle_timeout)).is_err()
    {
        return;
    }
    let mut writer = match stream.try_clone() {
        Ok(stream) => stream,
        Err(_) => return,
    };
    let mut reader = BufReader::new(stream);
    let mut line_buf = Vec::with_capacity(4096);
    let mut encode_buf = Vec::with_capacity(4096);
    let session_nonce = match gpui_agent::hmac_auth::random_nonce() {
        Ok(nonce) => {
            if gpui_agent::write_json_line(
                &mut writer,
                &mut encode_buf,
                &gpui_agent::challenge_for_nonce(&nonce),
            )
            .is_err()
                || writer.flush().is_err()
            {
                return;
            }
            Some(nonce)
        }
        Err(_) => return,
    };
    let write_response = |writer: &mut std::net::TcpStream,
                          encode_buf: &mut Vec<u8>,
                          resp: &gpui_agent::Response| {
        if !resp.ok && resp.error_code.is_none() {
            let mut coded = resp.clone();
            coded.fill_error_code();
            let _ = gpui_agent::write_json_line(writer, encode_buf, &coded);
        } else {
            let _ = gpui_agent::write_json_line(writer, encode_buf, resp);
        }
    };
    let mut cache = gpui_agent::TreeCache::new();
    loop {
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        match gpui_agent::read_limited_line_into(&mut reader, &mut line_buf, limits.max_line_bytes)
        {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                write_response(
                    &mut writer,
                    &mut encode_buf,
                    &gpui_agent::Response::err("?", format!("bad request: {error}")),
                );
                return;
            }
            Err(_) => return,
        }
        if gpui_agent::line_is_blank(&line_buf) {
            continue;
        }
        let req: gpui_agent::Request = match serde_json::from_slice(&line_buf) {
            Ok(req) => req,
            Err(error) => {
                write_response(
                    &mut writer,
                    &mut encode_buf,
                    &gpui_agent::Response::err("?", format!("bad json: {error}")),
                );
                return;
            }
        };
        let nonce = session_nonce.as_ref().map(|nonce| nonce.as_slice());
        if let Err(resp) = gpui_agent::authorize_request(&req, Some(token), nonce) {
            write_response(&mut writer, &mut encode_buf, &resp);
            return;
        }
        let shutdown_op = matches!(req.op, Op::Shutdown);
        let confirmed_quit = gpui_agent::op_is_confirmed_quit(&req.op);
        let timeout_ms = match &req.op {
            Op::Wait {
                timeout_ms: Some(timeout_ms),
            } => Some(*timeout_ms),
            Op::WaitUntil { timeout_ms, .. } => Some(*timeout_ms),
            _ => None,
        };
        let resp = if let Some(timeout_ms) = timeout_ms {
            // Timed waits poll the way gpui-agent's own `serve_host` does: one
            // lock per check, released between polls, so another client can
            // change the UI this one waits on. Shutdown stops a wait too.
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
            loop {
                if shutdown.load(Ordering::SeqCst) {
                    return;
                }
                let step = {
                    let host = host.lock().expect("host");
                    worker_wait_step(&host, &mut cache, &req, Some(token), deadline)
                };
                if let Some(resp) = step {
                    break resp;
                }
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                std::thread::sleep(remaining.min(std::time::Duration::from_millis(10)));
            }
        } else {
            let mut host = host.lock().expect("host");
            if shutdown.load(Ordering::SeqCst) {
                return;
            }
            gpui_agent::handle_request_cached(&mut *host, &mut cache, req, Some(token), nonce)
        };
        write_response(&mut writer, &mut encode_buf, &resp);
        if shutdown_op || (confirmed_quit && resp.ok) {
            shutdown.store(true, Ordering::SeqCst);
            return;
        }
    }
}

/// One poll of a timed `Op::Wait`/`Op::WaitUntil`, matching gpui-agent
/// 0.3.0's `dispatch::wait_step` (which is `pub(crate)` there): `None` while
/// the wait can still be satisfied, `Some` once it resolves or times out.
/// Called with the host lock held for exactly one check.
#[cfg(feature = "dev-agent")]
fn worker_wait_step(
    host: &NshAgentHost,
    cache: &mut gpui_agent::TreeCache,
    req: &gpui_agent::Request,
    token: Option<&str>,
    deadline: std::time::Instant,
) -> Option<gpui_agent::Response> {
    match &req.op {
        Op::Wait {
            timeout_ms: Some(_),
        } => {
            let mut hello = host.hello();
            if hello.ready {
                hello.auth = gpui_agent::HelloAuth::from_token_configured(token);
                gpui_agent::discovery::fill_hello_identity(&mut hello);
                let mut response = gpui_agent::Response::ok(&req.id);
                response.hello = Some(hello);
                Some(response)
            } else if std::time::Instant::now() >= deadline {
                Some(gpui_agent::Response::err(&req.id, "wait timed out"))
            } else {
                None
            }
        }
        Op::WaitUntil { spec, .. } => {
            let tree = cache.snapshot(host);
            match gpui_agent::assert_tree(&tree, spec) {
                Ok(()) => Some(gpui_agent::Response::ok(&req.id)),
                Err(error) => {
                    if gpui_agent::is_in_viewport_unavailable(&error)
                        && tree.platform == PlatformKind::Headless
                    {
                        return Some(gpui_agent::Response::err(&req.id, error));
                    }
                    if std::time::Instant::now() >= deadline {
                        return Some(gpui_agent::Response::err(
                            &req.id,
                            format!("wait_until timed out: {error}"),
                        ));
                    }
                    None
                }
            }
        }
        _ => unreachable!("worker_wait_step requires a timed wait op"),
    }
}

/// Validates the upstream environment configuration without starting a worker.
#[cfg(feature = "dev-agent")]
pub fn worker_config_from_env() -> Result<Option<(SocketAddr, String)>, String> {
    if std::env::var("GPUI_AGENT").ok().as_deref() != Some("1") {
        return Ok(None);
    }
    let Some(config) = gpui_agent::from_env().map_err(|error| error.to_string())? else {
        return Err("GPUI_AGENT=1 requires GPUI_AGENT_TOKEN and GPUI_AGENT_ADDR".into());
    };
    let token = config
        .token
        .filter(|token| !token.is_empty())
        .ok_or("GPUI_AGENT=1 requires a nonempty GPUI_AGENT_TOKEN")?;
    validate_worker_config(config.addr, &token)?;
    Ok(Some((config.addr, token)))
}

/// Starts the opt-in development worker from the upstream environment config.
#[cfg(feature = "dev-agent")]
pub fn start_from_env(host: NshAgentHost) -> Result<Option<WorkerHandle>, String> {
    let Some((addr, token)) = worker_config_from_env()? else {
        return Ok(None);
    };
    spawn_worker(addr, token, host).map(Some)
}

#[cfg(test)]
mod tests {
    use nsh_agent_actions::{SemanticScene, StrictNative};
    use nsh_core::geometry::{Bounds, Size};
    use nsh_testing::agent::{Native, Scene};

    use super::*;

    struct SnapshotScene(
        Result<Vec<SemanticNode>, String>,
        u64,
        /// How many times the semantic tree was built.
        Arc<std::sync::atomic::AtomicUsize>,
    );

    impl Scene for SnapshotScene {
        fn activity(&self) -> Result<nsh_testing::agent::Activity, String> {
            Ok(nsh_testing::agent::Activity {
                batches: self.1,
                since_last: Some(std::time::Duration::ZERO),
            })
        }

        fn take_errors(&self) -> Result<Vec<String>, String> {
            Ok(Vec::new())
        }

        fn pick(
            &self,
            _: &str,
            _: nsh_dev::protocol::PickTarget,
        ) -> Result<nsh_testing::Pick, String> {
            Err("unused test scene method".into())
        }

        fn scroll_ancestors(&self, _: NodeId) -> Result<Vec<(NodeId, bool)>, String> {
            Ok(Vec::new())
        }

        fn long_press_delay(&self, _: NodeId) -> Result<Option<f32>, String> {
            Ok(None)
        }

        fn horizontal(&self, _: NodeId) -> Result<bool, String> {
            Ok(false)
        }

        fn texts(&self) -> Result<Vec<String>, String> {
            Ok(Vec::new())
        }

        fn count(&self, _: &str) -> Result<usize, String> {
            Ok(0)
        }

        fn matches(&self, _: &str) -> Result<bool, String> {
            Ok(false)
        }

        fn enabled(&self, _: &str) -> Result<bool, String> {
            Ok(false)
        }

        fn span_style(&self, _: &str) -> Result<nsh_dev::protocol::SpanStyle, String> {
            Err("unused test scene method".into())
        }

        fn view_style(&self, _: &str) -> Result<Option<nsh_dev::protocol::ViewStyle>, String> {
            Err("unused test scene method".into())
        }

        fn input_id(&self, _: &str) -> Result<Option<NodeId>, String> {
            Ok(None)
        }

        fn dialogs(&self) -> Result<Vec<nsh_core::Dialog>, String> {
            Ok(Vec::new())
        }

        fn pending_dialog(&self) -> Result<Option<nsh_core::Dialog>, String> {
            Ok(None)
        }

        fn answered(&self) -> Result<(), String> {
            Ok(())
        }

        fn menu(&self) -> Result<Option<Vec<nsh_core::desktop::MenuItem>>, String> {
            Ok(None)
        }

        fn menu_closed(&self) -> Result<(), String> {
            Ok(())
        }

        fn clipboard(&self) -> Result<Option<String>, String> {
            Ok(None)
        }

        fn storage(&self) -> Result<std::collections::BTreeMap<String, String>, String> {
            Ok(std::collections::BTreeMap::new())
        }

        fn set_viewport(&self, _: Size<Points>) -> Result<(), String> {
            Ok(())
        }

        fn span(&self, _: NodeId) -> Result<Option<(NodeId, u32, u32)>, String> {
            Ok(None)
        }

        fn move_clock(&self, _: nsh_dev::protocol::ClockChange) -> Result<(), String> {
            Ok(())
        }

        fn send_host_event(&self, _: &str, _: serde_json::Value) -> Result<(), String> {
            Ok(())
        }

        fn opened_urls(&self) -> Result<Vec<String>, String> {
            Ok(Vec::new())
        }
    }

    impl SemanticScene for SnapshotScene {
        fn semantic_tree(&self) -> Result<Vec<SemanticNode>, String> {
            self.2.fetch_add(1, Ordering::Relaxed);
            self.0.clone()
        }
    }

    type DragRecords = Arc<std::sync::Mutex<Vec<(Point<Points>, Point<Points>)>>>;

    /// Available, the text of a masked input, not laid out.
    struct SnapshotNative {
        available: bool,
        input: Option<String>,
        viewport_unlaid: bool,
        hit: Option<NodeId>,
        presses: Arc<std::sync::Mutex<Vec<Point<Points>>>>,
        drags: DragRecords,
        keyboard: Result<Option<Bounds<Points>>, String>,
        /// The platform's text menu showing.
        text_menu: Option<nsh_agent_actions::TextMenu>,
    }

    impl SnapshotNative {
        fn new(available: bool, input: Option<String>, viewport_unlaid: bool) -> Self {
            Self {
                available,
                input,
                viewport_unlaid,
                hit: Some(3),
                presses: Arc::default(),
                drags: Arc::default(),
                keyboard: Ok(None),
                text_menu: None,
            }
        }
    }

    impl StrictNative for SnapshotNative {
        fn available(&self) -> bool {
            self.available
        }

        fn keyboard_frame(&self) -> Result<Option<Bounds<Points>>, String> {
            self.keyboard.clone()
        }

        fn laid_out(&self) -> Result<(), String> {
            if self.viewport_unlaid {
                Err("the viewport is not laid out yet".into())
            } else {
                Ok(())
            }
        }

        fn hit_at(&self, _: Point<Points>) -> Result<Option<NodeId>, String> {
            Ok(self.hit)
        }

        fn text_menu(&self) -> Option<nsh_agent_actions::TextMenu> {
            self.text_menu.clone()
        }
    }

    impl Native for SnapshotNative {
        fn bounds(&self, id: NodeId) -> Option<nsh_core::geometry::Bounds<Points>> {
            let (x, y, width, height) = match id {
                1 => (0.0, 0.0, 100.0, 100.0),
                2 => (80.0, 80.0, 30.0, 30.0),
                3 => (5.0, 5.0, 10.0, 10.0),
                5 => (0.0, 45.0, 50.0, 5.0),
                _ => return None,
            };
            Some(Bounds::new(
                Point::new(Points(x), Points(y)),
                Size::new(Points(width), Points(height)),
            ))
        }

        fn viewport(&self) -> Size<Points> {
            Size::new(Points(50.0), Points(50.0))
        }

        fn press(&self, at: Point<Points>) {
            self.presses.lock().unwrap().push(at);
        }

        fn long_press(&self, _: Point<Points>, _: std::time::Duration) {}
        fn scroll(&self, _: Point<Points>, _: Point<Points>) {}

        fn drag(&self, from: Point<Points>, to: Point<Points>) {
            self.drags.lock().unwrap().push((from, to));
        }

        fn type_text(&self, _: &str) {}
        fn key(&self, _: nsh_testing::agent::Key) {}
        fn input_text(&self, id: NodeId) -> Option<String> {
            (id == 3).then(|| self.input.clone()).flatten()
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
            Err("not available".into())
        }
        fn resize(&self, _: Points, _: Points) -> Result<(), String> {
            Ok(())
        }
        fn set_dark(&self, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn quit(&self) {}
    }

    fn snapshot_host_with_availability(scene: SnapshotScene, available: bool) -> NshAgentHost {
        snapshot_host_with_native(scene, SnapshotNative::new(available, None, false))
    }

    fn snapshot_host_with_native(scene: SnapshotScene, native: SnapshotNative) -> NshAgentHost {
        let agent = Arc::new(Agent::requiring_native_hit_test(
            Arc::new(scene),
            Box::new(native),
        ));
        NshAgentHost::new("test", PlatformKind::Desktop, agent).unwrap()
    }

    fn snapshot_host(scene: SnapshotScene) -> NshAgentHost {
        snapshot_host_with_availability(scene, true)
    }

    fn snapshot_host_on(platform: PlatformKind) -> NshAgentHost {
        let agent = Arc::new(Agent::requiring_native_hit_test(
            Arc::new(SnapshotScene(Ok(sample_nodes()), 1, Default::default())),
            Box::new(SnapshotNative::new(true, None, false)),
        ));
        NshAgentHost::new("test", platform, agent).unwrap()
    }

    fn sample_nodes() -> Vec<SemanticNode> {
        let mut root = SemanticNode::default();
        root.id = 1;
        root.role = "group".into();
        root.clipping = true;
        let mut child = SemanticNode::default();
        child.id = 2;
        child.parent = Some(1);
        child.test_id = Some("offscreen".into());
        child.hint = Some("reveal me".into());
        child.grouped = true;
        let mut visible = SemanticNode::default();
        visible.id = 3;
        visible.parent = Some(1);
        visible.test_id = Some("visible".into());
        visible.hint = Some("search gallery".into());
        visible.grouped = true;
        vec![root, child, visible]
    }

    #[test]
    fn worker_config_requires_loopback_and_nonempty_token() {
        assert!(validate_worker_config("127.0.0.1:17421".parse().unwrap(), "secret").is_ok());
        assert!(validate_worker_config("127.0.0.1:17421".parse().unwrap(), "  ").is_err());
        assert!(validate_worker_config("0.0.0.0:17421".parse().unwrap(), "secret").is_err());
    }

    #[test]
    fn snapshot_preserves_offscreen_metadata_and_clipped_visibility() {
        let host = snapshot_host(SnapshotScene(Ok(sample_nodes()), 1, Default::default()));
        let snapshot = host.snapshot();
        assert!(snapshot.ready);
        assert!(snapshot.nodes[0].visible);
        let child = &snapshot.nodes[0].children[0];
        assert_eq!(child.id, "offscreen");
        assert!(!child.visible);
        assert!(child.states.iter().any(|state| state == "hint: reveal me"));
        assert!(child.states.iter().any(|state| state == "grouped"));
        let compact = gpui_agent::compact_lines(&snapshot);
        assert!(compact.contains("hint: search gallery"));
        assert!(compact.contains("[grouped]"));
    }

    #[test]
    fn snapshot_builds_the_semantic_tree_once() {
        let builds = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let host = snapshot_host(SnapshotScene(Ok(sample_nodes()), 1, builds.clone()));
        let snapshot = host.snapshot();
        assert!(snapshot.ready);
        assert_eq!(snapshot.nodes[0].children.len(), 2);
        assert_eq!(builds.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn snapshot_never_shows_a_masked_input_text() {
        let secret = "hunter2";
        for masked in [true, false] {
            let mut nodes = sample_nodes();
            nodes[2].role = "textbox".into();
            nodes[2].input = true;
            if masked {
                nodes[2].states.push("masked".into());
                nodes[2].value = Some("\u{2022}".repeat(secret.len()));
            }
            let mut host = snapshot_host_with_native(
                SnapshotScene(Ok(nodes), 1, Default::default()),
                SnapshotNative::new(true, Some(secret.into()), false),
            );
            let snapshot = host.snapshot();
            let input = &snapshot.nodes[0].children[1];
            let lines = gpui_agent::compact_lines(&snapshot);
            if masked {
                assert_eq!(input.value.as_deref(), Some("•••••••"));
                assert!(input.states.iter().any(|state| state == "masked"));
                assert!(!lines.contains(secret), "{lines}");
                for op in [
                    Op::type_text("visible", secret),
                    Op::SetValue {
                        target: "visible".into(),
                        value: secret.into(),
                        expect_fingerprint: None,
                    },
                ] {
                    let reply = format!("{:?}", host.dispatch(&op));
                    assert!(!reply.contains(secret), "{reply}");
                }
            } else {
                assert_eq!(input.value.as_deref(), Some(secret));
            }
        }
    }

    #[test]
    fn snapshot_ends_with_the_platforms_text_menu_while_it_shows() {
        for menu in [
            None,
            nsh_agent_actions::TextMenu::from_strings(&[
                "insertion".into(),
                "Paste".into(),
                "Select all".into(),
            ]),
        ] {
            let mut native = SnapshotNative::new(true, None, false);
            native.text_menu = menu.clone();
            let host = snapshot_host_with_native(
                SnapshotScene(Ok(sample_nodes()), 1, Default::default()),
                native,
            );
            let snapshot = host.snapshot();
            let found = snapshot.nodes.iter().find(|node| node.id == TEXT_MENU_ID);
            let Some(menu) = menu else {
                assert!(found.is_none());
                continue;
            };
            let node = found.expect("the text menu's node");
            assert_eq!(node.role, "menu");
            assert!(!node.enabled);
            assert!(node.states.iter().any(|state| state == "system"));
            assert!(node.states.iter().any(|state| state == menu.kind));
            let items: Vec<(&str, bool)> = node
                .children
                .iter()
                .map(|item| (item.name.as_str(), item.enabled))
                .collect();
            assert_eq!(items, [("Paste", false), ("Select all", false)]);
        }
    }

    #[test]
    fn scene_failure_is_not_a_healthy_empty_snapshot() {
        let host = snapshot_host(SnapshotScene(
            Err("model unavailable".into()),
            1,
            Default::default(),
        ));
        let snapshot = host.snapshot();
        assert!(!snapshot.ready);
        assert!(snapshot.nodes.is_empty());
    }

    #[test]
    fn snapshot_is_not_ready_before_the_first_batch() {
        let host = snapshot_host(SnapshotScene(Ok(sample_nodes()), 0, Default::default()));
        let snapshot = host.snapshot();
        assert!(!snapshot.ready);
        assert!(snapshot.nodes.is_empty());
    }

    #[test]
    fn snapshot_is_not_ready_while_the_viewport_is_not_laid_out() {
        let host = snapshot_host_with_native(
            SnapshotScene(Ok(sample_nodes()), 1, Default::default()),
            SnapshotNative::new(true, None, true),
        );
        let snapshot = host.snapshot();
        assert!(!snapshot.ready);
        assert!(snapshot.nodes.is_empty());
    }

    #[test]
    fn hello_is_not_ready_while_the_viewport_is_not_laid_out() {
        let host = snapshot_host_with_native(
            SnapshotScene(Ok(sample_nodes()), 1, Default::default()),
            SnapshotNative::new(true, None, true),
        );
        assert!(!host.hello().ready);
    }

    #[test]
    fn detached_surface_reports_unavailable() {
        let mut host = snapshot_host_with_availability(
            SnapshotScene(Ok(sample_nodes()), 1, Default::default()),
            false,
        );
        assert!(!host.hello().ready);
        assert!(!host.snapshot().ready);
        let error = host
            .dispatch(&Op::Invoke {
                name: "press".into(),
                args: serde_json::json!({"target":"visible"}),
            })
            .unwrap_err();
        assert!(error.contains("unavailable"));
    }

    #[test]
    fn hello_lists_the_gated_operations_the_platform_delivers() {
        let host = snapshot_host_with_native(
            SnapshotScene(Ok(sample_nodes()), 1, Default::default()),
            SnapshotNative::new(true, None, false),
        );
        let hello = host.hello();
        assert_eq!(hello.framework.as_deref(), Some("nsh"));
        for feature in ["press", "scroll", "drag"] {
            assert!(hello.features.iter().any(|name| name == feature));
        }
        let lists = |name: &str| hello.features.iter().any(|feature| feature == name);
        match hello.platform {
            PlatformKind::Mobile => assert!(lists("swipe") && !lists("hover")),
            _ => assert!(lists("hover") && !lists("swipe")),
        }
    }

    fn keyboard_host(keyboard: Result<Option<Bounds<Points>>, String>) -> NshAgentHost {
        let mut native = SnapshotNative::new(true, None, false);
        native.keyboard = keyboard;
        let agent = Arc::new(Agent::requiring_native_hit_test(
            Arc::new(SnapshotScene(Ok(sample_nodes()), 1, Default::default())),
            Box::new(native),
        ));
        NshAgentHost::new("test", PlatformKind::Mobile, agent).unwrap()
    }

    #[test]
    fn snapshot_ends_with_the_software_keyboard_while_it_shows() {
        let frame = Bounds::new(
            Point::new(Points(0.0), Points(500.0)),
            Size::new(Points(390.0), Points(344.0)),
        );
        let snapshot = keyboard_host(Ok(Some(frame))).snapshot();
        let keyboard = snapshot.nodes.last().unwrap();
        assert_eq!(keyboard.id, SOFTWARE_KEYBOARD_ID);
        assert_eq!(keyboard.role, "keyboard");
        assert!(!keyboard.enabled, "the keyboard is not an app control");
        assert_eq!(
            (
                keyboard.bounds.x,
                keyboard.bounds.y,
                keyboard.bounds.w,
                keyboard.bounds.h
            ),
            (0.0, 500.0, 390.0, 344.0)
        );

        for hidden in [Ok(None), Err("unknown".to_string())] {
            let snapshot = keyboard_host(hidden).snapshot();
            assert!(snapshot.ready);
            assert!(
                snapshot
                    .nodes
                    .iter()
                    .all(|node| node.id != SOFTWARE_KEYBOARD_ID),
                "no keyboard node while none shows or the host cannot tell"
            );
        }
    }

    #[test]
    fn mobile_hello_says_whether_it_reports_the_software_keyboard() {
        let lists = |host: NshAgentHost| {
            host.hello()
                .features
                .iter()
                .any(|name| name == SOFTWARE_KEYBOARD_FEATURE)
        };
        assert!(lists(keyboard_host(Ok(None))));
        assert!(!lists(keyboard_host(Err("unknown".into()))));
        assert!(!lists(snapshot_host(SnapshotScene(
            Ok(sample_nodes()),
            1,
            Default::default()
        ))));
    }

    #[test]
    fn mobile_hello_lists_android_only_operations_only_on_android() {
        let hello = snapshot_host_on(PlatformKind::Mobile).hello();
        // Paste comes from every mobile driver — the iOS driver runs on
        // macOS, so this is not an Android-only operation.
        assert!(hello.features.iter().any(|name| name == "paste"));
        for feature in [
            "system.rotate",
            "system.keyboard_show",
            "system.keyboard_hide",
        ] {
            assert_eq!(
                hello.features.iter().any(|name| name == feature),
                cfg!(target_os = "android"),
                "{feature}"
            );
        }
    }

    #[test]
    fn rotate_rejects_values_other_than_portrait_or_landscape() {
        let mut host = snapshot_host(SnapshotScene(Ok(sample_nodes()), 1, Default::default()));
        let error = host
            .dispatch(&Op::System {
                action: gpui_agent::SystemAction::Rotate,
                value: Some("sideways".into()),
            })
            .unwrap_err();
        assert!(error.contains("portrait"));
        assert!(error.contains("landscape"));
    }

    #[test]
    fn a_press_operation_reaches_the_agent_like_the_invoke_it_replaces() {
        let mut nodes = sample_nodes();
        let visible = nodes.iter_mut().find(|node| node.id == 3).unwrap();
        visible.enabled = true;
        visible.pressable = true;
        let presses = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut native = SnapshotNative::new(true, None, false);
        native.presses = presses.clone();
        let mut host =
            snapshot_host_with_native(SnapshotScene(Ok(nodes), 1, Default::default()), native);

        host.dispatch(&Op::Press {
            target: Some("visible".into()),
            x: None,
            y: None,
            expect_fingerprint: None,
        })
        .unwrap();

        assert_eq!(
            *presses.lock().unwrap(),
            vec![Point::new(Points(10.0), Points(10.0))]
        );
    }

    #[test]
    fn mobile_swipe_dispatch_reaches_native_drag() {
        let mut nodes = sample_nodes();
        nodes.iter_mut().find(|node| node.id == 3).unwrap().enabled = true;
        let drags = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut native = SnapshotNative::new(true, None, false);
        native.drags = drags.clone();
        let agent = Arc::new(Agent::requiring_native_hit_test(
            Arc::new(SnapshotScene(Ok(nodes), 1, Default::default())),
            Box::new(native),
        ));
        let mut host = NshAgentHost::new("test", PlatformKind::Mobile, agent).unwrap();

        host.dispatch(&Op::Swipe {
            target: Some("visible".into()),
            direction: gpui_agent::SwipeDirection::Down,
            distance: Some(20.0),
        })
        .unwrap();

        assert_eq!(
            *drags.lock().unwrap(),
            vec![(
                Point::new(Points(10.0), Points(10.0)),
                Point::new(Points(10.0), Points(30.0)),
            )]
        );
    }

    #[test]
    fn semantic_ids_are_unique_even_when_test_ids_collide_with_fallbacks() {
        let mut identified = SemanticNode::default();
        identified.id = 1;
        identified.test_id = Some("n2".into());
        identified.role = "button".into();
        let mut fallback = SemanticNode::default();
        fallback.id = 2;
        let mut duplicate = SemanticNode::default();
        duplicate.id = 3;
        duplicate.test_id = Some("n2".into());
        let mut collision = SemanticNode::default();
        collision.id = 4;
        collision.test_id = Some("n2#2".into());
        let ids = stable_ids(&[identified, fallback, duplicate, collision]);
        assert_eq!(ids.get(&1).map(String::as_str), Some("n2#1"));
        assert_eq!(ids.get(&2).map(String::as_str), Some("n2#2"));
        assert_eq!(ids.get(&3).map(String::as_str), Some("n2#3"));
        assert_eq!(ids.get(&4).map(String::as_str), Some("n2#2#4"));
        assert_eq!(
            viewport_size(&serde_json::json!({"preset": "phone"})).unwrap(),
            (390.0, 844.0)
        );
        assert!(viewport_size(&serde_json::json!({"width": 0, "height": 10})).is_err());
        assert!(optional_point(&serde_json::json!({"x": 1})).is_err());
        assert_eq!(
            optional_point(&serde_json::json!({"x": 2, "y": 3})).unwrap(),
            Some(Point::new(Points(2.0), Points(3.0)))
        );
        assert!(number_arg(&serde_json::json!({"dx": "bad"}), "dx").is_err());
        assert!(number_arg(&serde_json::json!({"dx": 1e300}), "dx").is_err());
    }

    #[test]
    fn quit_menu_shortcuts_require_confirmation() {
        let mut binding = SemanticKeybinding::default();
        binding.id = "menu.File/Quit#1".into();
        binding.chord = "cmd+q".into();
        binding.label = Some("Quit".into());
        binding.menu_path = Some(vec!["File".into(), "Quit".into()]);
        let info = keybinding_info(&binding);
        assert!(gpui_agent::binding_is_dangerous(&info));
        binding.label = Some("Save".into());
        binding.chord = "ctrl+s".into();
        assert!(!keybinding_info(&binding).dangerous);
    }

    #[test]
    fn snapshot_reports_checked_states() {
        let mut nodes = sample_nodes();
        nodes[1].role = "checkbox".into();
        nodes[1].states.push("unchecked".into());
        nodes[2].role = "checkbox".into();
        nodes[2].states.push("checked".into());
        let host = snapshot_host(SnapshotScene(Ok(nodes), 1, Default::default()));

        let snapshot = host.snapshot();

        assert_eq!(snapshot.nodes[0].checked, None);
        assert_eq!(snapshot.nodes[0].children[0].checked, Some(false));
        assert_eq!(snapshot.nodes[0].children[1].checked, Some(true));
    }

    #[test]
    fn mobile_swipe_default_distance_stays_inside_the_viewport() {
        // Node 5 fills a 5pt sliver at the bottom of a 50pt viewport: an
        // unclamped 40% default (20pt) would swipe past the screen edge.
        let mut nodes = sample_nodes();
        let mut sliver = SemanticNode::default();
        sliver.id = 5;
        sliver.parent = Some(1);
        sliver.test_id = Some("sliver".into());
        sliver.enabled = true;
        nodes.push(sliver);
        let drags: DragRecords = Arc::default();
        let mut native = SnapshotNative::new(true, None, false);
        native.drags = drags.clone();
        native.hit = Some(5);
        let agent = Arc::new(Agent::requiring_native_hit_test(
            Arc::new(SnapshotScene(Ok(nodes), 1, Default::default())),
            Box::new(native),
        ));
        let mut host = NshAgentHost::new("test", PlatformKind::Mobile, agent).unwrap();

        host.dispatch(&Op::Swipe {
            target: Some("sliver".into()),
            direction: gpui_agent::SwipeDirection::Down,
            distance: None,
        })
        .unwrap();

        assert_eq!(
            *drags.lock().unwrap(),
            vec![(
                Point::new(Points(25.0), Points(47.5)),
                Point::new(Points(25.0), Points(49.0))
            )]
        );
    }

    #[cfg(feature = "dev-agent")]
    #[test]
    fn dropping_the_worker_closes_its_connections() {
        let host = snapshot_host(SnapshotScene(Ok(sample_nodes()), 1, Default::default()));
        let handle = spawn_worker("127.0.0.1:0".parse().unwrap(), "token".into(), host).unwrap();
        let mut client = gpui_agent::AgentClient::connect(handle.local_addr()).with_token("token");
        client.call(&Op::Hello).unwrap();

        drop(handle);

        // The established connection stops dispatching once the worker
        // is dropped; the next call fails instead of being served.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while client.call(&Op::Hello).is_ok() {
            assert!(
                std::time::Instant::now() < deadline,
                "connection keeps dispatching after the worker was dropped"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[cfg(feature = "dev-agent")]
    #[test]
    fn a_timed_wait_does_not_block_other_clients() {
        let host = snapshot_host(SnapshotScene(Ok(sample_nodes()), 1, Default::default()));
        let handle = spawn_worker("127.0.0.1:0".parse().unwrap(), "token".into(), host).unwrap();
        let addr = handle.local_addr();

        // One client waits on a node that never appears: it keeps polling
        // until its 10 s deadline.
        let mut waiting = gpui_agent::AgentClient::connect(addr).with_token("token");
        let waiter = std::thread::spawn(move || {
            waiting.call(&Op::WaitUntil {
                timeout_ms: 10_000,
                spec: gpui_agent::AssertSpec {
                    target: "n9999".into(),
                    ..Default::default()
                },
            })
        });
        // Give the wait time to reach its first poll.
        std::thread::sleep(std::time::Duration::from_millis(100));

        // A second client must be served while the first still waits: holding
        // the host lock for the whole wait would block this call for 10 s.
        let mut second = gpui_agent::AgentClient::connect(addr).with_token("token");
        let started = std::time::Instant::now();
        let hello = second.call(&Op::Hello).expect("hello dispatches");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "second client blocked behind a timed wait"
        );
        assert!(hello.ok, "{hello:?}");

        // Dropping the worker interrupts the still-running wait.
        drop(handle);
        let result = waiter.join().expect("waiter thread");
        assert!(
            result.is_err(),
            "timed wait kept running after drop: {result:?}"
        );
    }
}
