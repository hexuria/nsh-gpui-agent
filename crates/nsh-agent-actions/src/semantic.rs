//! The semantic tree of the views a host mounted, read from its model's
//! [`Screen`] ([`Model::screen`]).

use nsh_core::{
    AccessibilityRole, EventKind, NodeId, ROOT_ID as ROOT, ViewKind, desktop::MenuRole,
};
use nsh_testing::{MountedView, Screen, agent::Model};

use crate::{SemanticKeybinding, SemanticNode, SemanticScene, SemanticTarget};

impl SemanticScene for Model {
    fn semantic_tree(&self) -> Result<Vec<SemanticNode>, String> {
        Ok(self.screen(nodes))
    }

    fn keybindings(&self) -> Result<Vec<SemanticKeybinding>, String> {
        Ok(self.screen(keybindings))
    }

    fn long_press_disabled(&self, id: NodeId) -> Result<bool, String> {
        Ok(self.screen(|screen| long_press_disabled(screen, id)))
    }

    fn native_hold_receiver(&self, id: NodeId) -> Result<Option<NodeId>, String> {
        Ok(self.screen(|screen| native_hold_receiver(screen, id)))
    }
}

/// [`SemanticScene::native_hold_receiver`] on `screen`.
fn native_hold_receiver(screen: &Screen, id: NodeId) -> Option<NodeId> {
    let mut at = Some(id);
    while let Some(view_id) = at {
        let view = screen.mounted(view_id)?;
        // A raw touch observer installs no recognizer: a hold passes it to
        // the nearest ancestor with a press listener — the same
        // `nsh_core::input::has_press` set native routing picks (Press,
        // LongPress, PressDown, PressUp), so a touch observer that also
        // hears press-down/up takes the press and blocks the hold. A
        // disabled view's recognizer never fires either.
        if !view.disabled() && nsh_core::input::has_press(view.events()) {
            return Some(view_id);
        }
        at = view.parent();
    }
    None
}

/// [`SemanticScene::long_press_disabled`] on `screen`.
fn long_press_disabled(screen: &Screen, id: NodeId) -> bool {
    let listens = |view: &MountedView<'_>| {
        let events = view.events();
        events.contains(EventKind::LongPress)
            || events.contains(EventKind::Press)
            || events.contains(EventKind::Touch)
    };
    let mut at = id;
    let receiver = loop {
        let Some(view) = screen.mounted(at) else {
            return false;
        };
        if listens(&view) {
            break at;
        }
        let Some(parent) = view.parent() else {
            return false;
        };
        at = parent;
    };
    let mut at = id;
    loop {
        let Some(view) = screen.mounted(at) else {
            return false;
        };
        if view.disabled() {
            return true;
        }
        if at == receiver {
            return false;
        }
        let Some(parent) = view.parent() else {
            return false;
        };
        at = parent;
    }
}

fn semantic_role(role: AccessibilityRole, kind: Option<ViewKind>, pressable: bool) -> &'static str {
    match role {
        AccessibilityRole::None => match kind {
            Some(ViewKind::Text) => "text",
            Some(ViewKind::Input) => "textbox",
            Some(ViewKind::Scroll) => "scroll",
            _ if pressable => "button",
            _ => "generic",
        },
        AccessibilityRole::Button => "button",
        AccessibilityRole::Link => "link",
        AccessibilityRole::Search => "search",
        AccessibilityRole::Image => "image",
        AccessibilityRole::ImageButton => "imagebutton",
        AccessibilityRole::KeyboardKey => "keyboardkey",
        AccessibilityRole::Text => "text",
        AccessibilityRole::Adjustable => "adjustable",
        AccessibilityRole::Header => "header",
        AccessibilityRole::Summary => "summary",
        AccessibilityRole::Alert => "alert",
        AccessibilityRole::Checkbox => "checkbox",
        AccessibilityRole::Combobox => "combobox",
        AccessibilityRole::Menu => "menu",
        AccessibilityRole::Menubar => "menubar",
        AccessibilityRole::Menuitem => "menuitem",
        AccessibilityRole::Progressbar => "progressbar",
        AccessibilityRole::Radio => "radio",
        AccessibilityRole::Radiogroup => "radiogroup",
        AccessibilityRole::Scrollbar => "scrollbar",
        AccessibilityRole::Spinbutton => "spinbutton",
        AccessibilityRole::Switch => "switch",
        AccessibilityRole::Tab => "tab",
        AccessibilityRole::Tablist => "tablist",
        AccessibilityRole::Timer => "timer",
        AccessibilityRole::Toolbar => "toolbar",
        AccessibilityRole::List => "list",
        AccessibilityRole::Grid => "grid",
        AccessibilityRole::Table => "table",
        AccessibilityRole::Row => "row",
        AccessibilityRole::Cell => "cell",
        AccessibilityRole::ColumnHeader => "columnheader",
        AccessibilityRole::RowGroup => "rowgroup",
        AccessibilityRole::Listbox => "listbox",
        AccessibilityRole::Option => "option",
        AccessibilityRole::Group => "group",
        AccessibilityRole::Region => "region",
        AccessibilityRole::Navigation => "navigation",
        AccessibilityRole::Tree => "tree",
        AccessibilityRole::TreeItem => "treeitem",
        AccessibilityRole::MenuItemCheckbox => "menuitemcheckbox",
        AccessibilityRole::MenuItemRadio => "menuitemradio",
        AccessibilityRole::Dialog => "dialog",
        AccessibilityRole::AlertDialog => "alertdialog",
    }
}

