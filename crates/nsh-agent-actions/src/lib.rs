//! Strict agent actions on a Native Shell host.
//!
//! [`Agent`] acts on the app a host shows through the host's
//! [`Scene`](nsh_testing::agent::Scene) and [`Native`](nsh_testing::agent::Native),
//! as `nsh test --target`'s agent does, but on nodes of a semantic tree of the
//! mounted views ([`SemanticScene`]) and at points, and with strict
//! actionability: before any input it proves with a native hit-test
//! ([`StrictNative::hit_at`]) that the target, not a view covering it, gets
//! it, and it refuses disabled, offscreen and clipped targets.

use std::time::Duration;

use nsh_core::time::Instant;
use nsh_core::{
    NodeId,
    geometry::{Bounds, Point, Points, Size, point},
};
use nsh_dev::protocol::PickTarget;
use nsh_testing::{
    Pick,
    agent::{Key, LANDING, Native, Scene},
};

mod failure;
mod screen_data;
mod semantic;
#[cfg(test)]
mod tests;

/// How long after the last batch the app is quiet.
const QUIET: Duration = Duration::from_millis(100);
/// How long a step waits for the app to be quiet at most.
const SETTLE: Duration = Duration::from_secs(2);
/// How often waits look again while nothing comes.
const POLL: Duration = Duration::from_millis(20);
/// A long press's hold when the view does not say (`delayLongPress`).
const LONG_PRESS: Duration = Duration::from_millis(500);
/// How many gestures scrolling a view into view takes at most: only a
/// guard against a view that keeps moving without ever arriving. Gestures
/// that move nothing end it sooner ([`anchor`]), and each moves at most
/// part of its scroll view, so a row far down a long list in a small window
/// (the gallery's home at 640x480 needs about 30) takes many.
const SCROLLS_INTO_VIEW: usize = 100;
/// Error returned by platforms that cannot perform a native hit-test.
pub const NATIVE_HIT_TEST_UNSUPPORTED: &str = "native hit-testing is unsupported";
/// Error returned by platforms that cannot focus a node without a press.
pub const NATIVE_FOCUS_UNSUPPORTED: &str = "native focus is unsupported on this platform";
/// Error returned by platforms that cannot fire a keymap binding by
/// identity (only by chord, which a shared chord makes ambiguous).
pub const NATIVE_KEYBINDING_UNSUPPORTED: &str =
    "native keybinding dispatch is unsupported on this platform";

/// A swipe's direction on the surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwipeDirection {
    /// Towards the top edge.
    Up,
    /// Towards the bottom edge.
    Down,
    /// Towards the left edge.
    Left,
    /// Towards the right edge.
    Right,
}

/// The native destination represented by a semantic node.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
#[non_exhaustive]
pub enum SemanticTarget {
    /// A native view with the same ID as its semantic node.
    #[default]
    View,
    /// A canvas node, with bounds in its canvas owner's local coordinates.
    Canvas {
        /// The native canvas view.
        owner: NodeId,
        /// The node's bounds relative to its canvas.
        bounds: Bounds<Points>,
    },
    /// A text span, with offsets in UTF-16 code units.
    Text {
        /// The native text view.
        owner: NodeId,
        /// Inclusive start offset.
        start: u32,
        /// Exclusive end offset.
        end: u32,
    },
}

/// Platform-independent accessibility data for one mounted view.
///
/// Nodes are returned in deterministic preorder. Hidden accessibility
/// subtrees are omitted, while offscreen nodes remain present.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
#[non_exhaustive]
pub struct SemanticNode {
    /// The engine's node identifier.
    pub id: NodeId,
    /// The containing node, if any.
    pub parent: Option<NodeId>,
    /// The app-provided test identifier.
    pub test_id: Option<String>,
    /// The accessibility role, or `generic`.
    pub role: String,
    /// The accessible label, falling back to visible text.
    pub label: Option<String>,
    /// The accessibility hint.
    pub hint: Option<String>,
    /// The text or range value, when present.
    pub value: Option<String>,
    /// Whether this node and its ancestors are enabled.
    pub enabled: bool,
    /// Whether the node is selected.
    pub selected: bool,
    /// Whether this node groups its descendants for accessibility.
    pub grouped: bool,
    /// Whether this node clips its descendants.
    pub clipping: bool,
    /// Whether this node is a modal presentation's native root. A modal is
    /// painted as a layer of its own, not inside its logical parent's box.
    #[serde(default)]
    pub modal: bool,
    /// Whether this node is a text input.
    pub input: bool,
    /// Whether this node scrolls its contents.
    pub scroll: bool,
    /// Whether this node handles press events.
    pub pressable: bool,
    /// Whether this node handles long-press events.
    #[serde(default)]
    pub long_press: bool,
    /// Whether this node observes raw touch events.
    #[serde(default)]
    pub touch: bool,
    /// The node's accessibility states.
    pub states: Vec<String>,
    /// The native view or precise native region represented by this node.
    #[serde(default)]
    pub native_target: SemanticTarget,
    /// The mounted views under this node the tree leaves out (hidden
    /// subtrees, as an icon button's decorative icon): a native hit on one
    /// is a hit inside this node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_descendants: Vec<NodeId>,
    /// Those of [`SemanticNode::hidden_descendants`] a press goes to a
    /// hidden view of their own: each with that view (itself, or the
    /// innermost hidden view around it that takes presses).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_press_owners: Vec<(NodeId, NodeId)>,
    /// Those of [`SemanticNode::hidden_descendants`] a long press goes
    /// to a hidden view of their own: each with that view (itself, or
    /// the innermost hidden view around it that takes holds).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_hold_owners: Vec<(NodeId, NodeId)>,
    /// The link spans of hidden text views under this node, as
    /// `(link, text view, start, end)`: a native hit reports the text
    /// view while the gesture can belong to the link at the point.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_text_spans: Vec<(NodeId, NodeId, u32, u32)>,
}

/// A keyboard shortcut the host can fire through its native input path.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct SemanticKeybinding {
    /// Stable identifier supplied by the app or menu.
    pub id: String,
    /// The shortcut in `nsh/keymap` notation.
    pub chord: String,
    /// The visible label, when this comes from a menu.
    pub label: Option<String>,
    /// The menu action path, or `None` for a keymap binding.
    pub menu_path: Option<Vec<String>>,
    /// The menu action's stable identifier, when this comes from a menu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu_action_id: Option<u32>,
}

impl SemanticKeybinding {
    /// The numeric keymap binding id the semantic id carries
    /// (`keymap.<id>.<chord>`): `None` for a menu binding.
    pub fn keymap_id(&self) -> Option<u32> {
        self.id
            .strip_prefix("keymap.")?
            .split('.')
            .next()?
            .parse()
            .ok()
    }
}

#[derive(Clone, Copy)]
struct TargetGeometry {
    owner: NodeId,
    bounds: Bounds<Points>,
    synthetic: bool,
}

#[derive(Clone, Copy)]
enum PointActionability {
    Any,
    Press,
    LongPress,
}

/// What the agent sees of a host's app beyond [`Scene`]: a semantic tree
/// of its mounted views and the shortcuts it can fire.
pub trait SemanticScene: Scene {
    /// The app's portable semantic scene, including offscreen nodes.
    fn semantic_tree(&self) -> Result<Vec<SemanticNode>, String> {
        Err("the host does not provide a semantic scene".into())
    }
    /// Keyboard shortcuts from the app's keymap and visible menu.
    fn keybindings(&self) -> Result<Vec<SemanticKeybinding>, String> {
        Ok(Vec::new())
    }
    /// Whether a long press on view `id` is refused: the nearest view at or
    /// around it that hears long presses, presses or touches is disabled,
    /// or a view between them is.
    fn long_press_disabled(&self, id: NodeId) -> Result<bool, String> {
        let _ = id;
        Ok(false)
    }
    /// The view the platform hands the long press of a hold starting on
    /// `id`: `pick_press`'s `long_node` when set, else the picked
    /// press's `node`. GPUI's pick gives the press to the innermost
    /// enabled view listening to `Press`, `LongPress`, `PressDown` or
    /// `PressUp`; when that view listens only to `LongPress`, taps fall
    /// to the nearest tap-listening ancestor while the long press stays
    /// with it — this method returns that view, not the tap owner, so
    /// the answer can differ from both the model's tap pick and its hold
    /// pick. It can also name a blocker rather than a handler: a view
    /// listening only for press-down/up takes the press yet never fires
    /// a long-press. A view that only observes raw touches is no
    /// recognizer — the hold still reaches an ancestor's — so it does
    /// not appear here. `Err` means the host cannot say: the agent
    /// falls back to the model's long-press pick, which counts touch
    /// listeners.
    fn native_hold_receiver(&self, id: NodeId) -> Result<Option<NodeId>, String> {
        let _ = id;
        Err("the host does not report native hold receivers".into())
    }
}

/// What the agent needs of a platform beyond [`Native`]: proof of what is
/// under a point, focus without a press, and the platform's own lifecycle.
pub trait StrictNative: Native {
    /// Whether the native surface still exists.
    fn available(&self) -> bool {
        true
    }
    /// Returns the native view at `at`; `Err(NATIVE_HIT_TEST_UNSUPPORTED)`
    /// means the host cannot prove that a target is unobstructed.
    fn hit_at(&self, at: Point<Points>) -> Result<Option<NodeId>, String> {
        let _ = at;
        Err(NATIVE_HIT_TEST_UNSUPPORTED.into())
    }
    /// Checks whether `keys` can be sent before an input target is focused.
    fn validate_keys(&self, keys: &str) -> Result<(), String> {
        let _ = keys;
        Ok(())
    }
    /// Focuses a text input without synthesizing a pointer press.
    fn focus_node(&self, id: NodeId) -> Result<(), String> {
        let _ = id;
        Err(NATIVE_FOCUS_UNSUPPORTED.into())
    }
    /// Fires a menu shortcut through the platform's native action.
    fn fire_menu_keybinding(&self, binding: &SemanticKeybinding) -> Result<(), String> {
        self.press_keys(&binding.chord)
    }
    /// Fires a keymap binding by identity, so a chord another binding also
    /// claims still runs this command and no other. Platforms that can only
    /// send the chord return `Err(NATIVE_KEYBINDING_UNSUPPORTED)`; the
    /// agent refuses an ambiguous chord rather than firing the wrong one.
    fn fire_keybinding(&self, binding: &SemanticKeybinding) -> Result<(), String> {
        let _ = binding;
        Err(NATIVE_KEYBINDING_UNSUPPORTED.into())
    }
    /// Whether the app currently has native window focus.
    fn app_focused(&self) -> bool {
        false
    }
    /// Requests portrait or landscape orientation.
    fn rotate(&self, landscape: bool) -> Result<(), String> {
        let _ = landscape;
        Err("rotate is not supported on this platform".into())
    }
    /// Where the software keyboard covers the surface, in the surface's
    /// points: `Ok(None)` while none shows (a desktop has none). `Err`
    /// means the host cannot tell. A press whose point is inside the frame
    /// would land on the keyboard, not on the app, and is refused.
    fn keyboard_frame(&self) -> Result<Option<Bounds<Points>>, String> {
        Ok(None)
    }

    /// Shows or hides the software keyboard.
    fn keyboard(&self, shown: bool) -> Result<(), String> {
        let _ = shown;
        Err("keyboard is not supported on this platform".into())
    }
    /// Whether the window shows the app laid out at its current size: an
    /// error while it has no size (minimized) or a resize is still being
    /// laid out.
    fn laid_out(&self) -> Result<(), String> {
        Ok(())
    }
    /// A failure an earlier call met without returning it (a main thread
    /// that did not answer), taken once.
    fn take_failure(&self) -> Option<String> {
        None
    }
    /// The platform's own text menu showing now ([`TextMenu`]), read
    /// without touching it. `None` when none shows or the platform does
    /// not say.
    fn text_menu(&self) -> Option<TextMenu> {
        None
    }

    /// Presses item `word` ([`TextMenu::PRESSABLE`]) of the platform's own
    /// text menu showing now, through that menu, so the platform's own
    /// handler runs as a finger's tap on it would. Refused when no menu
    /// shows, the item is absent or disabled, or a masked field would give
    /// its text away (cut, copy).
    fn press_text_menu_item(&self, word: &str) -> Result<(), String> {
        let _ = word;
        Err("this platform cannot press its text menu".into())
    }

    /// Whether the clipboard holds a clip this run of the agent put there
    /// (Cut or Copy it pressed, a paste it made): `None` when the platform
    /// cannot tell. A Paste the agent presses needs `Some(true)`, so it never
    /// inserts what a person or another app copied.
    fn clipboard_set_by_agent(&self) -> Option<bool> {
        None
    }

