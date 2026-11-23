use nsh_core::{
    AccessibilityRole, Checked, EventKind, NodeId, ViewKind,
    element::PropEnum,
    geometry::{Bounds, Point, Points, Size},
};
use nsh_testing::{
    MountedView, Screen,
    snapshot::{ScreenSnapshot, ViewSnapshot},
};

pub(super) trait ScreenData {
    fn mounted(&self, id: NodeId) -> Option<ViewData<'_>>;
    fn children(&self, id: NodeId) -> &[NodeId];
    fn stack_screens(&self, id: NodeId) -> Vec<NodeId>;
}
impl ScreenData for Screen {
    fn mounted(&self, id: NodeId) -> Option<ViewData<'_>> {
        self.mounted(id).map(ViewData::Mounted)
    }
    fn children(&self, id: NodeId) -> &[NodeId] {
        self.children(id)
    }
    fn stack_screens(&self, id: NodeId) -> Vec<NodeId> {
        self.stack_screens(id)
    }
}
impl ScreenData for ScreenSnapshot {
    fn mounted(&self, id: NodeId) -> Option<ViewData<'_>> {
        self.views
            .iter()
            .find(|v| v.id == id)
            .map(ViewData::Snapshot)
    }
    fn children(&self, id: NodeId) -> &[NodeId] {
        self.views
            .iter()
            .find(|v| v.id == id)
            .map_or(&[], |v| &v.children)
    }
    fn stack_screens(&self, id: NodeId) -> Vec<NodeId> {
        self.views
            .iter()
            .find(|v| v.id == id)
            .map_or_else(Vec::new, |v| v.stack_screens.clone())
    }
}
pub(super) enum ViewData<'a> {
    Mounted(MountedView<'a>),
    Snapshot(&'a ViewSnapshot),
}
pub(super) struct State {
    pub checked: Option<Checked>,
    pub busy: bool,
    pub expanded: Option<bool>,
}
pub(super) struct Span<'a> {
    pub view: NodeId,
    pub text: &'a str,
    pub start: u32,
    pub end: u32,
}
pub(super) struct Value<'a> {
    pub text: Option<&'a str>,
    pub now: Option<f32>,
}
impl ViewData<'_> {
    pub(super) fn hidden(&self) -> bool {
        match self {
            Self::Mounted(v) => v.hidden(),
            Self::Snapshot(v) => v.hidden,
        }
    }
    pub(super) fn accessibility_hidden(&self) -> bool {
        match self {
            Self::Mounted(v) => v.accessibility_hidden(),
            Self::Snapshot(v) => v.accessibility_hidden,
        }
    }
    pub(super) fn disabled(&self) -> bool {
        match self {
            Self::Mounted(v) => v.disabled(),
            Self::Snapshot(v) => v.disabled,
        }
    }
    pub(super) fn secure(&self) -> bool {
        match self {
            Self::Mounted(v) => v.secure(),
            Self::Snapshot(v) => v.secure,
        }
    }
    pub(super) fn selected(&self) -> bool {
        match self {
            Self::Mounted(v) => v.selected(),
            Self::Snapshot(v) => v.selected,
        }
    }
    pub(super) fn clips(&self) -> bool {
        match self {
            Self::Mounted(v) => v.clips(),
            Self::Snapshot(v) => v.clips,
        }
    }
    pub(super) fn scrolls(&self) -> bool {
        match self {
            Self::Mounted(v) => v.scrolls(),
            Self::Snapshot(v) => v.scrolls,
        }
    }
    pub(super) fn canvas_node(&self) -> bool {
        match self {
            Self::Mounted(v) => v.canvas_node(),
            Self::Snapshot(v) => v.canvas_node,
        }
    }
    pub(super) fn test_id(&self) -> Option<&str> {
        match self {
            Self::Mounted(v) => v.test_id(),
            Self::Snapshot(v) => v.test_id.as_deref(),
        }
    }
    pub(super) fn label(&self) -> Option<&str> {
        match self {
            Self::Mounted(v) => v.label(),
            Self::Snapshot(v) => v.label.as_deref(),
        }
    }
    pub(super) fn text(&self) -> Option<&str> {
        match self {
            Self::Mounted(v) => v.text(),
            Self::Snapshot(v) => v.text.as_deref(),
        }
    }
    pub(super) fn input_text(&self) -> Option<&str> {
        match self {
            Self::Mounted(v) => v.input_text(),
            Self::Snapshot(v) => v.input_text.as_deref(),
        }
    }
    pub(super) fn accessibility_hint(&self) -> Option<&str> {
        match self {
            Self::Mounted(v) => v.accessibility_hint(),
            Self::Snapshot(v) => v.accessibility_hint.as_deref(),
        }
    }
    pub(super) fn parent(&self) -> Option<NodeId> {
        match self {
            Self::Mounted(v) => v.parent(),
            Self::Snapshot(v) => v.parent,
        }
    }
    pub(super) fn accessible(&self) -> Option<bool> {
        match self {
            Self::Mounted(v) => v.accessible(),
            Self::Snapshot(v) => v.accessible,
        }
    }
    pub(super) fn listens(&self, event: EventKind) -> bool {
        match self {
            Self::Mounted(v) => v.events().contains(event),
            Self::Snapshot(v) => v.events.iter().any(|name| name == event.prop_name()),
        }
    }
    pub(super) fn kind(&self) -> Option<ViewKind> {
        match self {
            Self::Mounted(v) => v.kind(),
            Self::Snapshot(v) => match v.kind.as_deref() {
                Some("view") => Some(ViewKind::View),
                Some("text") => Some(ViewKind::Text),
                Some("image") => Some(ViewKind::Image),
                Some("scroll") => Some(ViewKind::Scroll),
                Some("input") => Some(ViewKind::Input),
                Some("stack") => Some(ViewKind::Stack),
                Some("screen") => Some(ViewKind::Screen),
                Some("modal") => Some(ViewKind::Modal),
                Some("header") => Some(ViewKind::HeaderItem),
                Some("accessory") => Some(ViewKind::InputAccessory),
                Some("canvas") => Some(ViewKind::Canvas),
                Some("web") => Some(ViewKind::WebView),
                Some("native") => Some(ViewKind::Native),
                Some("video") => Some(ViewKind::Video),
                _ => None,
            },
        }
    }
    pub(super) fn role(&self) -> AccessibilityRole {
        match self {
            Self::Mounted(v) => v.role(),
            Self::Snapshot(v) => match v.role.as_str() {
                name if name == AccessibilityRole::None.js_name() => AccessibilityRole::None,
                name if name == AccessibilityRole::Button.js_name() => AccessibilityRole::Button,
                name if name == AccessibilityRole::Link.js_name() => AccessibilityRole::Link,
                name if name == AccessibilityRole::Search.js_name() => AccessibilityRole::Search,
                name if name == AccessibilityRole::Image.js_name() => AccessibilityRole::Image,
                name if name == AccessibilityRole::ImageButton.js_name() => {
                    AccessibilityRole::ImageButton
                }
                name if name == AccessibilityRole::KeyboardKey.js_name() => {
                    AccessibilityRole::KeyboardKey
                }
                name if name == AccessibilityRole::Text.js_name() => AccessibilityRole::Text,
                name if name == AccessibilityRole::Adjustable.js_name() => {
                    AccessibilityRole::Adjustable
                }
                name if name == AccessibilityRole::Header.js_name() => AccessibilityRole::Header,
                name if name == AccessibilityRole::Summary.js_name() => AccessibilityRole::Summary,
                name if name == AccessibilityRole::Alert.js_name() => AccessibilityRole::Alert,
                name if name == AccessibilityRole::Checkbox.js_name() => {
                    AccessibilityRole::Checkbox
                }
                name if name == AccessibilityRole::Combobox.js_name() => {
                    AccessibilityRole::Combobox
                }
                name if name == AccessibilityRole::Menu.js_name() => AccessibilityRole::Menu,
                name if name == AccessibilityRole::Menubar.js_name() => AccessibilityRole::Menubar,
                name if name == AccessibilityRole::Menuitem.js_name() => {
                    AccessibilityRole::Menuitem
                }
                name if name == AccessibilityRole::Progressbar.js_name() => {
                    AccessibilityRole::Progressbar
                }
                name if name == AccessibilityRole::Radio.js_name() => AccessibilityRole::Radio,
                name if name == AccessibilityRole::Radiogroup.js_name() => {
                    AccessibilityRole::Radiogroup
                }
                name if name == AccessibilityRole::Scrollbar.js_name() => {
                    AccessibilityRole::Scrollbar
                }
                name if name == AccessibilityRole::Spinbutton.js_name() => {
                    AccessibilityRole::Spinbutton
                }
                name if name == AccessibilityRole::Switch.js_name() => AccessibilityRole::Switch,
                name if name == AccessibilityRole::Tab.js_name() => AccessibilityRole::Tab,
                name if name == AccessibilityRole::Tablist.js_name() => AccessibilityRole::Tablist,
                name if name == AccessibilityRole::Timer.js_name() => AccessibilityRole::Timer,
                name if name == AccessibilityRole::Toolbar.js_name() => AccessibilityRole::Toolbar,
                name if name == AccessibilityRole::List.js_name() => AccessibilityRole::List,
                name if name == AccessibilityRole::Grid.js_name() => AccessibilityRole::Grid,
                name if name == AccessibilityRole::Table.js_name() => AccessibilityRole::Table,
                name if name == AccessibilityRole::Row.js_name() => AccessibilityRole::Row,
                name if name == AccessibilityRole::Cell.js_name() => AccessibilityRole::Cell,
                name if name == AccessibilityRole::ColumnHeader.js_name() => {
                    AccessibilityRole::ColumnHeader
                }
                name if name == AccessibilityRole::RowGroup.js_name() => {
                    AccessibilityRole::RowGroup
                }
                name if name == AccessibilityRole::Listbox.js_name() => AccessibilityRole::Listbox,
                name if name == AccessibilityRole::Option.js_name() => AccessibilityRole::Option,
                name if name == AccessibilityRole::Group.js_name() => AccessibilityRole::Group,
                name if name == AccessibilityRole::Region.js_name() => AccessibilityRole::Region,
                name if name == AccessibilityRole::Navigation.js_name() => {
                    AccessibilityRole::Navigation
                }
                name if name == AccessibilityRole::Tree.js_name() => AccessibilityRole::Tree,
                name if name == AccessibilityRole::TreeItem.js_name() => {
                    AccessibilityRole::TreeItem
                }
                name if name == AccessibilityRole::MenuItemCheckbox.js_name() => {
                    AccessibilityRole::MenuItemCheckbox
                }
                name if name == AccessibilityRole::MenuItemRadio.js_name() => {
                    AccessibilityRole::MenuItemRadio
                }
                name if name == AccessibilityRole::Dialog.js_name() => AccessibilityRole::Dialog,
                name if name == AccessibilityRole::AlertDialog.js_name() => {
                    AccessibilityRole::AlertDialog
                }
                _ => AccessibilityRole::None,
            },
        }
    }
    pub(super) fn frame(&self) -> Bounds<Points> {
        match self {
            Self::Mounted(v) => v.frame(),
            Self::Snapshot(v) => {
                let [x, y, w, h] = v.frame;
                Bounds::new(
                    Point::new(Points(x), Points(y)),
                    Size::new(Points(w), Points(h)),
                )
            }
        }
    }
    pub(super) fn accessibility_state(&self) -> State {
        match self {
            Self::Mounted(v) => {
                let s = v.accessibility_state();
                State {
                    checked: s.checked,
                    busy: s.busy,
                    expanded: s.expanded,
                }
            }
            Self::Snapshot(v) => {
                let s = &v.accessibility_state;
                State {
                    checked: match s.checked.as_deref() {
                        Some("true") => Some(Checked::True),
                        Some("false") => Some(Checked::False),
                        Some("mixed") => Some(Checked::Mixed),
                        _ => None,
                    },
                    busy: s.busy,
                    expanded: s.expanded,
                }
            }
        }
    }
    pub(super) fn accessibility_value(&self) -> Option<Value<'_>> {
        match self {
            Self::Mounted(v) => v.accessibility_value().map(|v| Value {
                text: v.text.as_deref(),
                now: v.now,
            }),
            Self::Snapshot(v) => v.accessibility_value.as_ref().map(|v| Value {
                text: v.text.as_deref(),
                now: v.now,
            }),
        }
    }
    pub(super) fn span(&self) -> Option<Span<'_>> {
        match self {
            Self::Mounted(v) => v.span().map(|v| Span {
                view: v.view,
                text: v.text,
                start: v.start,
                end: v.end,
            }),
            Self::Snapshot(v) => v.span.as_ref().map(|v| Span {
                view: v.view,
                text: &v.text,
                start: v.start,
                end: v.end,
            }),
        }
    }
}