pub(crate) fn nodes(screen: &Screen) -> Vec<SemanticNode> {
    let mut nodes: Vec<SemanticNode> = Vec::new();
    // Where each node is in `nodes`, for the hidden views under it.
    let mut at = std::collections::HashMap::new();
    // The last element: in a hidden subtree, the innermost hidden view
    // above that takes presses.
    let mut stack = vec![(ROOT, None, false, false, None)];
    while let Some((id, parent, inherited_hidden, inherited_disabled, hidden_press)) = stack.pop() {
        let Some(view) = screen.mounted(id) else {
            continue;
        };
        let hidden = inherited_hidden || view.hidden() || view.accessibility_hidden();
        let disabled = inherited_disabled || view.disabled();
        let children: Vec<NodeId> = if view.kind() == Some(ViewKind::Stack) {
            screen
                .stack_screens(id)
                .last()
                .copied()
                .into_iter()
                .collect()
        } else {
            screen.children(id).to_vec()
        };
        if hidden {
            // A press on it goes to it, or to the hidden view around
            // it that takes presses, not to `parent`.
            let hidden_press = if view.events().contains(EventKind::Press) {
                Some(id)
            } else {
                hidden_press
            };
            // Left out, but still under `parent` (the nearest node in
            // the tree) for native hits.
            if let Some(owner) = parent.and_then(|parent| at.get(&parent).copied()) {
                let owner: &mut SemanticNode = &mut nodes[owner];
                owner.hidden_descendants.push(id);
                if let Some(receiver) = hidden_press {
                    owner.hidden_press_owners.push((id, receiver));
                }
            }
            stack.extend(
                children
                    .into_iter()
                    .rev()
                    .map(|child| (child, parent, true, disabled, hidden_press)),
            );
            continue;
        }
        let role = semantic_role(
            view.role(),
            view.kind(),
            view.events().contains(EventKind::Press),
        );
        let masked = view.secure();
        // A masked input's text never leaves: a bullet per character,
        // as platform secure fields show it.
        let input_value = view.input_text().map(|text| {
            if masked {
                "\u{2022}".repeat(text.chars().count())
            } else {
                text.to_owned()
            }
        });
        let value = input_value.or_else(|| {
            view.accessibility_value().and_then(|value| {
                value
                    .text
                    .as_deref()
                    .map(str::to_owned)
                    .or_else(|| value.now.map(|number| number.to_string()))
            })
        });
        let state = view.accessibility_state();
        let mut states = Vec::new();
        if view.disabled() {
            states.push("disabled".to_owned());
        }
        if view.selected() {
            states.push("selected".to_owned());
        }
        if masked {
            states.push("masked".to_owned());
        }
        if let Some(checked) = state.checked {
            states.push(
                match checked {
                    nsh_core::Checked::False => "unchecked",
                    nsh_core::Checked::True => "checked",
                    nsh_core::Checked::Mixed => "mixed",
                }
                .to_owned(),
            );
        }
        if state.busy {
            states.push("busy".to_owned());
        }
        if let Some(expanded) = state.expanded {
            states.push(if expanded { "expanded" } else { "collapsed" }.to_owned());
        }
        at.insert(id, nodes.len());
        nodes.push(SemanticNode {
            id,
            parent,
            test_id: view.test_id().map(str::to_owned),
            role: role.to_owned(),
            label: view
                .label()
                .map(str::to_owned)
                .or_else(|| view.text().map(str::to_owned))
                .or_else(|| view.span().map(|span| span.text.to_owned())),
            hint: view.accessibility_hint().map(str::to_owned),
            value,
            enabled: !disabled,
            selected: view.selected(),
            grouped: view.accessible() == Some(true),
            clipping: view.clips() || view.scrolls(),
            input: view.kind() == Some(ViewKind::Input),
            scroll: view.kind() == Some(ViewKind::Scroll) || view.scrolls(),
            pressable: view.events().contains(EventKind::Press),
            states,
            native_target: if view.canvas_node() {
                view.parent()
                    .map_or_else(SemanticTarget::default, |owner| SemanticTarget::Canvas {
                        owner,
                        bounds: view.frame(),
                    })
            } else if let Some(span) = view.span() {
                SemanticTarget::Text {
                    owner: span.view,
                    start: span.start,
                    end: span.end,
                }
            } else {
                SemanticTarget::View
            },
            hidden_descendants: Vec::new(),
            hidden_press_owners: Vec::new(),
        });
        stack.extend(
            children
                .into_iter()
                .rev()
                .map(|child| (child, Some(id), hidden, disabled, None)),
        );
    }
    nodes
}