    /// The selections of inputs `ids` as the platform holds them, read in
    /// one go (a snapshot asks for every input at once, so a host whose
    /// views live on another thread pays one hop, not one per field). Each
    /// is its start and end in UTF-16 code units, start first: the
    /// direction a selection was made in is dropped, and a bare caret has
    /// equal ends. An input the platform says nothing of (or an id that is
    /// no input) is left out; the default says nothing of any.
    fn input_selections(&self, ids: &[NodeId]) -> std::collections::HashMap<NodeId, (u32, u32)> {
        let _ = ids;
        std::collections::HashMap::new()
    }
}

/// Whether `at` is inside `bounds` clear of its edges along the axis a
/// scroll view moves it (`horizontal`): of the outer tenth of each side,
/// 48 points at most. Across that axis only inside: scrolling cannot move
/// it there (a button at a row's end).
fn well_inside(bounds: &Bounds<Points>, at: Point<Points>, horizontal: bool) -> bool {
    let margin = |length: f32| (length * 0.1).min(48.0);
    let (margin_x, margin_y) = if horizontal {
        (margin(bounds.size.width.0), 0.0)
    } else {
        (0.0, margin(bounds.size.height.0))
    };
    let (x0, y0) = (bounds.origin.x.0 + margin_x, bounds.origin.y.0 + margin_y);
    let (x1, y1) = (
        bounds.origin.x.0 + bounds.size.width.0 - margin_x,
        bounds.origin.y.0 + bounds.size.height.0 - margin_y,
    );
    (x0..=x1).contains(&at.x.0) && (y0..=y1).contains(&at.y.0)
}

/// The spot of a scroll view `extent` points long from `start` (along the
/// axis it scrolls) nearest `along` (a fraction of its length) that no
/// span of `covered` (in points along it) holds, a finger's margin clear of
/// each, within the middle 70% of it (where a gesture has room); as a
/// fraction of its length, `None` when no spot is clear.
fn clear_spot(start: f32, extent: f32, along: f32, covered: &[(f32, f32)]) -> Option<f32> {
    const MARGIN: f32 = 8.0;
    if extent <= 0.0 {
        return None;
    }
    let (lo, hi) = (start + extent * 0.15, start + extent * 0.85);
    let clear = |p: f32| {
        (lo..=hi).contains(&p)
            && covered
                .iter()
                .all(|&(from, to)| p <= from - MARGIN || p >= to + MARGIN)
    };
    let want = start + extent * along;
    // The nearest clear spot is `along` itself, an end of the range or
    // one just past a span.
    std::iter::once(want)
        .chain([lo, hi])
        .chain(
            covered
                .iter()
                .flat_map(|&(from, to)| [from - MARGIN, to + MARGIN]),
        )
        .filter(|&p| clear(p))
        .min_by(|a, b| (a - want).abs().total_cmp(&(b - want).abs()))
        .map(|p| (p - start) / extent)
}

/// Where along a scroll view a finger lands to scroll it, as a fraction of
/// its length, after `misses` gestures that moved nothing (`None`: every
/// spot was tried): its middle, then off the middle, for what scrolls by
/// itself there (a text area), as a user moves the finger: first a quarter
/// of the way from the edge the finger moves away from (the end when it
/// moves `to_start`), where it has the most room, so a gesture undone (a
/// fling back) costs the next ones no reach, then a quarter of the way
/// from the other.
fn anchor(misses: usize, to_start: bool) -> Option<f32> {
    let (roomy, other) = if to_start { (0.75, 0.25) } else { (0.25, 0.75) };
    [0.5, roomy, other].get(misses).copied()
}

fn intersection(left: Bounds<Points>, right: Bounds<Points>) -> Option<Bounds<Points>> {
    let x0 = left.origin.x.0.max(right.origin.x.0);
    let y0 = left.origin.y.0.max(right.origin.y.0);
    let x1 = (left.origin.x.0 + left.size.width.0).min(right.origin.x.0 + right.size.width.0);
    let y1 = (left.origin.y.0 + left.size.height.0).min(right.origin.y.0 + right.size.height.0);
    (x0 < x1 && y0 < y1).then(|| {
        Bounds::new(
            Point::new(Points(x0), Points(y0)),
            Size::new(Points(x1 - x0), Points(y1 - y0)),
        )
    })
}

/// Two chord strings name the same shortcut when their parts match after
/// folding case and common key-name aliases — catalogue spellings like
/// `ctrl+escape` and `⌃⎋` would otherwise look like different chords and
/// let a shared one go out as plain keys.
fn normalize_chord(chord: &str) -> Vec<String> {
    let mut parts: Vec<String> = chord
        .split(['+', '-'])
        .map(|part| {
            let part = part.trim().to_lowercase();
            match part.as_str() {
                "esc" | "⎋" => "escape".to_string(),
                "cmd" | "command" | "⌘" | "super" | "win" | "windows" => "meta".to_string(),
                "ctl" | "control" | "⌃" => "ctrl".to_string(),
                "opt" | "option" | "⌥" => "alt".to_string(),
                "⇧" => "shift".to_string(),
                "return" | "↵" | "⏎" => "enter".to_string(),
                "del" => "delete".to_string(),
                "spacebar" | "␣" => "space".to_string(),
                // The menu catalogue's key labels spell these short.
                "pageup" => "pgup".to_string(),
                "pagedown" => "pgdn".to_string(),
                "insert" => "ins".to_string(),
                _ => part,
            }
        })
        .filter(|part| !part.is_empty())
        .collect();
    parts.sort();
    parts
}

/// The window point `within` names in a node whose native frame is
/// `bounds`: `within` is from the frame's top-left corner, in points, and
/// must fall inside the frame.
fn point_within(
    bounds: Bounds<Points>,
    within: Point<Points>,
    selector: &str,
) -> Result<Point<Points>, String> {
    if !within.x.0.is_finite()
        || !within.y.0.is_finite()
        || within.x.0 < 0.0
        || within.y.0 < 0.0
        || within.x.0 >= bounds.size.width.0
        || within.y.0 >= bounds.size.height.0
    {
        return Err(format!(
            "`{selector}`: ({}, {}) is outside the node ({} by {} points)",
            within.x.0, within.y.0, bounds.size.width.0, bounds.size.height.0
        ));
    }
    Ok(Point::new(
        Points(bounds.origin.x.0 + within.x.0),
        Points(bounds.origin.y.0 + within.y.0),
    ))
}

/// What a press on `target` goes to: itself when it is an input or
/// pressable, else its nearest pressable ancestor (`None`: nothing takes it).
fn press_owner(nodes: &[SemanticNode], target: &SemanticNode) -> Option<NodeId> {
    if target.input || target.pressable {
        return Some(target.id);
    }
    let mut parent = target.parent;
    while let Some(parent_id) = parent {
        let ancestor = nodes.iter().find(|node| node.id == parent_id)?;
        if ancestor.pressable {
            return Some(ancestor.id);
        }
        parent = ancestor.parent;
    }
    None
}

/// Every mounted view's parent the semantic tree knows: its nodes', and
/// its hidden views' ([`SemanticNode::hidden_descendants`]), linked to the
/// node they are under.
fn mounted_parents(nodes: &[SemanticNode]) -> std::collections::HashMap<NodeId, Option<NodeId>> {
    nodes
        .iter()
        .flat_map(|node| {
            std::iter::once((node.id, node.parent)).chain(
                node.hidden_descendants
                    .iter()
                    .map(move |&hidden| (hidden, Some(node.id))),
            )
        })
        .collect()
}

fn is_descendant_or_same(
    hit: NodeId,
    target: NodeId,
    parents: &std::collections::HashMap<NodeId, Option<NodeId>>,
) -> bool {
    let mut current = Some(hit);
    while let Some(id) = current {
        if id == target {
            return true;
        }
        current = parents.get(&id).copied().flatten();
    }
    false
}

/// How far, in points, a field's edges may lie from its box's and the field
/// still cover the box (rounding of the two frames).
const COVER_SLACK: f32 = 1.0;

/// Whether `inner` covers `outer`: every edge within [`COVER_SLACK`].
fn covers(inner: Bounds<Points>, outer: Bounds<Points>) -> bool {
    let edges = |b: Bounds<Points>| {
        [
            b.origin.x.0,
            b.origin.y.0,
            b.origin.x.0 + b.size.width.0,
            b.origin.y.0 + b.size.height.0,
        ]
    };
    edges(inner)
        .iter()
        .zip(edges(outer))
        .all(|(a, b)| (a - b).abs() <= COVER_SLACK)
}

/// Whether a hold whose `receivers` are only the target's own hidden field
/// still reaches `target`: it does when `target` observes raw touches. The
/// platform gives the hold to the field (its native text selection), while
/// the touches still travel to every touch observer on their path, so a
/// view covered by its own field (a one-time-code box) hears the held
/// finger and can act on it. "Its own" is strict: an enabled input inside
/// the target whose native frame covers the target's (`bounds`) within a
/// point. A visible field somewhere inside a card, an input that is a
/// sibling or an overlay, and a target that only has a long-press
/// recognizer (it never hears a hold its input takes) all stay occluded.
fn hold_reaches_through_own_input(
    target: &SemanticNode,
    bounds: Bounds<Points>,
    receivers: &[NodeId],
    by_id: &std::collections::HashMap<NodeId, &SemanticNode>,
    parents: &std::collections::HashMap<NodeId, Option<NodeId>>,
    native_bounds: impl Fn(NodeId) -> Option<Bounds<Points>>,
) -> bool {
    target.touch
        && !receivers.is_empty()
        && receivers.iter().all(|&receiver| {
            receiver != target.id
                && by_id
                    .get(&receiver)
                    .is_some_and(|node| node.input && node.enabled)
                && is_descendant_or_same(receiver, target.id, parents)
                && native_bounds(receiver).is_some_and(|field| covers(field, bounds))
        })
}

/// Whether `keys` (one keystroke in `nsh/keymap` notation, `mod+v`) is a
/// paste chord on some platform, read as the crate reads every chord
/// ([`normalize_chord`]: `⌘`, `cmd`, `super`, `windows` are `meta`; `⌃`,
/// `ctl`, `control` are `ctrl`; `⇧` is `shift`; `insert` is `ins`; any case,
/// `+` or `-`): V with `meta`, `ctrl` or `mod` (Shift or Alt besides, as
/// paste-as-plain-text has), Shift+Insert, or a paste key of its own
/// (Android's `KEYCODE_PASTE`, X11's `XF86Paste`).
///
/// Only chords are gated. A desktop field's own context menu reached by a
/// key sequence (the Menu key, then Paste) is no chord and is not gated; no
/// recipe drives one.
pub fn is_paste_chord(keys: &str) -> bool {
    let parts = normalize_chord(keys);
    let has = |name: &str| parts.iter().any(|part| part == name);
    let paste_key = parts
        .iter()
        .any(|part| matches!(part.as_str(), "paste" | "xf86paste" | "keycode_paste"));
    paste_key
        || (has("v") && (has("meta") || has("ctrl") || has("mod")))
        || (has("ins") && has("shift"))
}

/// The platform's own text menu showing over an input, as read from the
/// platform (read-only: the agent does not press its items).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TextMenu {
    /// Which menu: [`TextMenu::SELECTION`], [`TextMenu::INSERTION`] or
    /// [`TextMenu::EDIT`].
    pub kind: &'static str,
    /// The input it is over, when the platform can tell.
    pub field: Option<NodeId>,
    /// A fixed word for each visible item, in order (`cut`, `copy`, `paste`,
    /// `select-all`, …, `other` for an item the platform does not define):
    /// never the item's own title, which other apps may fill freely. None
    /// for a masked field.
    pub items: Vec<String>,
}

impl TextMenu {
    /// The field's own toolbar over a selection it holds; the platform
    /// draws the selection's handles with it.
    pub const SELECTION: &'static str = "selection";
    /// The field's own toolbar at its caret (Paste, Select all); the
    /// platform draws the caret's handle with it.
    pub const INSERTION: &'static str = "insertion";
    /// A toolbar the app opened over a selection it draws itself.
    pub const EDIT: &'static str = "edit";

