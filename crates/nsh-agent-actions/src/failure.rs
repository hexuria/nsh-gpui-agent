//! The failure messages of the agent's actions, worded as `nsh test`'s
//! agent words them (`nsh-testing`'s `failure` module), so a client sees
//! the same text from both.

/// No `what` (`"view"`, `"pressable view"`, `"input"`, ...) matches
/// `selector`.
pub(crate) fn not_found(what: &str, selector: impl std::fmt::Display) -> String {
    format!("no {what} matches `{selector}`")
}

/// The `what` `selector` names takes no step: it is disabled.
pub(crate) fn disabled(selector: impl std::fmt::Display, what: &str) -> String {
    if what == "scroll view" {
        format!("`{selector}` does not scroll: the user cannot scroll it (`scrollable` is false)")
    } else {
        format!("`{selector}` is disabled")
    }
}

/// The view `selector` names is under `cover`, which a press there
/// reaches instead.
pub(crate) fn covered(selector: impl std::fmt::Display, cover: &str) -> String {
    format!("`{selector}` is covered: a press there reaches {cover}")
}
