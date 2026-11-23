//! macOS development-surface integration for NSH's gpui-agent.
//!
//! Empty off macOS. Call [`install`] before opening any development
//! surfaces. In a debug build with feature `dev-agent`, `GPUI_AGENT=1` and
//! the upstream token/address settings give each surface a strict adapter
//! worker. Feature `gpui-agent-server` compiles buwiz's in-process server
//! instead; as on buwiz, it owns the listener when both are enabled.
//! `gpui-agent` alone provides the in-process tree and action API.

#![cfg(target_os = "macos")]

use std::collections::HashMap;
use std::sync::{
    Arc, LazyLock, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};

#[cfg(all(
    feature = "dev-agent",
    not(feature = "gpui-agent-server"),
    debug_assertions
))]
use nsh_agent_actions::Agent;
use nsh_gpui_agent::WorkerHandle;
#[cfg(all(
    feature = "dev-agent",
    not(feature = "gpui-agent-server"),
    debug_assertions
))]
use nsh_gpui_agent::{NshAgentHost, PlatformKind};
#[cfg(feature = "gpui-agent")]
use nsh_platform_macos::surface::Surface;
use nsh_platform_macos::surface::SurfaceHandle;
use nsh_testing::agent::{Model, OpenedSurface, SurfaceAttachment};

static MODELS: LazyLock<Mutex<HashMap<u64, Weak<Model>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[doc(hidden)]
pub use crate::strict::MacStrictNative;

pub mod provenance;

pub use provenance::{Input, on_input};
mod strict;

#[cfg(feature = "gpui-agent")]
#[cfg_attr(not(feature = "gpui-agent-server"), allow(dead_code, unused_imports))]
mod gpui_agent;

#[cfg(feature = "gpui-agent")]
pub use crate::gpui_agent::{SCREEN_CHANGED, UiAct};
#[cfg(feature = "gpui-agent")]
pub use ::gpui_agent::tree as ui_tree;

/// Installs the development surface hook before any macOS surfaces open.
///
/// A second installer is ignored, matching the platform host integrations:
/// the hook registry accepts one tool for the process.
pub fn install() {
    let _ = nsh_testing::agent::set_surface_hook(opened);
}

fn opened(opened: &OpenedSurface<'_>) -> Option<SurfaceAttachment> {
    let handle = opened.surface.downcast_ref::<SurfaceHandle>()?;
    let key = handle.key();

    // The legacy private server is independent of the strict adapter worker.
    // It reads GPUI_AGENT itself and does nothing unless its server terms are
    // present, exactly as on buwiz.
    #[cfg(feature = "gpui-agent-server")]
    if let Some(surface) = Surface::with_key(key) {
        crate::gpui_agent::install(&surface);
    }

    let model = Arc::new(Model::default());
    let mut models = MODELS.lock().unwrap_or_else(|e| e.into_inner());
    models.retain(|_, model| model.strong_count() > 0);
    models.insert(key, Arc::downgrade(&model));
    let alive = Arc::new(AtomicBool::new(true));
    let worker = start_strict_worker(key, model.clone(), alive.clone());
    Some(SurfaceAttachment::new(
        Some(model),
        Box::new(Attachment {
            alive,
            _worker: worker,
        }),
    ))
}

#[cfg(all(
    feature = "dev-agent",
    not(feature = "gpui-agent-server"),
    debug_assertions
))]
fn start_strict_worker(
    key: u64,
    model: Arc<Model>,
    alive: Arc<AtomicBool>,
) -> Option<WorkerHandle> {
    let native = MacStrictNative::new(key, alive);
    let agent = Arc::new(Agent::requiring_native_hit_test(model, Box::new(native)));
    match NshAgentHost::new("Native Shell", PlatformKind::Desktop, agent) {
        Ok(host) => match nsh_gpui_agent::start_from_env(host) {
            Ok(worker) => worker,
            Err(error) => {
                log::error!("invalid GPUI_AGENT development configuration: {error}");
                None
            }
        },
        Err(error) => {
            log::error!("cannot construct the GPUI-agent development host: {error}");
            None
        }
    }
}

#[cfg(not(all(
    feature = "dev-agent",
    not(feature = "gpui-agent-server"),
    debug_assertions
)))]
fn start_strict_worker(
    _key: u64,
    _model: Arc<Model>,
    _alive: Arc<AtomicBool>,
) -> Option<WorkerHandle> {
    None
}

/// The batch count and pending-work bit the legacy server settles on.
///
/// Upstream keeps the pending bit inside `Surface`; it cannot be a pure
/// door. The plug uses the hook model for batch revisions and treats the
/// unavailable pending bit as quiet.
#[cfg(feature = "gpui-agent")]
pub(crate) fn model_activity(key: u64) -> (u64, bool) {
    use nsh_testing::agent::Scene;
    let model = MODELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .and_then(Weak::upgrade);
    let batches = model
        .and_then(|model| model.activity().ok().map(|activity| activity.batches))
        .unwrap_or(0);
    (batches, false)
}

/// The semantic model the surface hook attached, for the crate's native
/// integration checks.
#[doc(hidden)]
pub fn surface_model(key: u64) -> Option<Arc<Model>> {
    MODELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .and_then(Weak::upgrade)
}

struct Attachment {
    alive: Arc<AtomicBool>,
    _worker: Option<WorkerHandle>,
}

impl Drop for Attachment {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

/// In-process access buwiz exposed as inherent `AppView` methods.
///
/// NSH's orphan rules do not let the plug add inherent methods to upstream's
/// `AppView`, so they are this extension trait. Importing it preserves the
/// call shape (`view.ui_tree()` and `view.act(...)`).
#[cfg(feature = "gpui-agent")]
pub trait AppViewAgentExt {
    /// The semantic tree of this view's surface, as the private server's
    /// `snapshot` operation answers it. Main thread.
    fn ui_tree(&self) -> ui_tree::UiTree;

    /// Acts on a node of [`Self::ui_tree`] as an agent, without opening a
    /// listener. Main thread.
    fn act(
        &self,
        target: &str,
        act: UiAct,
        still: impl Fn() -> bool + Send + 'static,
        done: impl FnOnce(Result<(), String>) + Send + 'static,
    );
}

#[cfg(feature = "gpui-agent")]
impl AppViewAgentExt for nsh_platform_macos::AppView {
    fn ui_tree(&self) -> ui_tree::UiTree {
        let surface = Surface::with_key(self.surface_handle().key())
            .expect("an AppView's surface outlives the view");
        crate::gpui_agent::snapshot(&surface)
    }

    fn act(
        &self,
        target: &str,
        act: UiAct,
        still: impl Fn() -> bool + Send + 'static,
        done: impl FnOnce(Result<(), String>) + Send + 'static,
    ) {
        let surface = Surface::with_key(self.surface_handle().key())
            .expect("an AppView's surface outlives the view");
        crate::gpui_agent::act_in_process(&surface, target, act, Box::new(still), done);
    }
}