    /// The items the agent presses: those whose effect stays inside the
    /// app and the clipboard. Share, assist, autofill and any item the
    /// platform does not define (`other`) leave the app and are never
    /// pressed.
    pub const PRESSABLE: [&'static str; 4] = ["cut", "copy", "paste", "select-all"];

    /// Whether the agent presses item `word` ([`TextMenu::PRESSABLE`]).
    pub fn pressable(word: &str) -> bool {
        Self::PRESSABLE.contains(&word)
    }

    /// The menu a host's bridge answered as `[kind, field, item, item, …]`
    /// (`field` the input's node id, empty when it is not known): none for
    /// an answer without both, a kind it does not know or a field that is
    /// no node id, so a bridge fault shows no menu rather than a wrong one.
    pub fn from_strings(values: &[String]) -> Option<TextMenu> {
        let [kind, field, items @ ..] = values else {
            return None;
        };
        let kind = [Self::SELECTION, Self::INSERTION, Self::EDIT]
            .into_iter()
            .find(|known| *known == kind.as_str())?;
        let field = match field.as_str() {
            "" => None,
            id => Some(id.parse::<NodeId>().ok()?),
        };
        Some(TextMenu {
            kind,
            field,
            items: items.to_vec(),
        })
    }
}

/// The selections a host's bridge answered as flat `(id, start, end)`
/// triples for the inputs `asked`. Strict, so a bridge fault never moves
/// one field's selection onto another: anything but whole triples gives
/// nothing at all, an id not asked for or answered twice is dropped (both
/// copies of a duplicate), and so is a triple whose offsets are negative or
/// whose end is before its start.
pub fn selections_from_triples(
    asked: &[NodeId],
    values: &[i32],
) -> std::collections::HashMap<NodeId, (u32, u32)> {
    let mut out = std::collections::HashMap::new();
    if !values.len().is_multiple_of(3) {
        return out;
    }
    let mut seen = std::collections::HashSet::new();
    let mut twice = std::collections::HashSet::new();
    for &[id, start, end] in values.as_chunks::<3>().0 {
        let Ok(id) = NodeId::try_from(id) else {
            continue;
        };
        if !asked.contains(&id) {
            continue;
        }
        if !seen.insert(id) {
            twice.insert(id);
            continue;
        }
        if start >= 0 && end >= start {
            out.insert(id, (start as u32, end as u32));
        }
    }
    out.retain(|id, _| !twice.contains(id));
    out
}

/// Acts on a host's app through its [`SemanticScene`] and [`StrictNative`].
pub struct Agent {
    scene: std::sync::Arc<dyn SemanticScene>,
    native: Box<dyn StrictNative>,
    require_hit_test: bool,
    keyboard: std::sync::Mutex<KeyboardTrack>,
}

/// How long the software keyboard's frame must stay the same before it
/// counts: its slide in or out takes about 250 ms, and the platforms report
/// the end frame as the slide starts.
pub const KEYBOARD_SETTLE: Duration = Duration::from_millis(300);

/// How much of a target must show above the software keyboard, in points,
/// for a press on that part (all of a target shorter than this).
const KEYBOARD_MIN_VISIBLE: f32 = 8.0;

/// The longest [`Agent::keyboard_frame`] waits for a moving keyboard.
const KEYBOARD_SETTLE_MAX: Duration = Duration::from_millis(1500);

/// The keyboard frames the platform reported, and the one that settled.
#[derive(Debug, Default)]
struct KeyboardTrack {
    /// The last frame read (`None` inside: no keyboard).
    last: Option<Option<Bounds<Points>>>,
    /// When `last` was first read.
    since: Option<Instant>,
    /// The last frame that stayed the same for [`KEYBOARD_SETTLE`].
    settled: Option<Bounds<Points>>,
}

impl KeyboardTrack {
    /// Takes the frame read at `now`; `None` when it has settled (then
    /// [`KeyboardTrack::settled`] is it), else when it will have.
    fn observe(&mut self, read: Option<Bounds<Points>>, now: Instant) -> Option<Instant> {
        // No keyboard at the first look (a desktop, a phone with none up):
        // nothing to wait for.
        if self.last.is_none() && read.is_none() {
            self.last = Some(None);
            self.since = Some(now);
            self.settled = None;
            return None;
        }
        if self.last != Some(read) {
            self.last = Some(read);
            self.since = Some(now);
        }
        let settles = self.since.unwrap_or(now) + KEYBOARD_SETTLE;
        if now >= settles {
            self.settled = read;
            None
        } else {
            Some(settles)
        }
    }
}

impl Agent {
    /// An agent on the app `scene` sees, acting through `native`.
    pub fn new(scene: std::sync::Arc<dyn SemanticScene>, native: Box<dyn StrictNative>) -> Agent {
        Agent {
            scene,
            native,
            require_hit_test: false,
            keyboard: std::sync::Mutex::default(),
        }
    }

    /// Creates an agent that requires native hit-testing before targeted input.
    pub fn requiring_native_hit_test(
        scene: std::sync::Arc<dyn SemanticScene>,
        native: Box<dyn StrictNative>,
    ) -> Agent {
        Agent {
            scene,
            native,
            require_hit_test: true,
            keyboard: std::sync::Mutex::default(),
        }
    }

    /// Semantic nodes currently exposed by the scene.
    pub fn semantic_nodes(&self) -> Result<Vec<SemanticNode>, String> {
        self.scene.semantic_tree()
    }

    /// Whether the native scene has applied its first rendered batch.
    pub fn has_rendered_batch(&self) -> Result<bool, String> {
        Ok(self.scene.activity()?.batches > 0)
    }

    /// Whether this agent's native surface is still available.
    pub fn native_available(&self) -> bool {
        self.native.available()
    }

    /// Waits for the current scene to settle before a host resolves a target.
    pub fn settle(&self) -> Result<(), String> {
        self.settled()
    }

    /// Current native bounds for `id`. Builds the semantic tree: to look
    /// up many nodes, take it once ([`Agent::semantic_nodes`]) and use
    /// [`Agent::node_native_bounds`].
    pub fn native_bounds(&self, id: NodeId) -> Option<Bounds<Points>> {
        let Ok(nodes) = self.scene.semantic_tree() else {
            return self.native.bounds(id);
        };
        let Some(node) = nodes.iter().find(|node| node.id == id) else {
            return self.native.bounds(id);
        };
        self.node_native_bounds(node)
    }

    /// Current native bounds for `node`, a node of a semantic tree the
    /// caller already has: a canvas or text target's own part of its
    /// owner's view.
    pub fn node_native_bounds(&self, node: &SemanticNode) -> Option<Bounds<Points>> {
        self.target_geometry(node).map(|geometry| geometry.bounds)
    }

    /// Current native viewport.
    pub fn native_viewport(&self) -> Size<Points> {
        self.native.viewport()
    }

    /// Whether the window shows the app laid out at its current size
    /// ([`Native::laid_out`]).
    pub fn native_laid_out(&self) -> Result<(), String> {
        self.native.laid_out()
    }

    /// [`Native::take_failure`].
    pub fn take_native_failure(&self) -> Option<String> {
        self.native.take_failure()
    }

    /// Whether the native app window currently has focus.
    pub fn app_focused(&self) -> bool {
        self.native.app_focused()
    }

    /// Current native input value for `id`.
    pub fn native_input_text(&self, id: NodeId) -> Option<String> {
        self.native.input_text(id)
    }

    /// The platform's own text menu showing now ([`StrictNative::text_menu`]).
    pub fn native_text_menu(&self) -> Option<TextMenu> {
        self.native.text_menu()
    }

    /// Presses item `word` of the platform's own text menu showing now
    /// ([`StrictNative::press_text_menu_item`]): only an item of
    /// [`TextMenu::PRESSABLE`], and only one the showing menu has.
    pub fn press_text_menu_item(&self, word: &str) -> Result<(), String> {
        if !TextMenu::pressable(word) {
            return Err(format!(
                "`{word}` is not pressed: only {} of the text menu stay inside the app",
                TextMenu::PRESSABLE.join(", ")
            ));
        }
        let menu = self
            .native
            .text_menu()
            .ok_or_else(|| "no text menu shows".to_string())?;
        if !menu.items.iter().any(|item| item == word) {
            return Err(format!(
                "the {} text menu has no `{word}` (it has: {})",
                menu.kind,
                if menu.items.is_empty() {
                    "nothing it reports".to_owned()
                } else {
                    menu.items.join(", ")
                }
            ));
        }
        // The device's clipboard holds whatever a person or another app
        // copied (codes, passwords, addresses), and a pasted field is read
        // back unmasked into proof: only a clip this run set is pasted.
        if word == "paste" && self.native.clipboard_set_by_agent() != Some(true) {
            return Err(
                "the clipboard was not set by this run: Paste would put in what a person or \
                 another app copied (cut or copy through the text menu first)"
                    .into(),
            );
        }
        self.native.press_text_menu_item(word)?;
        self.settled()
    }

    /// The selections of inputs `ids` as the platform holds them
    /// ([`StrictNative::input_selections`]).
    pub fn native_input_selections(
        &self,
        ids: &[NodeId],
    ) -> std::collections::HashMap<NodeId, (u32, u32)> {
        self.native.input_selections(ids)
    }

    fn target_geometry(&self, node: &SemanticNode) -> Option<TargetGeometry> {
        match &node.native_target {
            SemanticTarget::View => Some(TargetGeometry {
                owner: node.id,
                bounds: self.native.bounds(node.id)?,
                synthetic: false,
            }),
            SemanticTarget::Canvas { owner, bounds } => {
                let owner_bounds = self.native.bounds(*owner)?;
                Some(TargetGeometry {
                    owner: *owner,
                    bounds: Bounds::new(owner_bounds.origin + bounds.origin, bounds.size),
                    synthetic: true,
                })
            }
            SemanticTarget::Text { owner, start, end } => Some(TargetGeometry {
                owner: *owner,
                bounds: self
                    .native
                    .text_bounds(*owner, *start, *end)
                    .filter(|b| !b.is_empty())?,
                synthetic: true,
            }),
        }
    }

    fn target_point(&self, id: NodeId, selector: &str) -> Result<Point<Points>, String> {
        let nodes = self.scene.semantic_tree()?;
        let node = nodes
            .iter()
            .find(|node| node.id == id)
            .ok_or_else(|| format!("`{selector}` is offscreen (node unavailable)"))?;
        self.target_point_for_node(node, selector)
    }

    fn target_point_for_node(
        &self,
        node: &SemanticNode,
        selector: &str,
    ) -> Result<Point<Points>, String> {
        self.target_node_window(node.id)?;
        match &node.native_target {
            SemanticTarget::View => self.scroll_into_view(node.id, selector),
            SemanticTarget::Canvas { owner, bounds } => {
                self.scroll_point_into_view(*owner, selector, Some(bounds.center()))?;
                self.target_geometry(node)
                    .map(|geometry| geometry.bounds.center())
                    .ok_or_else(|| format!("`{selector}` is offscreen (no native bounds)"))
            }
            SemanticTarget::Text { owner, .. } => {
                self.scroll_into_view(*owner, selector)?;
                self.target_geometry(node)
                    .map(|geometry| geometry.bounds.center())
                    .ok_or_else(|| format!("`{selector}` is offscreen (no native text bounds)"))
            }
        }
    }

    /// Captures the current surface as actual PNG bytes.
    pub fn native_screenshot(&self) -> Result<Vec<u8>, String> {
        self.native.screenshot()
    }

    /// Whether actions require native hit-test proof before dispatch.
    pub fn requires_native_hit_test(&self) -> bool {
        self.require_hit_test
    }

    /// Clicks semantic node `id`, after native actionability is established.
    pub fn press_node(&self, id: NodeId) -> Result<(), String> {
        self.settled_and_keyboard()?;
        let selector = format!("node {id}");
        let nodes = self.scene.semantic_tree()?;
        let target = nodes
            .iter()
            .find(|node| node.id == id)
            .ok_or_else(|| format!("`{selector}` is offscreen (node unavailable)"))?;
        if !target.enabled {
            return Err(format!("`{selector}` is disabled"));
        }
        let owner =
            press_owner(&nodes, target).ok_or_else(|| format!("`{selector}` is not pressable"))?;
        let selected = self.target_point(id, &selector)?;
        let geometry = self
            .target_geometry(target)
            .ok_or_else(|| format!("`{selector}` is offscreen (no native bounds)"))?;
        let at = self.actionable_point_with_press_owner(
            id,
            geometry.owner,
            selected,
            &selector,
            Some(owner),
            false,
            false,
        )?;
        self.native.press(at);
        self.settled()
    }

    /// Clicks semantic node `id` at `within`, a point in the node's own
    /// frame (from its top-left corner, in points): one cell of a row of
    /// cells that is one node. The point must be inside the part of the
    /// node on screen, and the press passes the same actionability check
    /// as [`Agent::press_node`] there; it is never moved.
    pub fn press_node_within(&self, id: NodeId, within: Point<Points>) -> Result<(), String> {
        self.settled_and_keyboard()?;
        let selector = format!("node {id}");
        let nodes = self.scene.semantic_tree()?;
        let target = nodes
            .iter()
            .find(|node| node.id == id)
            .ok_or_else(|| format!("`{selector}` is offscreen (node unavailable)"))?;
        if !target.enabled {
            return Err(format!("`{selector}` is disabled"));
        }
        let owner =
            press_owner(&nodes, target).ok_or_else(|| format!("`{selector}` is not pressable"))?;
        let geometry = self
            .target_geometry(target)
            .ok_or_else(|| format!("`{selector}` is offscreen (no native bounds)"))?;
        let at = point_within(geometry.bounds, within, &selector)?;
        let at = self.actionable_point_with_press_owner(
            id,
            geometry.owner,
            at,
            &selector,
            Some(owner),
            true,
            false,
        )?;
        self.native.press(at);
        self.settled()
    }