pub(crate) fn keybindings(screen: &Screen) -> Vec<SemanticKeybinding> {
    let mut bindings = Vec::new();
    if let Some(keymap) = screen.keymap() {
        bindings.extend(keymap.bindings().iter().map(|binding| {
            let chord = binding.keystroke.to_string();
            SemanticKeybinding {
                id: format!("keymap.{}.{}", binding.id, chord.replace('+', "-")),
                chord,
                ..SemanticKeybinding::default()
            }
        }));
    }
    if let Some(menu) = screen.app_menu() {
        let mut path = Vec::new();
        menu_keybindings(menu, &mut path, &mut bindings);
    }
    bindings
}

fn menu_keybindings(
    items: &[nsh_core::desktop::MenuItem],
    path: &mut Vec<String>,
    out: &mut Vec<SemanticKeybinding>,
) {
    use nsh_core::desktop::MenuItem;

    for item in items {
        match item {
            MenuItem::Submenu {
                label,
                enabled: true,
                items,
                ..
            } => {
                path.push(label.clone());
                menu_keybindings(items, path, out);
                path.pop();
            }
            MenuItem::Action(action) if action.enabled => {
                let Some(shortcut) = action.shortcut else {
                    continue;
                };
                let mut keys: Vec<String> = Vec::new();
                if shortcut.ctrl {
                    keys.push("ctrl".to_owned());
                }
                if shortcut.alt {
                    keys.push("alt".to_owned());
                }
                if shortcut.shift {
                    keys.push("shift".to_owned());
                }
                if shortcut.meta {
                    keys.push("cmd".to_owned());
                }
                keys.push(shortcut.key.label().to_lowercase());
                let chord = keys.join("+");
                let mut menu_path = path.clone();
                menu_path.push(action.label.clone());
                let id = format!("menu.{}#{}", menu_path.join("/"), action.id);
                let label = if action.label.is_empty() {
                    action.role.map_or("", MenuRole::label)
                } else {
                    &action.label
                };
                out.push(SemanticKeybinding {
                    id,
                    chord,
                    label: Some(label.to_owned()),
                    menu_path: Some(menu_path),
                    menu_action_id: Some(action.id),
                });
            }
            MenuItem::Separator | MenuItem::Submenu { .. } | MenuItem::Action(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nsh_core::{
        Bounds, Command, EventMask, InputConfig, MountBatch, MountItem, ViewProps,
        desktop::{DesktopCommand, MenuAction, MenuItem, Shortcut},
        geometry::{Point, Points, Size},
        keymap::When,
    };

    use super::*;

    fn batch(items: Vec<MountItem>) -> MountBatch {
        MountBatch { items }
    }

    fn create(id: NodeId, kind: ViewKind) -> MountItem {
        MountItem::Create { id, kind }
    }

    fn insert(parent: NodeId, child: NodeId) -> MountItem {
        MountItem::Insert {
            parent,
            child,
            before: None,
        }
    }

    fn props(id: NodeId, change: impl FnOnce(&mut ViewProps)) -> MountItem {
        let mut props = ViewProps::default();
        change(&mut props);
        MountItem::ViewProps {
            id,
            props: Box::new(props),
        }
    }

    #[test]
    fn semantic_keybindings_follow_application_menu_commands_not_popup_lifecycle() {
        let action = |id, label: &str, shortcut: &str, enabled| {
            MenuItem::Action(MenuAction {
                id,
                label: label.into(),
                shortcut: Some(Shortcut::parse(shortcut, false).unwrap()),
                enabled,
                checked: None,
                role: None,
                when: When::Auto,
                icon: None,
                symbol: None,
                destructive: false,
            })
        };
        let application_menu: Arc<[MenuItem]> = vec![MenuItem::Submenu {
            label: "File".into(),
            enabled: true,
            icon: None,
            symbol: None,
            inline: false,
            items: vec![
                action(19, "Save", "ctrl+s", true),
                MenuItem::Submenu {
                    label: "Disabled".into(),
                    enabled: false,
                    icon: None,
                    symbol: None,
                    inline: false,
                    items: vec![action(20, "Hidden", "ctrl+h", true)],
                },
                action(21, "Unavailable", "ctrl+u", false),
            ],
        }]
        .into();
        let replacement: Arc<[MenuItem]> = vec![action(22, "Open", "ctrl+o", true)].into();
        let popup: Arc<[MenuItem]> = vec![action(23, "Popup only", "ctrl+p", true)].into();
        let set_menu = |menu| MountItem::Command {
            id: ROOT,
            command: Command::Desktop(Box::new(DesktopCommand::SetMenu(menu))),
        };
        let popup_menu = || MountItem::Command {
            id: ROOT,
            command: Command::Desktop(Box::new(DesktopCommand::PopupMenu {
                items: popup.clone(),
                at: None,
            })),
        };
        let mut screen = Screen::default();
        screen.apply(&batch(vec![set_menu(Some(application_menu))]));
        let bindings = keybindings(&screen);
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].id, "menu.File/Save#19");
        assert_eq!(bindings[0].chord, "ctrl+s");
        assert_eq!(
            bindings[0].menu_path.as_deref(),
            Some(&["File".to_owned(), "Save".to_owned()][..])
        );
        assert_eq!(bindings[0].menu_action_id, Some(19));
        // A context menu is not the application menu.
        screen.apply(&batch(vec![popup_menu()]));
        assert_eq!(keybindings(&screen), bindings);
        screen.apply(&batch(vec![set_menu(Some(replacement))]));
        let bindings = keybindings(&screen);
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].id, "menu.Open#22");
        screen.apply(&batch(vec![popup_menu(), set_menu(None)]));
        assert!(keybindings(&screen).is_empty());
    }

    #[test]
    fn semantic_targets_preserve_canvas_bounds_and_text_span_offsets() {
        let cell_bounds = Bounds::new(Point::default(), Size::new(Points(100.0), Points(40.0)));
        let mut canvas = Screen::default();
        canvas.apply(&batch(vec![
            create(2, ViewKind::View),
            create(4, ViewKind::Canvas),
            insert(1, 2),
            insert(2, 4),
            MountItem::Canvas {
                id: 4,
                drawing: Arc::new(nsh_core::canvas::Drawing {
                    nodes: vec![
                        nsh_core::canvas::CanvasNode::new(
                            cell_bounds,
                            AccessibilityRole::Text,
                            "cell",
                        )
                        .test_id("cell"),
                    ],
                    ..Default::default()
                }),
            },
        ]));
        let cell = nodes(&canvas)
            .into_iter()
            .find(|node| node.test_id.as_deref() == Some("cell"))
            .unwrap();
        assert_eq!(
            cell.native_target,
            SemanticTarget::Canvas {
                owner: 4,
                bounds: cell_bounds,
            }
        );

        let mut screen = Screen::default();
        let link = nsh_core::TextLink {
            start: 0,
            end: 7,
            node: 102,
            events: EventMask::default(),
            focus_stop: false,
        };
        screen.apply(&batch(vec![
            create(4, ViewKind::Text),
            insert(1, 4),
            MountItem::Text {
                id: 4,
                text: Arc::new(nsh_core::AttributedText {
                    text: "Privacy".into(),
                    links: vec![link],
                    ..Default::default()
                }),
                layout: nsh_core::TextLayout(Arc::new(())),
                content: Bounds::default(),
            },
        ]));
        let span = nodes(&screen)
            .into_iter()
            .find(|node| node.id == 102)
            .unwrap();
        assert_eq!(span.label.as_deref(), Some("Privacy"));
        assert_eq!(
            span.native_target,
            SemanticTarget::Text {
                owner: 4,
                start: 0,
                end: 7,
            }
        );
        let mut legacy = serde_json::to_value(SemanticNode::default()).unwrap();
        legacy.as_object_mut().unwrap().remove("native_target");
        let legacy: SemanticNode = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.native_target, SemanticTarget::View);
    }

    #[test]
    fn semantic_tree_keeps_offscreen_nodes_and_omits_hidden_subtrees() {
        let mut screen = Screen::default();
        screen.apply(&batch(vec![
            create(10, ViewKind::View),
            create(11, ViewKind::View),
            create(12, ViewKind::View),
            create(13, ViewKind::View),
            insert(ROOT, 10),
            insert(ROOT, 12),
            insert(10, 11),
            insert(12, 13),
            props(10, |props| {
                props.test_id = Some("group".into());
                props.accessibility.accessible = Some(true);
                props.accessibility.state.disabled = true;
            }),
            props(11, |props| props.test_id = Some("offscreen".into())),
            props(12, |props| props.accessibility.hidden = true),
        ]));

        let nodes = nodes(&screen);
        assert_eq!(
            nodes.iter().map(|node| node.id).collect::<Vec<_>>(),
            [ROOT, 10, 11]
        );
        assert!(nodes[1].grouped);
        assert!(!nodes[1].enabled);
        assert!(!nodes[2].enabled);
        assert_eq!(nodes[2].parent, Some(10));
        // Left out, the hidden views still link to the node they are under.
        assert_eq!(nodes[0].hidden_descendants, [12, 13]);
        assert!(nodes[1].hidden_descendants.is_empty());
    }

    #[test]
    fn semantic_tree_masks_a_masked_input_value() {
        let mut screen = Screen::default();
        for (id, secure) in [(2, true), (3, false)] {
            let config = InputConfig {
                secure,
                ..Default::default()
            };
            screen.apply(&batch(vec![
                create(id, ViewKind::Input),
                MountItem::Input {
                    id,
                    config: Box::new(config),
                },
                MountItem::InputValue {
                    id,
                    text: "hunter2é".into(),
                    count: 0,
                },
                insert(1, id),
            ]));
        }
        let nodes = nodes(&screen);
        let node = |id| nodes.iter().find(|node| node.id == id).unwrap();
        assert_eq!(node(2).value.as_deref(), Some("••••••••"));
        assert!(node(2).states.iter().any(|state| state == "masked"));
        assert_eq!(node(3).value.as_deref(), Some("hunter2é"));
        assert!(!node(3).states.iter().any(|state| state == "masked"));
    }

    #[test]
    fn semantic_roles_are_inferred_from_view_kind_and_events() {
        assert_eq!(
            semantic_role(AccessibilityRole::None, Some(ViewKind::Text), false),
            "text"
        );
        assert_eq!(
            semantic_role(AccessibilityRole::None, Some(ViewKind::Input), false),
            "textbox"
        );
        assert_eq!(
            semantic_role(AccessibilityRole::None, Some(ViewKind::Scroll), false),
            "scroll"
        );
        assert_eq!(
            semantic_role(AccessibilityRole::None, Some(ViewKind::View), true),
            "button"
        );
        assert_eq!(
            semantic_role(AccessibilityRole::Button, Some(ViewKind::Text), false),
            "button"
        );
    }
}