    /// Presses and holds semantic node `id`, after long-press actionability is
    /// established.
    pub fn long_press_node(&self, id: NodeId, hold: Option<Duration>) -> Result<(), String> {
        self.long_press(id, hold)
    }

    /// Presses and holds semantic node `id` at `within`, a point in the
    /// node's own frame (from its top-left corner, in points): one cell of
    /// a row of cells that is one node. The point must be inside the part
    /// of the node on screen, and the hold passes the same actionability
    /// check as [`Agent::long_press_node`] there; it is never moved.
    pub fn long_press_node_within(
        &self,
        id: NodeId,
        within: Point<Points>,
        hold: Option<Duration>,
    ) -> Result<(), String> {
        self.settled_and_keyboard()?;
        let selector = &format!("view:{id}");
        self.settled()?;
        let target = match self.long_pressable(id)? {
            Pick::Disabled => {
                return self
                    .picked(Pick::Disabled, selector, "pressable view")
                    .map(drop);
            }
            Pick::None => match self.scene.pick(selector, PickTarget::Input)? {
                Pick::One(_) => PickTarget::Input,
                _ => PickTarget::LongPressable,
            },
            _ => PickTarget::LongPressable,
        };
        // Brings the node into view as a targeted hold does.
        let (id, _) = self.find(selector, "pressable view", target)?;
        let nodes = self.scene.semantic_tree()?;
        let node = nodes
            .iter()
            .find(|node| node.id == id)
            .ok_or_else(|| format!("`{selector}` is offscreen (node unavailable)"))?;
        if !node.enabled {
            return Err(format!("`{selector}` is disabled"));
        }
        let geometry = self
            .target_geometry(node)
            .ok_or_else(|| format!("`{selector}` is offscreen (no native bounds)"))?;
        let at = point_within(geometry.bounds, within, selector)?;
        let at = self.actionable_point_with_press_owner(
            id,
            geometry.owner,
            at,
            selector,
            Some(id),
            true,
            true,
        )?;
        self.hold_long_press(id, at, hold)
    }

    /// Presses and holds a checked native point.
    pub fn long_press_at(&self, at: Point<Points>, hold: Option<Duration>) -> Result<(), String> {
        let (owner, at) = self.checked_point_target(at, PointActionability::LongPress)?;
        self.hold_long_press(owner, at, hold)
    }

    /// Pastes `text` into input node `id`, focusing it first when needed.
    pub fn paste_node(&self, id: NodeId, text: &str) -> Result<(), String> {
        self.paste(&format!("view:{id}"), &nsh_core::PasteEvent::new(text))
    }

    /// The system's back gesture or button, on platforms that have one.
    pub fn back(&self) -> Result<(), String> {
        self.native.back().and_then(|()| self.settled())
    }

    /// Requests portrait or landscape orientation.
    pub fn rotate(&self, landscape: bool) -> Result<(), String> {
        self.native.rotate(landscape).and_then(|()| self.settled())
    }

    /// Where the software keyboard covers the surface once it has stopped
    /// moving ([`StrictNative::keyboard_frame`]): a frame counts when it
    /// has stayed the same for [`KEYBOARD_SETTLE`], so while the keyboard
    /// slides in this waits for it to arrive, and while it slides out it
    /// still covers where it was. Waits 1.5 s at most;
    /// a frame that never settles counts as it was last read.
    pub fn keyboard_frame(&self) -> Result<Option<Bounds<Points>>, String> {
        let deadline = Instant::now() + KEYBOARD_SETTLE_MAX;
        loop {
            let read = self.native.keyboard_frame()?;
            let now = Instant::now();
            let mut track = self.keyboard.lock().unwrap_or_else(|e| e.into_inner());
            let Some(settles) = track.observe(read, now) else {
                return Ok(track.settled);
            };
            drop(track);
            if now >= deadline {
                return Ok(read);
            }
            while Instant::now() < settles {
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }

    /// [`Agent::settled`], then the software keyboard settled too
    /// ([`Agent::keyboard_frame`]): a step reads the tree only once the
    /// keyboard has stopped moving, so a control `adjustResize` moved
    /// above the keyboard is where the tree says.
    fn settled_and_keyboard(&self) -> Result<(), String> {
        self.settled()?;
        let _ = self.keyboard_frame();
        Ok(())
    }

    /// Whether the platform can tell where the software keyboard is.
    pub fn keyboard_known(&self) -> bool {
        self.native.keyboard_frame().is_ok()
    }

    /// Shows or hides the software keyboard.
    pub fn keyboard(&self, shown: bool) -> Result<(), String> {
        self.native.keyboard(shown).and_then(|()| self.settled())
    }

    /// Types `text` into input `id`, focusing it first when needed, and waits for the resulting state.
    pub fn type_node(&self, id: NodeId, text: &str) -> Result<(), String> {
        self.settled()?;
        let node = self
            .scene
            .semantic_tree()?
            .into_iter()
            .find(|node| node.id == id)
            .ok_or_else(|| format!("node {id} is offscreen or unavailable"))?;
        if !node.input {
            return Err(format!("node {id} is not a text input"));
        }
        if !node.enabled {
            return Err(format!("node {id} is disabled"));
        }
        // An input that has the focus takes the keys where it is, even
        // under the software keyboard (a person types into it there too):
        // only a press to focus it needs a point that is clear.
        if self.native.focused() != Some(id) {
            self.press_to_focus(id, &node)?;
        }
        self.native.type_text(text);
        self.settled()
    }

    /// Presses input `id` (`node`) to give it the focus. An input under an
    /// overlay of its own component (NSH's `OtpInput` / pin cells: the cells
    /// over the text field that takes the keys, under the same parent) is
    /// pressed on the overlay, as a finger does, only when nothing there
    /// takes the press itself ([`Agent::overlay_passes`]); the point is
    /// re-hit-tested, and the press must then have given the input the
    /// focus. Any other occluder refuses the press.
    fn press_to_focus(&self, id: NodeId, node: &SemanticNode) -> Result<(), String> {
        let error = match self.node_point(id) {
            Ok(at) => {
                self.native.press(at);
                return Ok(());
            }
            Err(error) => error,
        };
        let Some(hit) = error
            .strip_prefix("occluded_by: ")
            .and_then(|rest| rest.parse::<NodeId>().ok())
        else {
            return Err(error);
        };
        let Some(component) = node.parent else {
            return Err(error);
        };
        let nodes = self.scene.semantic_tree()?;
        let by_id: std::collections::HashMap<NodeId, &SemanticNode> =
            nodes.iter().map(|node| (node.id, node)).collect();
        // The input's visible part: the viewport, its bounds and every
        // clipping view around it.
        let viewport = Bounds::new(Point::default(), self.native.viewport());
        let Some(mut visible) = self
            .native
            .bounds(id)
            .and_then(|bounds| intersection(viewport, bounds))
        else {
            return Err(error);
        };
        let mut parent = if node.modal { None } else { node.parent };
        while let Some(ancestor) = parent.and_then(|parent| by_id.get(&parent)) {
            if ancestor.clipping {
                let Some(clipped) = self
                    .native
                    .bounds(ancestor.id)
                    .and_then(|bounds| intersection(visible, bounds))
                else {
                    return Err(error);
                };
                visible = clipped;
            }
            if ancestor.modal {
                break;
            }
            parent = ancestor.parent;
        }
        let at = self.clear_of_keyboard(visible, visible.center(), false, &format!("node {id}"))?;
        if !self.overlay_passes(&nodes, hit, id, component, at) {
            return Err(error);
        }
        // What is under the point now is what the press goes to.
        match self.native.hit_at(at)? {
            Some(now) if now == id || self.overlay_passes(&nodes, now, id, component, at) => {}
            Some(now) => return Err(format!("occluded_by: {now}")),
            None => return Err(error),
        }
        self.native.press(at);
        self.settled()?;
        if self.native.focused() == Some(id) {
            Ok(())
        } else {
            Err(format!(
                "{error}: a press on its overlay at ({}, {}) did not give node {id} the focus",
                at.x.0, at.y.0
            ))
        }
    }

    /// Whether a press at `at` that natively hits `hit` passes through to
    /// input `id`: `hit` is inside `component` (the input's parent), no
    /// view from `hit` up to the component takes presses or holds itself or
    /// is another input, and the press goes to no receiver but the input.
    /// A touch observer (the cells of a code box) passes.
    fn overlay_passes(
        &self,
        nodes: &[SemanticNode],
        hit: NodeId,
        id: NodeId,
        component: NodeId,
        at: Point<Points>,
    ) -> bool {
        let parents = mounted_parents(nodes);
        let by_id: std::collections::HashMap<NodeId, &SemanticNode> =
            nodes.iter().map(|node| (node.id, node)).collect();
        let hidden_press_owners: std::collections::HashMap<NodeId, NodeId> = nodes
            .iter()
            .flat_map(|node| node.hidden_press_owners.iter().copied())
            .collect();
        if hidden_press_owners.contains_key(&hit) {
            return false;
        }
        let mut current = Some(hit);
        let mut inside = false;
        while let Some(view) = current {
            if view == component {
                inside = true;
                break;
            }
            if let Some(node) = by_id.get(&view)
                && (node.pressable || node.long_press || (node.input && node.id != id))
            {
                return false;
            }
            current = parents.get(&view).copied().flatten();
        }
        if !inside {
            return false;
        }
        match self.press_receiver(nodes, &by_id, &parents, &hidden_press_owners, hit, at) {
            None => true,
            Some(receiver) => receiver == id,
        }
    }

    /// Replaces the value of input `id`.
    pub fn set_value_node(&self, id: NodeId, value: &str) -> Result<(), String> {
        self.settled()?;
        let node = self
            .scene
            .semantic_tree()?
            .into_iter()
            .find(|node| node.id == id)
            .ok_or_else(|| format!("node {id} is offscreen or unavailable"))?;
        if !node.input {
            return Err(format!("node {id} is not a text input"));
        }
        if !node.enabled {
            return Err(format!("node {id} is disabled"));
        }
        if self.native.focused() != Some(id) {
            self.press_to_focus(id, &node)?;
        }
        self.native.key(Key::SelectAll);
        self.native.key(Key::Backspace);
        self.native.type_text(value);
        self.settled()
    }

    /// Sends a hardware key chord after verifying the target.
    pub fn key_node(&self, id: NodeId, keys: &str) -> Result<(), String> {
        // A paste chord pastes the system clipboard, which can hold what a
        // person or another app copied (codes, passwords, addresses), into a
        // field read back unmasked into proof: only a clip this run set.
        if is_paste_chord(keys) && self.native.clipboard_set_by_agent() != Some(true) {
            return Err(format!(
                "`{keys}` pastes the system clipboard, which was not set by this run: give the \
                 text with the paste op instead (it pastes that text, not the clipboard)"
            ));
        }
        self.settled()?;
        let node = self
            .scene
            .semantic_tree()?
            .into_iter()
            .find(|node| node.id == id)
            .ok_or_else(|| format!("node {id} is offscreen or unavailable"))?;
        if !node.enabled {
            return Err(format!("node {id} is disabled"));
        }
        self.native.validate_keys(keys)?;
        // A focused input takes the keys where it is (see `type_node`).
        if !(node.input && self.native.focused() == Some(id)) {
            let at = self.node_point(id)?;
            if let Err(error) = self.native.focus_node(id) {
                if node.input && error == NATIVE_FOCUS_UNSUPPORTED {
                    self.native.press(at);
                } else {
                    return Err(error);
                }
            }
        }
        self.native.press_keys(keys)?;
        self.settled()
    }

    /// Returns the app's current keymap and visible menu shortcuts.
    pub fn semantic_keybindings(&self) -> Result<Vec<SemanticKeybinding>, String> {
        self.scene.keybindings()
    }

    /// Fires a catalogued shortcut using the platform's native dispatch.
    /// A chord another binding also claims can only be sent when the
    /// platform fires bindings by identity: a plain chord could dispatch
    /// to that other command.
    pub fn fire_keybinding(&self, binding: &str) -> Result<(), String> {
        self.settled()?;
        let bindings = self.scene.keybindings()?;
        let entry = bindings
            .iter()
            .find(|entry| entry.id == binding)
            .ok_or_else(|| format!("unknown binding `{binding}`"))?;
        if entry.menu_path.is_some() {
            self.native.fire_menu_keybinding(entry)?;
        } else if bindings.iter().any(|other| {
            other.id != entry.id && normalize_chord(&other.chord) == normalize_chord(&entry.chord)
        }) {
            self.native.fire_keybinding(entry)?;
        } else {
            self.native.press_keys(&entry.chord)?;
        }
        self.settled()
    }

    /// Scrolls the native content at semantic node `id`, refusing a
    /// locked scroller: native wheel routing would skip it and move an
    /// eligible ancestor instead of the named view.
    pub fn scroll_node(&self, id: NodeId, delta: Point<Points>) -> Result<(), String> {
        match self
            .scene
            .pick(&format!("view:{id}"), PickTarget::ScrollView)?
        {
            Pick::One(receiver) if receiver == id => {}
            Pick::One(receiver) => {
                return Err(format!(
                    "`view:{id}` does not scroll (`view:{receiver}` would take its scrolls)"
                ));
            }
            Pick::Disabled => {
                return Err(format!("`view:{id}` is a scroll view that does not scroll"));
            }
            Pick::Many(n) => return Err(format!("`view:{id}` matches {n} scroll views")),
            Pick::None => {}
        }
        let at = self.node_point(id)?;
        self.native.scroll(at, delta);
        self.settled()
    }

    /// Hovers semantic node `id`, failing when this native platform has no mouse.
    pub fn hover_node(&self, id: NodeId) -> Result<(), String> {
        let at = self.node_point(id)?;
        self.native.hover(at)?;
        self.settled()
    }

    /// Presses the semantic node hit by a checked native point.
    pub fn press_at(&self, at: Point<Points>) -> Result<(), String> {
        let (_, at) = self.checked_point_target(at, PointActionability::Press)?;
        self.native.press(at);
        self.settled()
    }

    /// Hovers the semantic node hit by a checked native point.
    pub fn hover_at(&self, at: Point<Points>) -> Result<(), String> {
        let (_, at) = self.checked_point_target(at, PointActionability::Any)?;
        self.native.hover(at)?;
        self.settled()
    }

    /// Scrolls the semantic node hit by a checked native point.
    pub fn scroll_at(&self, at: Point<Points>, delta: Point<Points>) -> Result<(), String> {
        let (_, at) = self.checked_point_target(at, PointActionability::Any)?;
        self.native.scroll(at, delta);
        self.settled()
    }

    /// Drags from a checked native point to an on-screen point.
    pub fn drag_at(&self, from: Point<Points>, to: Point<Points>) -> Result<(), String> {
        let (_, from) = self.checked_point_target(from, PointActionability::Any)?;
        // The caller's absolute endpoint stays exact; a delta round-trip
        // could round it onto the viewport edge and fail the check.
        self.drag_checked(from, to)
    }

    /// Drags a checked node `id` `distance` points in `direction`: the
    /// default, 40% of the viewport, only as far as fits from the checked
    /// origin.
    pub fn swipe_node(
        &self,
        id: NodeId,
        direction: SwipeDirection,
        distance: Option<f32>,
    ) -> Result<(), String> {
        let selector = format!("node {id}");
        let _ = self.node_point(id)?;
        self.settled()?;
        let origin = self.node_point_without_reveal(id, &selector)?;
        let delta = self.swipe_delta(origin, direction, distance)?;
        self.drag_checked(origin, origin + delta)
    }

    /// [`Agent::swipe_node`] for a swipe starting at coordinate `from`.
    pub fn swipe_at(
        &self,
        from: Point<Points>,
        direction: SwipeDirection,
        distance: Option<f32>,
    ) -> Result<(), String> {
        let (_, origin) = self.checked_point_target(from, PointActionability::Any)?;
        let delta = self.swipe_delta(origin, direction, distance)?;
        self.drag_checked(origin, origin + delta)
    }

    /// The swipe's displacement: `distance` when given, else 40% of the
    /// viewport clamped to the room `origin` leaves in `direction`, so the
    /// endpoint stays inside the surface.
    fn swipe_delta(
        &self,
        origin: Point<Points>,
        direction: SwipeDirection,
        distance: Option<f32>,
    ) -> Result<Point<Points>, String> {
        let viewport = self.native.viewport();
        let (width, height) = (viewport.width.0, viewport.height.0);
        let horizontal = matches!(direction, SwipeDirection::Left | SwipeDirection::Right);
        let distance = match distance {
            Some(distance) if distance.is_finite() && distance > 0.0 => distance,
            Some(_) => return Err("swipe distance must be finite and positive".into()),
            None => {
                let base = (if horizontal { width } else { height }) * 0.4;
                // One point shy of the edge: the endpoint must stay inside
                // the surface even when `origin` nearly touches it.
                let room = match direction {
                    SwipeDirection::Up => origin.y.0,
                    SwipeDirection::Down => height - origin.y.0,
                    SwipeDirection::Left => origin.x.0,
                    SwipeDirection::Right => width - origin.x.0,
                } - 1.0;
                if room <= 0.0 {
                    return Err("no room to swipe: the edge is already at hand".into());
                }
                base.min(room)
            }
        };
        Ok(match direction {
            SwipeDirection::Up => Point::new(Points(0.0), Points(-distance)),
            SwipeDirection::Down => Point::new(Points(0.0), Points(distance)),
            SwipeDirection::Left => Point::new(Points(-distance), Points(0.0)),
            SwipeDirection::Right => Point::new(Points(distance), Points(0.0)),
        })
    }

    /// Drags from `origin` to `to`, refusing an offscreen or
    /// untargetable destination.
    fn drag_checked(&self, origin: Point<Points>, to: Point<Points>) -> Result<(), String> {
        let viewport = self.native.viewport();
        if to.x.0 < 0.0 || to.y.0 < 0.0 || to.x.0 >= viewport.width.0 || to.y.0 >= viewport.height.0
        {
            return Err("drag destination is offscreen".into());
        }
        if self.native.hit_at(to)?.is_none() {
            return Err("drag destination is offscreen or unavailable".into());
        }
        self.native.drag(origin, to);
        self.settled()
    }

    /// Drags from one checked semantic node to another.
    pub fn drag_nodes(&self, from: NodeId, to: NodeId) -> Result<(), String> {
        let nodes = self.scene.semantic_tree()?;
        for id in [from, to] {
            if !nodes.iter().any(|node| node.id == id) {
                return Err(format!("`node {id}` is offscreen (node unavailable)"));
            }
        }
        if self.scene.window_of(from)? != self.scene.window_of(to)? {
            return Err("drag endpoints are in different windows".into());
        }
        let from_selector = format!("node {from}");
        let to_selector = format!("node {to}");
        let _ = self.node_point(from)?;
        let _ = self.node_point(to)?;
        self.settled()?;
        let from = self.node_point_without_reveal(from, &from_selector)?;
        let to = self.node_point_without_reveal(to, &to_selector)?;
        self.native.drag(from, to);
        self.settled()
    }

    /// Drags a checked node to an on-screen native drop point.
    pub fn drag_to(&self, from: NodeId, to: Point<Points>) -> Result<(), String> {
        let selector = format!("node {from}");
        let _ = self.node_point(from)?;
        self.settled()?;
        let origin = self.node_point_without_reveal(from, &selector)?;
        let viewport = self.native.viewport();
        if to.x.0 < 0.0 || to.y.0 < 0.0 || to.x.0 >= viewport.width.0 || to.y.0 >= viewport.height.0
        {
            return Err("drag destination is offscreen".into());
        }
        if self.native.hit_at(to)?.is_none() {
            return Err("drag destination is offscreen or unavailable".into());
        }
        self.native.drag(origin, to);
        self.settled()
    }

    /// Drags checked node `id` by `delta`: from `from` (in the window's
    /// content, an error when outside the node's visible area), or from
    /// the middle of its visible area when `None`.
    pub fn drag_node_by(
        &self,
        id: NodeId,
        from: Option<Point<Points>>,
        delta: Point<Points>,
    ) -> Result<(), String> {
        let selector = format!("node {id}");
        let origin = match from {
            Some(from) => {
                // The client picked the origin: check it against the node's
                // visible bounds and the native hit, without first requiring
                // a hit at the node's own point (something may cover its
                // middle while its edge stays draggable).
                self.settled()?;
                self.target_node_window(id)?;
                let node = self
                    .scene
                    .semantic_tree()?
                    .into_iter()
                    .find(|node| node.id == id)
                    .ok_or_else(|| format!("`{selector}` is offscreen (node unavailable)"))?;
                if !node.enabled {
                    return Err(format!("`{selector}` is disabled"));
                }
                let geometry = self
                    .target_geometry(&node)
                    .ok_or_else(|| format!("`{selector}` is offscreen (no native bounds)"))?;
                self.explicit_actionable_point(id, geometry.owner, from, &selector)?
            }
            None => {
                let _ = self.node_point(id)?;
                self.settled()?;
                self.node_point_without_reveal(id, &selector)?
            }
        };
        self.drag_checked(origin, origin + delta)
    }

    /// Resizes the native content viewport, rejecting invalid dimensions.
    pub fn resize_viewport(&self, width: Points, height: Points) -> Result<(), String> {
        if !width.0.is_finite() || width.0 <= 0.0 || !height.0.is_finite() || height.0 <= 0.0 {
            return Err("viewport width and height must be finite and positive".into());
        }
        self.native.resize(width, height)?;
        self.settled()
    }

    fn node_point(&self, id: NodeId) -> Result<Point<Points>, String> {
        self.settled_and_keyboard()?;
        let selector = format!("node {id}");
        let node = self
            .scene
            .semantic_tree()?
            .into_iter()
            .find(|node| node.id == id)
            .ok_or_else(|| format!("`{selector}` is offscreen (node unavailable)"))?;
        if !node.enabled {
            return Err(format!("`{selector}` is disabled"));
        }
        let at = self.target_point(id, &selector)?;
        let geometry = self
            .target_geometry(&node)
            .ok_or_else(|| format!("`{selector}` is offscreen (no native bounds)"))?;
        self.actionable_point(id, geometry.owner, at, &selector)
    }

    /// Checks that the native hit at `at` is enabled and supports `actionability`.
    fn checked_point_target(
        &self,
        at: Point<Points>,
        actionability: PointActionability,
    ) -> Result<(NodeId, Point<Points>), String> {
        self.settled_and_keyboard()?;
        let id = self
            .native
            .hit_at(at)?
            .ok_or("offscreen: coordinate has no native target")?;
        let nodes = self.scene.semantic_tree()?;
        let node = match nodes.iter().find(|node| node.id == id) {
            Some(node) => node,
            // A decorative accessibility-hidden view resolves to the
            // semantic node it is under: the point is inside that node too.
            // A hidden view that takes presses itself keeps its press.
            None => {
                let owner = nodes
                    .iter()
                    .find(|node| node.hidden_descendants.contains(&id))
                    .ok_or("target_gone: coordinate has no semantic target")?;
                let (gesture, owners) = match actionability {
                    PointActionability::Press => ("press", owner.hidden_press_owners.clone()),
                    // A hold's release is still a press: a hidden view
                    // hearing either gesture intercepts the hold, its own
                    // hold owner first when it listens for both.
                    PointActionability::LongPress => (
                        "hold",
                        owner
                            .hidden_hold_owners
                            .iter()
                            .chain(&owner.hidden_press_owners)
                            .copied()
                            .collect(),
                    ),
                    PointActionability::Any => ("press", owner.hidden_press_owners.clone()),
                };
                let receiver = owners
                    .iter()
                    .find(|(hidden, _)| *hidden == id)
                    .map(|(_, receiver)| *receiver)
                    .or_else(|| {
                        // A hit on a hidden text view reports the text
                        // view; the gesture can belong to its link.
                        owner
                            .hidden_text_spans
                            .iter()
                            .find(|(_, text, start, end)| {
                                *text == id
                                    && self
                                        .native
                                        .text_bounds(*text, *start, *end)
                                        .is_some_and(|bounds| bounds.contains(&at))
                            })
                            .and_then(|(link, ..)| {
                                owners
                                    .iter()
                                    .find(|(hidden, _)| hidden == link)
                                    .map(|(_, receiver)| *receiver)
                            })
                    });
                if !matches!(actionability, PointActionability::Any)
                    && let Some(receiver) = receiver
                {
                    return Err(format!(
                        "node {id} is accessibility-hidden: a {gesture} at ({}, {}) goes to \
                         node {receiver}, not to node {}",
                        at.x.0, at.y.0, owner.id
                    ));
                }
                owner
            }
        };
        if !node.enabled {
            return Err(format!("node {id} is disabled"));
        }
        let mut target = node.id;
        let action_target = match actionability {
            PointActionability::Any => id,
            PointActionability::Press => {
                if let Some(owner) = press_owner(&nodes, node) {
                    owner
                } else {
                    // A text view nothing pressable owns may still take the
                    // press in one of its spans (a link).
                    let link = self
                        .pressable_span_at(&nodes, id, at)
                        .ok_or_else(|| format!("node {id} is not pressable"))?;
                    if !link.enabled {
                        return Err(format!("node {} is disabled", link.id));
                    }
                    target = link.id;
                    link.id
                }
            }
            PointActionability::LongPress => {
                let selector = format!("view:{id}");
                // A hold on a link inside pressable text goes to the
                // span's receiver, as a tap would.
                if let Some((span, receiver)) = self.long_pressable_span_at(&nodes, id, at)? {
                    target = span;
                    receiver
                } else {
                    match self.long_pressable(id)? {
                        Pick::One(receiver) => receiver,
                        Pick::None => {
                            if let Some(input) = press_owner(&nodes, node).filter(|owner| {
                                nodes.iter().any(|candidate| {
                                    candidate.id == *owner && candidate.input && candidate.enabled
                                })
                            }) {
                                target = input;
                                input
                            } else {
                                self.picked(Pick::None, &selector, "pressable view")?
                            }
                        }
                        pick => self.picked(pick, &selector, "pressable view")?,
                    }
                }
            }
        };
        let bounds = self
            .native
            .bounds(id)
            .ok_or_else(|| format!("offscreen: node {id} has no native bounds"))?;
        if at.x.0 < bounds.origin.x.0
            || at.y.0 < bounds.origin.y.0
            || at.x.0 >= bounds.origin.x.0 + bounds.size.width.0
            || at.y.0 >= bounds.origin.y.0 + bounds.size.height.0
        {
            return Err(format!("offscreen: coordinate is outside node {id}"));
        }
        let selector = format!("node {target}");
        let at = if matches!(actionability, PointActionability::LongPress) {
            self.actionable_point_with_press_owner(
                target,
                id,
                at,
                &selector,
                Some(action_target),
                true,
                true,
            )?
        } else {
            self.explicit_actionable_point(target, id, at, &selector)?
        };
        Ok((action_target, at))
    }

    /// The pressable span of text view `text` whose characters are at
    /// `at` (on their first line, as [`Native::text_bounds`] gives them).
    /// Nested links can share the point: the innermost wins, as GPUI
    /// delivers the press to it.
    fn pressable_span_at<'a>(
        &self,
        nodes: &'a [SemanticNode],
        text: NodeId,
        at: Point<Points>,
    ) -> Option<&'a SemanticNode> {
        self.span_at(nodes, text, at, |node| node.pressable)
    }

    /// [`Agent::pressable_span_at`] matching spans that can take a hold —
    /// a link listening only for long presses still owns the gesture.
    fn holdable_span_at<'a>(
        &self,
        nodes: &'a [SemanticNode],
        text: NodeId,
        at: Point<Points>,
    ) -> Option<&'a SemanticNode> {
        self.span_at(nodes, text, at, |node| node.pressable || node.long_press)
    }

    /// The innermost span of text view `text` matching `hears` whose
    /// characters are at `at`: the smallest matching range wins, as
    /// `link_at` resolves it, with ties going to the span mounted first.
    fn span_at<'a>(
        &self,
        nodes: &'a [SemanticNode],
        text: NodeId,
        at: Point<Points>,
        hears: impl Fn(&SemanticNode) -> bool,
    ) -> Option<&'a SemanticNode> {
        nodes
            .iter()
            .filter(|node| {
                hears(node)
                    && matches!(node.native_target, SemanticTarget::Text { owner, start, end }
                    if owner == text
                        && self
                            .native
                            .text_bounds(owner, start, end)
                            .is_some_and(|bounds| bounds.contains(&at)))
            })
            .min_by_key(|node| match node.native_target {
                SemanticTarget::Text { start, end, .. } => end - start,
                _ => 0,
            })
    }

    /// The hidden view a gesture on hidden hit `hit` goes to: `hit`
    /// itself when `owners` names it, else the hidden link span of a
    /// hidden text hit at `at` — the native hit reports the text view
    /// while the gesture belongs to the link.
    fn hidden_owner_at(
        &self,
        nodes: &[SemanticNode],
        owners: &std::collections::HashMap<NodeId, NodeId>,
        hit: NodeId,
        at: Point<Points>,
    ) -> Option<NodeId> {
        if let Some(&receiver) = owners.get(&hit) {
            return Some(receiver);
        }
        nodes
            .iter()
            .flat_map(|node| node.hidden_text_spans.iter())
            .find(|(_, text, start, end)| {
                *text == hit
                    && self
                        .native
                        .text_bounds(*text, *start, *end)
                        .is_some_and(|bounds| bounds.contains(&at))
            })
            .and_then(|(link, ..)| owners.get(link))
            .copied()
    }

    /// The view a press at `at` (which natively hit `hit`) goes to: the
    /// innermost listener the platform picks — a pressable span under the
    /// point on a text view, a pressable view itself, or the nearest
    /// pressable ancestor — `None` when nothing takes the press. A hidden
    /// view resolves through its recorded press owner or its semantic
    /// owner's receiver.
    fn press_receiver(
        &self,
        nodes: &[SemanticNode],
        by_id: &std::collections::HashMap<NodeId, &SemanticNode>,
        parents: &std::collections::HashMap<NodeId, Option<NodeId>>,
        hidden_press_owners: &std::collections::HashMap<NodeId, NodeId>,
        hit: NodeId,
        at: Point<Points>,
    ) -> Option<NodeId> {
        if let Some(receiver) = self.hidden_owner_at(nodes, hidden_press_owners, hit, at) {
            return Some(receiver);
        }
        if let Some(node) = by_id.get(&hit).copied() {
            if let Some(span) = self.pressable_span_at(nodes, hit, at) {
                return Some(span.id);
            }
            return press_owner(nodes, node);
        }
        // A decorative hidden view: its press goes where its semantic
        // owner's would.
        let mut current = parents.get(&hit).copied().flatten();
        while let Some(id) = current {
            if let Some(&receiver) = hidden_press_owners.get(&id) {
                return Some(receiver);
            }
            if let Some(node) = by_id.get(&id) {
                return press_owner(nodes, node);
            }
            current = parents.get(&id).copied().flatten();
        }
        None
    }

    /// [`Agent::press_receiver`] for a hold: every receiver a long press
    /// starting at `at` can reach — the model's pick (it counts raw touch
    /// listeners, an app may hold from touches) and the platform's
    /// recognizer receiver (a touch observer does not own a hold).
    fn hold_receivers(
        &self,
        nodes: &[SemanticNode],
        by_id: &std::collections::HashMap<NodeId, &SemanticNode>,
        parents: &std::collections::HashMap<NodeId, Option<NodeId>>,
        hidden_hold_owners: &std::collections::HashMap<NodeId, NodeId>,
        hit: NodeId,
        at: Point<Points>,
    ) -> Vec<NodeId> {
        if let Some(receiver) = self.hidden_owner_at(nodes, hidden_hold_owners, hit, at) {
            return vec![receiver];
        }
        if let Some(node) = by_id.get(&hit).copied() {
            if let Ok(Some((_, receiver))) = self.long_pressable_span_at(nodes, hit, at) {
                return vec![receiver];
            }
            return self.hold_owners(nodes, node, parents);
        }
        // A decorative hidden view: its hold goes where its semantic
        // owner's would.
        let mut current = parents.get(&hit).copied().flatten();
        while let Some(id) = current {
            if let Some(&receiver) = hidden_hold_owners.get(&id) {
                return vec![receiver];
            }
            if let Some(node) = by_id.get(&id) {
                return self.hold_owners(nodes, node, parents);
            }
            current = parents.get(&id).copied().flatten();
        }
        Vec::new()
    }

    /// What a hold on `target` can go to: an enabled input itself (native
    /// text selection keeps the hold even under an ancestor's recognizer,
    /// while a hold handler on the input names it the receiver either
    /// way), else every receiver the gesture can reach — the model's
    /// long-press pick and the native recognizer's. An empty list means
    /// nothing takes the hold.
    fn hold_owners(
        &self,
        nodes: &[SemanticNode],
        target: &SemanticNode,
        parents: &std::collections::HashMap<NodeId, Option<NodeId>>,
    ) -> Vec<NodeId> {
        let mut current = Some(target.id);
        while let Some(id) = current {
            let Some(node) = nodes.iter().find(|node| node.id == id) else {
                break;
            };
            if node.input && node.enabled {
                return vec![node.id];
            }
            let mut receivers = Vec::new();
            let native = self.scene.native_hold_receiver(id).ok().flatten();
            if let Some(receiver) = native {
                receivers.push(receiver);
            }
            if let Ok(Pick::One(receiver)) = self.long_pressable(id)
                && !receivers.contains(&receiver)
            {
                // The model pick counts raw touch listeners, and a
                // touch-driven hold still reaches one over a
                // press-down/up-only child: touches follow GPUI's raw
                // touch path beside the press recognizers. That fallback
                // holds only when the pick is touch-only — a picked
                // ancestor that also owns a native press callback needs
                // its recognizer, so it counts just at or below the
                // native receiver.
                let touch_only = nodes
                    .iter()
                    .find(|node| node.id == receiver)
                    .is_some_and(|node| node.touch)
                    && self.scene.native_hold_receiver(receiver).ok().flatten() != Some(receiver);
                if touch_only
                    || native.is_none_or(|native| is_descendant_or_same(receiver, native, parents))
                {
                    receivers.push(receiver);
                }
            }
            if !receivers.is_empty() {
                return receivers;
            }
            current = node.parent;
        }
        Vec::new()
    }

    fn long_pressable_span_at(
        &self,
        nodes: &[SemanticNode],
        text: NodeId,
        at: Point<Points>,
    ) -> Result<Option<(NodeId, NodeId)>, String> {
        let Some(span) = self.holdable_span_at(nodes, text, at) else {
            return Ok(None);
        };
        if !span.enabled {
            return Err(format!("node {} is disabled", span.id));
        }
        let selector = format!("view:{}", span.id);
        match self.long_pressable(span.id)? {
            Pick::One(receiver) => Ok(Some((span.id, receiver))),
            Pick::None => Ok(None),
            pick => self
                .picked(pick, &selector, "pressable view")
                .map(|receiver| Some((span.id, receiver))),
        }
    }

    fn node_point_without_reveal(
        &self,
        id: NodeId,
        selector: &str,
    ) -> Result<Point<Points>, String> {
        self.target_node_window(id)?;
        let nodes = self.scene.semantic_tree()?;
        let node = nodes
            .iter()
            .find(|node| node.id == id)
            .ok_or_else(|| format!("`{selector}` is offscreen (node unavailable)"))?;
        if !node.enabled {
            return Err(format!("`{selector}` is disabled"));
        }
        let geometry = self
            .target_geometry(node)
            .ok_or_else(|| format!("`{selector}` is offscreen (no native bounds)"))?;
        self.actionable_point(id, geometry.owner, geometry.bounds.center(), selector)
    }

    /// The one view `pick` gave for `selector` (from [`Self::find`]).
    fn picked(&self, pick: Pick, selector: &str, what: &str) -> Result<NodeId, String> {
        match pick {
            Pick::One(id) => Ok(id),
            Pick::None => Err(format!(
                "{}; {}",
                failure::not_found(what, selector),
                self.on_screen()
            )),
            Pick::Many(n) => Err(format!(
                "`{selector}` matches {n} {what}s: say which (`{selector} >> nth=0`), or look inside \
                 a view around it (`id:list >> {selector}`)"
            )),
            Pick::Disabled => Err(format!(
                "{}; {}",
                failure::disabled(selector, what),
                self.on_screen()
            )),
        }
    }

    /// The view `selector` names as `pick` finds it in the model, and the
    /// middle of it on screen.
    fn find(
        &self,
        selector: &str,
        what: &str,
        target: PickTarget,
    ) -> Result<(NodeId, Point<Points>), String> {
        self.settled()?;
        let picked = self.scene.pick(selector, target)?;
        let id = self.picked(picked, selector, what)?;
        self.target_node_window(id)?;
        if let Some((canvas, center)) = self.scene.canvas_node(selector)? {
            let at = self.scroll_point_into_view(canvas, selector, Some(center))?;
            return Ok((id, self.actionable_point(id, canvas, at, selector)?));
        }
        let Some((text, start, end)) = self.scene.span(id)? else {
            // A press on what the selector matches inside the view that
            // takes it (a cell's text in a row) lands on that match, as a
            // finger taps the text it reads.
            let pressing = matches!(target, PickTarget::Pressable | PickTarget::LongPressable);
            let inner = if pressing {
                self.scene.match_inside(selector, id)?
            } else {
                None
            };
            let (owner, at) = match inner {
                Some(inner) if self.native.bounds(inner).is_some() => {
                    (inner, self.scroll_into_view(inner, selector)?)
                }
                _ => (id, self.scroll_into_view(id, selector)?),
            };
            if matches!(target, PickTarget::Input) {
                // Scrolling can bring the field underneath another view.
                self.scene.pick(selector, target)?;
            }
            return Ok((id, self.actionable_point(id, owner, at, selector)?));
        };
        // A span of a text: its text view comes into view, then the finger
        // goes on its characters.
        self.scroll_into_view(text, selector)?;
        let at = self
            .native
            .text_bounds(text, start, end)
            .filter(|b| !b.is_empty())
            .ok_or_else(|| {
                format!(
                    "`{selector}` is a span of a text, and this platform does not say where its \
                     characters are; {}",
                    self.on_screen()
                )
            })?;
        Ok((id, self.actionable_point(id, text, at.center(), selector)?))
    }

    fn target_node_window(&self, id: NodeId) -> Result<(), String> {
        self.native.target_window(self.scene.window_of(id)?)
    }

    fn actionable_point(
        &self,
        id: NodeId,
        native_owner: NodeId,
        fallback: Point<Points>,
        selector: &str,
    ) -> Result<Point<Points>, String> {
        self.actionable_point_with_press_owner(
            id,
            native_owner,
            fallback,
            selector,
            None,
            false,
            false,
        )
    }

    /// [`Agent::actionable_point`] for a point the caller chose: outside
    /// the target's visible area it is an error, not moved to its middle.
    fn explicit_actionable_point(
        &self,
        id: NodeId,
        native_owner: NodeId,
        at: Point<Points>,
        selector: &str,
    ) -> Result<Point<Points>, String> {
        self.actionable_point_with_press_owner(id, native_owner, at, selector, None, true, false)
    }

    #[allow(clippy::too_many_arguments)]
    fn actionable_point_with_press_owner(
        &self,
        id: NodeId,
        native_owner: NodeId,
        fallback: Point<Points>,
        selector: &str,
        press_owner: Option<NodeId>,
        explicit: bool,
        for_long_press: bool,
    ) -> Result<Point<Points>, String> {
        if !self.require_hit_test {
            return Ok(fallback);
        }
        let nodes = self.scene.semantic_tree()?;
        let by_id: std::collections::HashMap<NodeId, &SemanticNode> =
            nodes.iter().map(|node| (node.id, node)).collect();
        let target = by_id
            .get(&id)
            .or_else(|| by_id.get(&native_owner))
            .copied()
            .ok_or_else(|| format!("`{selector}` is offscreen (node {id} is unavailable)"))?;
        if !target.enabled {
            return Err(format!("`{selector}` is disabled"));
        }
        let geometry = match self.target_geometry(target) {
            Some(geometry) if geometry.owner == native_owner => geometry,
            Some(geometry) if !geometry.synthetic => TargetGeometry {
                owner: native_owner,
                bounds: self
                    .native
                    .bounds(native_owner)
                    .ok_or_else(|| format!("`{selector}` is offscreen (no native bounds)"))?,
                synthetic: false,
            },
            _ if matches!(target.native_target, SemanticTarget::View) => TargetGeometry {
                owner: native_owner,
                bounds: self
                    .native
                    .bounds(native_owner)
                    .ok_or_else(|| format!("`{selector}` is offscreen (no native bounds)"))?,
                synthetic: false,
            },
            _ => return Err(format!("`{selector}` is offscreen (no native bounds)")),
        };
        if geometry.synthetic && !geometry.bounds.contains(&fallback) {
            return Err(format!("`{selector}` is offscreen"));
        }
        let mut visible = Bounds::new(Point::default(), self.native.viewport());
        visible = intersection(visible, geometry.bounds)
            .ok_or_else(|| format!("`{selector}` is offscreen"))?;
        let owner_node = by_id.get(&native_owner).copied().unwrap_or(target);
        let mut parent = if target.modal {
            None
        } else if geometry.synthetic {
            Some(native_owner)
        } else {
            owner_node.parent
        };
        while let Some(parent_id) = parent {
            let Some(ancestor) = by_id.get(&parent_id) else {
                break;
            };
            if ancestor.clipping {
                let ancestor_bounds = self.native.bounds(parent_id).ok_or_else(|| {
                    format!("`{selector}` is offscreen (clipping ancestor has no native bounds)")
                })?;
                visible = intersection(visible, ancestor_bounds)
                    .ok_or_else(|| format!("`{selector}` is offscreen"))?;
            }
            if ancestor.modal {
                break;
            }
            parent = ancestor.parent;
        }
        let at = if visible.contains(&fallback) {
            fallback
        } else if explicit {
            return Err(format!(
                "`{selector}` is offscreen at ({}, {}): the point is outside its visible area",
                fallback.x.0, fallback.y.0
            ));
        } else {
            visible.center()
        };
        let at = self.clear_of_keyboard(visible, at, explicit, selector)?;
        let parents = mounted_parents(&nodes);
        // A press on a hidden view that takes presses itself goes to it,
        // not to the target.
        let hidden_press_owners: std::collections::HashMap<NodeId, NodeId> = nodes
            .iter()
            .flat_map(|node| node.hidden_press_owners.iter().copied())
            .collect();
        // A hold's hidden owners are its own set — a view listening only
        // for long presses reserves the hold, not the tap — but a hold's
        // release still delivers a press, so a press-only hidden view
        // intercepts the hold too. Hold receivers win ties: a view with
        // both listeners resolves to its hold receiver.
        let mut hidden_hold_owners = hidden_press_owners.clone();
        hidden_hold_owners.extend(
            nodes
                .iter()
                .flat_map(|node| node.hidden_hold_owners.iter().copied()),
        );
        let deadline = Instant::now() + Duration::from_millis(100);
        loop {
            match self.native.hit_at(at) {
                Ok(Some(hit))
                    if press_owner.is_some()
                        && let Some(receiver) = self.hidden_owner_at(
                            &nodes,
                            if for_long_press {
                                &hidden_hold_owners
                            } else {
                                &hidden_press_owners
                            },
                            hit,
                            at,
                        ) =>
                {
                    return Err(format!(
                        "`{selector}`: a {} at ({}, {}) goes to node {receiver}, an \
                         accessibility-hidden view that takes it itself",
                        if for_long_press { "hold" } else { "press" },
                        at.x.0,
                        at.y.0
                    ));
                }
                Ok(Some(hit))
                    if match press_owner {
                        // A targeted press proves the hit's press receiver is
                        // the requested one: the platform sends the press to
                        // the innermost view that hears presses, which an
                        // independently pressable nested control can be even
                        // when it sits inside the target. A synthetic target
                        // (a link span, a canvas) is the receiver under its
                        // owner view's hit, and decorative views (no press
                        // receiver of their own) under the target still count.
                        Some(expected) => {
                            // A hold's hit check resolves receivers by the
                            // long-press rules — an input or a long-press-only
                            // child inside a tappable parent takes the hold
                            // itself — not by the press rules.
                            if for_long_press {
                                // A hold can reach several receivers: a
                                // touch-observing child takes a touch-driven
                                // hold while an ancestor's recognizer still
                                // fires. Any of them may be the expected one.
                                let receivers = self.hold_receivers(
                                    &nodes,
                                    &by_id,
                                    &parents,
                                    &hidden_hold_owners,
                                    hit,
                                    at,
                                );
                                if receivers.is_empty() {
                                    is_descendant_or_same(hit, expected, &parents)
                                } else {
                                    // A hold targeted at an enabled input
                                    // owns the gesture itself (native text
                                    // selection) even when the model picked
                                    // a pressable ancestor as the receiver.
                                    receivers.contains(&expected)
                                        || (target.input
                                            && target.enabled
                                            && receivers.contains(&target.id))
                                        || (expected == target.id
                                            && hold_reaches_through_own_input(
                                                target,
                                                geometry.bounds,
                                                &receivers,
                                                &by_id,
                                                &parents,
                                                |id| self.native.bounds(id),
                                            ))
                                }
                            } else {
                                match self.press_receiver(
                                    &nodes,
                                    &by_id,
                                    &parents,
                                    &hidden_press_owners,
                                    hit,
                                    at,
                                ) {
                                    Some(receiver) => {
                                        // The same receiver is not enough: a
                                        // covering sibling shares the button
                                        // but the press still missed the
                                        // target. The hit (or the semantic
                                        // node it reports through) must be
                                        // the target or sit inside it — or
                                        // carry the target inside it, as a
                                        // synthetic span's owner view does.
                                        receiver == expected && {
                                            let mut hit_node = Some(hit);
                                            let semantic_hit = loop {
                                                match hit_node {
                                                    None => break None,
                                                    Some(id) if by_id.contains_key(&id) => {
                                                        break Some(id);
                                                    }
                                                    Some(id) => {
                                                        hit_node = nodes
                                                            .iter()
                                                            .find(|node| {
                                                                node.hidden_descendants
                                                                    .contains(&id)
                                                            })
                                                            .map(|node| node.id)
                                                            .or_else(|| {
                                                                parents.get(&id).copied().flatten()
                                                            });
                                                    }
                                                }
                                            };
                                            semantic_hit.is_some_and(|hit_node| {
                                                is_descendant_or_same(hit_node, target.id, &parents)
                                                    || is_descendant_or_same(
                                                        target.id, hit_node, &parents,
                                                    )
                                            })
                                        }
                                    }
                                    None => is_descendant_or_same(hit, expected, &parents),
                                }
                            }
                        }
                        None => is_descendant_or_same(hit, native_owner, &parents),
                    } =>
                {
                    return Ok(at);
                }
                Ok(Some(hit)) => {
                    if Instant::now() >= deadline {
                        return Err(format!("occluded_by: {hit}"));
                    }
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        return Err(format!("`{selector}` is offscreen or has no native hit"));
                    }
                }
                Err(error) if error == NATIVE_HIT_TEST_UNSUPPORTED => {
                    return Err(error);
                }
                Err(error) => return Err(error),
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// `at`, or, when the software keyboard covers it, the middle of the
    /// part of `visible` above the keyboard's top edge, as a finger finds
    /// the part of a control the keyboard leaves showing. A point the
    /// caller chose (`explicit`) is not moved. Refused with `occluded_by`
    /// when the keyboard covers the target: a press there lands on a key.
    /// A host that cannot tell where the keyboard is leaves `at` as it is.
    fn clear_of_keyboard(
        &self,
        visible: Bounds<Points>,
        at: Point<Points>,
        explicit: bool,
        selector: &str,
    ) -> Result<Point<Points>, String> {
        let Ok(Some(keyboard)) = self.keyboard_frame() else {
            return Ok(at);
        };
        if !keyboard.contains(&at) {
            return Ok(at);
        }
        // A finger needs some of the target above the keyboard: all of a
        // short one, or 8 points of a tall one.
        let reachable = KEYBOARD_MIN_VISIBLE.min(visible.size.height.0);
        if !explicit && keyboard.origin.y.0 - visible.origin.y.0 >= reachable {
            let above = Bounds::new(
                visible.origin,
                Size::new(
                    visible.size.width,
                    Points(keyboard.origin.y.0 - visible.origin.y.0),
                ),
            );
            let at = above.center();
            if !keyboard.contains(&at) {
                return Ok(at);
            }
        }
        Err(format!(
            "occluded_by: software keyboard (a press on `{selector}` at ({}, {}) lands on a key; \
             hide the keyboard or scroll the target above it first)",
            at.x.0, at.y.0
        ))
    }

    /// The middle of view `id` on screen, scrolled into view first when it
    /// is not: the innermost scroll view around it that hides it scrolls it
    /// towards its own middle, a gesture at a time, as a user would.
    fn scroll_into_view(&self, id: NodeId, selector: &str) -> Result<Point<Points>, String> {
        self.scroll_point_into_view(id, selector, None)
    }

    /// Brings a point within a view into view (its center when `local`
    /// is `None`), retaining a canvas node's position through scrolling.
    fn scroll_point_into_view(
        &self,
        id: NodeId,
        selector: &str,
        local: Option<Point<Points>>,
    ) -> Result<Point<Points>, String> {
        let (at, scrolled) = self.bring_into_view(id, selector, local)?;
        if !scrolled {
            return Ok(at);
        }
        // What the scrolling brought (rows bound, images decoded) lands
        // before the step acts, once: between the gestures only the view's
        // own movement is waited for ([`Agent::stopped`]).
        self.settled()?;
        Ok(self
            .native
            .bounds(id)
            .map_or(at, |b| local.map_or(b.center(), |p| b.origin + p)))
    }

    /// [`Agent::scroll_into_view`]'s gestures: the view's middle, and
    /// whether a gesture moved anything.
    fn bring_into_view(
        &self,
        id: NodeId,
        selector: &str,
        local: Option<Point<Points>>,
    ) -> Result<(Point<Points>, bool), String> {
        // Strict mode's errors say `offscreen`, as its others do; the
        // ordinary wording is the headless driver's.
        let not_on_screen = || {
            let state = if self.require_hit_test {
                "is offscreen"
            } else {
                "is not on screen"
            };
            format!(
                "`{selector}` {state} (it has no native view); {}",
                self.on_screen()
            )
        };
        let scrollers = self.scene.scroll_ancestors(id)?;
        let mut before: Option<Point<Points>> = None;
        // The gestures that moved nothing: each moves the finger elsewhere
        // ([`anchor`]).
        let mut misses = 0;
        for _ in 0..SCROLLS_INTO_VIEW {
            let bounds = self.native.bounds(id).ok_or_else(not_on_screen)?;
            let at = local.map_or(bounds.center(), |p| bounds.origin + p);
            let viewport = self.native.viewport();
            let window = Bounds::new(Point::new(Points(0.0), Points(0.0)), viewport);
            if let Some(b) = before
                && (b.x.0 - at.x.0).abs() < 1.0
                && (b.y.0 - at.y.0).abs() < 1.0
            {
                misses += 1;
            }
            // A scroll view already at the end a gesture would move it
            // towards brings nothing nearer: the finger would pull it past
            // that end (a pull to refresh, a page sheet following the drag
            // down and closing).
            let can_move = |scroller: NodeId, horizontal: bool, b: &Bounds<Points>| {
                let back = if horizontal {
                    at.x.0 < b.center().x.0
                } else {
                    at.y.0 < b.center().y.0
                };
                self.native
                    .scroll_room(scroller, horizontal)
                    .is_none_or(|(start, end)| if back { start.0 } else { end.0 } >= 1.0)
            };
            // The innermost scroll view whose bounds leave the middle out,
            // or near their edge (where a finger does not press and bars,
            // the home indicator and fades sit), else the outermost one
            // when the window does.
            let hiding = scrollers.iter().find_map(|&(scroller, horizontal)| {
                let b = self.native.bounds(scroller)?;
                (!well_inside(&b, at, horizontal) && can_move(scroller, horizontal, &b))
                    .then_some((scroller, horizontal, b))
            });
            let hiding = hiding.or_else(|| {
                let &(s, h) = scrollers.last()?;
                (!well_inside(&window, at, h)).then_some(())?;
                let b = self.native.bounds(s)?;
                can_move(s, h, &b).then_some((s, h, b))
            });
            let Some((scroll, horizontal, scroller)) = hiding else {
                if window.contains(&at) {
                    return Ok((at, before.is_some()));
                }
                return Err(not_on_screen());
            };
            // The finger moves towards the start when the content moves
            // towards its end.
            let to_start = if horizontal {
                at.x.0 > scroller.center().x.0
            } else {
                at.y.0 > scroller.center().y.0
            };
            let Some(along) = anchor(misses, to_start) else {
                // No gesture moves it any more (its scroll view's end): it
                // is pressed where it is when it shows.
                return if window.contains(&at) {
                    Ok((at, before.is_some()))
                } else {
                    Err(not_on_screen())
                };
            };
            let along = self.landing(scroll, &scroller, horizontal, along)?;
            let middle = if horizontal {
                point(
                    scroller.origin.x + scroller.size.width * along,
                    scroller.center().y,
                )
            } else {
                point(
                    scroller.center().x,
                    scroller.origin.y + scroller.size.height * along,
                )
            };
            let (distance, extent) = if horizontal {
                (at.x.0 - middle.x.0, scroller.size.width.0)
            } else {
                (at.y.0 - middle.y.0, scroller.size.height.0)
            };
            // A finger dragging it stays inside it: as far as the edge it
            // moves towards (the start when the content moves towards its
            // end) less a tenth, a tenth at least.
            let toward = if distance > 0.0 { along } else { 1.0 - along };
            let room = extent * (toward - 0.1).max(0.1);
            let step = distance.clamp(-room, room);
            let delta = if horizontal {
                Point::new(Points(step), Points(0.0))
            } else {
                Point::new(Points(0.0), Points(step))
            };
            self.native.scroll(middle, delta);
            self.stopped(id)?;
            before = Some(at);
        }
        Err(format!(
            "`{selector}` did not come into view; {}",
            self.on_screen()
        ))
    }

    /// Where along scroll view `id` (at `bounds`) a finger lands to scroll
    /// it, as a fraction of its length, nearest `along`: on a spot clear of
    /// the views inside that take a drag themselves
    /// ([`Scene::drag_takers`]), as a user puts the finger on the page and
    /// not on a slider or a drawing area. On one of those the app's
    /// responder decides, a moment later, whether the scroll view keeps the
    /// gesture: it may scroll, stop half way or not move at all. `along`
    /// where no spot is clear, and on a platform with a mouse
    /// ([`Native::mouse`]), whose wheel scrolls the scroll view wherever it
    /// is.
    fn landing(
        &self,
        id: NodeId,
        bounds: &Bounds<Points>,
        horizontal: bool,
        along: f32,
    ) -> Result<f32, String> {
        if self.native.mouse() {
            return Ok(along);
        }
        let takers = self.scene.drag_takers(id)?;
        if takers.is_empty() {
            return Ok(along);
        }
        // Along the scroll view's length, and across it, where the finger
        // moves: its middle.
        let axes = |b: &Bounds<Points>| {
            if horizontal {
                (
                    (b.origin.x.0, b.size.width.0),
                    (b.origin.y.0, b.size.height.0),
                )
            } else {
                (
                    (b.origin.y.0, b.size.height.0),
                    (b.origin.x.0, b.size.width.0),
                )
            }
        };
        let ((start, extent), (across, across_extent)) = axes(bounds);
        let line = across + across_extent / 2.0;
        // What the takers cover of the finger's line.
        let covered: Vec<(f32, f32)> = takers
            .into_iter()
            .filter_map(|taker| self.native.bounds(taker))
            .filter_map(|b| {
                let ((from, length), (side, width)) = axes(&b);
                (side..=side + width)
                    .contains(&line)
                    .then_some((from, from + length))
            })
            .collect();
        Ok(clear_spot(start, extent, along, &covered).unwrap_or(along))
    }

    /// [`Scene::pick`] of view `id` for a long press: `Disabled` when its
    /// receiver is ([`SemanticScene::long_press_disabled`]).
    fn long_pressable(&self, id: NodeId) -> Result<Pick, String> {
        let pick = self
            .scene
            .pick(&format!("view:{id}"), PickTarget::LongPressable)?;
        if pick == Pick::None && self.scene.long_press_disabled(id)? {
            return Ok(Pick::Disabled);
        }
        Ok(pick)
    }

    fn long_press(&self, id: NodeId, hold: Option<Duration>) -> Result<(), String> {
        self.settled_and_keyboard()?;
        let selector = &format!("view:{id}");
        self.settled()?;
        let (what, target) = match self.long_pressable(id)? {
            Pick::Disabled => {
                return self
                    .picked(Pick::Disabled, selector, "pressable view")
                    .map(drop);
            }
            Pick::None => match self.scene.pick(selector, PickTarget::Input)? {
                Pick::One(_) => ("input", PickTarget::Input),
                _ => ("pressable view", PickTarget::LongPressable),
            },
            _ => ("pressable view", PickTarget::LongPressable),
        };
        let (id, at) = self.find(selector, what, target)?;
        let nodes = self.scene.semantic_tree()?;
        let node = nodes
            .iter()
            .find(|node| node.id == id)
            .ok_or_else(|| format!("`{selector}` is offscreen (node unavailable)"))?;
        if !node.enabled {
            return Err(format!("`{selector}` is disabled"));
        }
        let geometry = self
            .target_geometry(node)
            .ok_or_else(|| format!("`{selector}` is offscreen (no native bounds)"))?;
        let at = self.actionable_point_with_press_owner(
            id,
            geometry.owner,
            at,
            selector,
            Some(id),
            false,
            true,
        )?;
        self.hold_long_press(id, at, hold)
    }

    fn hold_long_press(
        &self,
        id: NodeId,
        at: Point<Points>,
        hold: Option<Duration>,
    ) -> Result<(), String> {
        let hold = match hold {
            Some(hold) => hold,
            None => {
                let delay = self.scene.long_press_delay(id)?;
                // A synthetic target (a link span) has no delay of its
                // own: the view owning it does.
                let delay = match delay {
                    delay @ Some(_) => delay,
                    None => {
                        let owner = self
                            .scene
                            .semantic_tree()?
                            .into_iter()
                            .find(|node| node.id == id)
                            .and_then(|node| match node.native_target {
                                SemanticTarget::Text { owner, .. }
                                | SemanticTarget::Canvas { owner, .. } => Some(owner),
                                _ => None,
                            })
                            .unwrap_or(id);
                        self.scene.long_press_delay(owner)?
                    }
                }
                .map_or(LONG_PRESS, |ms| {
                    Duration::from_secs_f64(f64::from(ms.max(0.0)) / 1000.0)
                });
                // Hold a little past the delay, as a finger does.
                delay + Duration::from_millis(100)
            }
        };
        if !self.scene.manual_clock() {
            self.native.long_press(at, hold);
            return self.settled();
        }
        // The app's time passes while the finger rests, as headless: once
        // it has landed, the manual clock moves by the rest of the hold
        // (the timers of a hold the app keeps itself fire on the way).
        // The move is sent while the finger rests, so the engine takes it
        // between the touch's start and its end.
        let moved = std::thread::scope(|s| {
            s.spawn(|| self.native.long_press(at, hold));
            std::thread::sleep(LANDING);
            let ms = hold.saturating_sub(LANDING).as_millis() as u64;
            self.scene
                .move_clock(nsh_dev::protocol::ClockChange::RunFor { ms })
        });
        moved.and_then(|()| self.settled())
    }

    /// Focuses the input `selector` names by pressing it.
    fn focus(&self, selector: &str) -> Result<(), String> {
        let (id, at) = self.find(selector, "input", PickTarget::Input)?;
        self.native.press(at);
        self.settled()?;
        if let Some(focused) = self.native.focused()
            && focused != id
        {
            return Err(failure::covered(selector, &format!("`view:{focused}`")));
        }
        Ok(())
    }

    fn paste(&self, selector: &str, content: &nsh_core::PasteEvent) -> Result<(), String> {
        // As the user pastes into the field they are in: a press first
        // would move its caret, collapsing a selection the paste replaces.
        let (id, _) = self.find(selector, "input", PickTarget::Input)?;
        if self.native.focused() != Some(id) {
            self.focus(selector)?;
        }
        self.native.paste(content)?;
        self.settled()
    }

    /// Waits until the app is quiet ([`QUIET`], no native animation that
    /// ends by itself), [`SETTLE`] at most, then fails if it threw
    /// meanwhile.
    fn settled(&self) -> Result<(), String> {
        let start = Instant::now();
        // When an animation last ran: what it ends with (a screen popped
        // once it slid out) comes after it.
        let mut moved = start;
        let mut first = None;
        loop {
            let activity = self.scene.activity()?;
            let first = *first.get_or_insert(activity.batches);
            if self.native.animating() {
                moved = Instant::now();
            }
            // Quiet since the step began, the last batch and the last
            // animation.
            let since = activity
                .since_last
                .map_or(start.elapsed(), |d| d.min(start.elapsed()))
                .min(moved.elapsed());
            if since >= QUIET {
                break;
            }
            if start.elapsed() >= SETTLE {
                // The step goes on all the same; why it waited this long
                // tells why a run is slow.
                log::info!(
                    "test agent: the app did not settle in {} s: {} batches came, an animation ran {} ms ago",
                    SETTLE.as_secs(),
                    activity.batches - first,
                    moved.elapsed().as_millis()
                );
                break;
            }
            self.scene.wait_for_batch(activity.batches, POLL);
        }
        if let Some(failure) = self.native.take_failure() {
            return Err(failure);
        }
        self.native.laid_out()?;
        self.check()
    }

    /// Waits until view `id` stops moving after a gesture scrolled it: its
    /// bounds the same over a few polls ([`SETTLE`] at most). What else the
    /// app does meanwhile (a ticking timer, an animation of its own) does
    /// not hold the next gesture back, as it holds a step's end
    /// ([`Agent::settled`]).
    fn stopped(&self, id: NodeId) -> Result<(), String> {
        const STILL: usize = 3;
        let start = Instant::now();
        let mut last = self.native.bounds(id);
        let mut still = 0;
        while still < STILL && start.elapsed() < SETTLE {
            std::thread::sleep(POLL);
            let now = self.native.bounds(id);
            if now == last {
                still += 1;
            } else {
                still = 0;
                last = now;
            }
        }
        self.check()
    }

    /// Fails with the app's first error since the last look.
    fn check(&self) -> Result<(), String> {
        match self.scene.take_errors()?.into_iter().next() {
            Some(error) => Err(format!("the app threw: {error}")),
            None => Ok(()),
        }
    }

    fn on_screen(&self) -> String {
        let texts = self.scene.texts().unwrap_or_default();
        format!(
            "on screen: [{}]",
            nsh_testing::screen_texts(&texts, Some(nsh_testing::SCREEN_TEXT_LIMIT))
        )
    }
}
