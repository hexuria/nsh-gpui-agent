//! Loading the Java bridge and attaching a worker to an opened surface.

use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JClass, JObjectArray, JString, JValue},
};
use nsh::AndroidSurface;
use nsh_agent_actions::SemanticScene;
use nsh_gpui_agent::{NshAgentHost, PlatformKind, WorkerHandle, spawn_worker};
use nsh_testing::agent::{Model, OpenedSurface, SurfaceAttachment};

use crate::native::AndroidStrictNative;

/// The dex bytecode `build.rs` compiled from `AgentBridge.java`.
const DEX: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/classes.dex"));
const BRIDGE_CLASS: &str = "com.longbridge.nsh.agent.AgentBridge";

/// The bridge's class and loader in the Java VM it was loaded into.
pub(crate) struct Bridge {
    vm: JavaVM,
    class: GlobalRef,
    _loader: GlobalRef,
}

pub(crate) type SharedBridge = Arc<Bridge>;

impl Bridge {
    /// Loads the bridge once in this process, in a child of the surface's
    /// class loader. NSH's minimum SDK is 26, where
    /// `InMemoryDexClassLoader` is available.
    fn load(surface: &AndroidSurface) -> Result<SharedBridge, String> {
        // SAFETY: the VM pointer belongs to the process's live Java VM;
        // `JavaVM::from_raw` only wraps it and does not destroy it.
        let vm = unsafe { JavaVM::from_raw(surface.java_vm()) }.map_err(|e| e.to_string())?;
        let (class, loader) = {
            let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
            // SAFETY: `DEX` is immutable static memory and outlives every use of
            // the direct buffer. The buffer is only the input of the constructor.
            let buffer = unsafe { env.new_direct_byte_buffer(DEX.as_ptr() as *mut u8, DEX.len()) }
                .map_err(|e| e.to_string())?;
            let view_class = env
                .get_object_class(surface.view().as_obj())
                .map_err(|e| e.to_string())?;
            let parent = env
                .call_method(
                    view_class,
                    "getClassLoader",
                    "()Ljava/lang/ClassLoader;",
                    &[],
                )
                .and_then(|loader| loader.l())
                .map_err(|e| e.to_string())?;
            let loader = env
                .new_object(
                    "dalvik/system/InMemoryDexClassLoader",
                    "(Ljava/nio/ByteBuffer;Ljava/lang/ClassLoader;)V",
                    &[JValue::Object(&buffer), JValue::Object(&parent)],
                )
                .map_err(|e| e.to_string())?;
            let name = env.new_string(BRIDGE_CLASS).map_err(|e| e.to_string())?;
            let class = env
                .call_method(
                    &loader,
                    "loadClass",
                    "(Ljava/lang/String;)Ljava/lang/Class;",
                    &[JValue::Object(&name)],
                )
                .and_then(|class| class.l())
                .map_err(|e| e.to_string())?;
            if class.is_null() {
                return Err(format!("{BRIDGE_CLASS} did not load"));
            }
            (
                env.new_global_ref(&class).map_err(|e| e.to_string())?,
                env.new_global_ref(&loader).map_err(|e| e.to_string())?,
            )
        };
        Ok(Arc::new(Bridge {
            vm,
            class,
            _loader: loader,
        }))
    }

    /// The bridge once for the process, or the first load's error.
    fn shared(surface: &AndroidSurface) -> Result<SharedBridge, String> {
        static BRIDGE: OnceLock<Result<SharedBridge, String>> = OnceLock::new();
        BRIDGE.get_or_init(|| Bridge::load(surface)).clone()
    }

    /// Runs `f` with this thread attached to the Java VM. JNI errors become
    /// the caller's default, as the platform's own test driver does.
    pub(crate) fn with_env<R: Default>(
        &self,
        f: impl FnOnce(&mut JNIEnv<'_>) -> jni::errors::Result<R>,
    ) -> R {
        let Ok(mut env) = self.vm.attach_current_thread() else {
            return R::default();
        };
        f(&mut env).unwrap_or_default()
    }

    /// The loaded `AgentBridge` class.
    pub(crate) fn class(&self) -> &GlobalRef {
        &self.class
    }

    /// Reads the host activity's agent configuration: address, token and
    /// screenshot directory. `None` means no agent was requested or allowed.
    fn agent_config(&self, view: &GlobalRef) -> Result<Option<AgentConfig>, String> {
        let mut env = self.vm.attach_current_thread().map_err(|e| e.to_string())?;
        let class: &JClass<'_> = self.class.as_obj().into();
        let array = env
            .call_static_method(
                class,
                "agentConfig",
                "(Landroid/view/View;)[Ljava/lang/String;",
                &[JValue::Object(view.as_obj())],
            )
            .and_then(|array| array.l())
            .map_err(|e| e.to_string())?;
        if array.is_null() {
            return Ok(None);
        }
        let array = JObjectArray::from(array);
        if env.get_array_length(&array).map_err(|e| e.to_string())? < 3 {
            return Err("AgentBridge returned an incomplete configuration".into());
        }
        let mut string_at = |index| -> Result<String, String> {
            let object = env
                .get_object_array_element(&array, index)
                .map_err(|e| e.to_string())?;
            let object = JString::from(object);
            let string = env.get_string(&object).map_err(|e| e.to_string())?;
            Ok(String::from(string))
        };
        Ok(Some(AgentConfig {
            address: string_at(0)?,
            token: string_at(1)?,
            cache_dir: string_at(2)?,
        }))
    }
}

struct AgentConfig {
    address: String,
    token: String,
    cache_dir: String,
}

/// What keeps the worker alive and stops the strict driver when the
/// surface closes.
struct Attachment {
    alive: Arc<AtomicBool>,
    _worker: WorkerHandle,
}

impl Drop for Attachment {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

/// Installs the surface hook once. A second registration (the constructor
/// and the UI-set entry both run) is harmless.
pub(crate) fn register() {
    let _ = nsh_testing::agent::set_surface_hook(surface_hook);
}

fn surface_hook(opened: &OpenedSurface<'_>) -> Option<SurfaceAttachment> {
    let surface = opened.surface.downcast_ref::<AndroidSurface>()?;
    match attach(surface) {
        Ok(attachment) => attachment,
        Err(error) => {
            log::warn!("GPUI-agent attachment failed: {error}");
            None
        }
    }
}

fn attach(surface: &AndroidSurface) -> Result<Option<SurfaceAttachment>, String> {
    let bridge = Bridge::shared(surface)?;
    let Some(config) = bridge.agent_config(surface.view())? else {
        return Ok(None);
    };
    let address: SocketAddr = config
        .address
        .parse()
        .map_err(|error| format!("GPUI_AGENT_ADDR must be a socket address: {error}"))?;
    let model = Arc::new(Model::default());
    let alive = Arc::new(AtomicBool::new(true));
    let native = AndroidStrictNative::new(
        surface.native(),
        bridge,
        surface.view().clone(),
        surface.density(),
        alive.clone(),
    );
    let scene: Arc<dyn SemanticScene> = model.clone();
    let agent = Arc::new(nsh_agent_actions::Agent::requiring_native_hit_test(
        scene,
        Box::new(native),
    ));
    let host = NshAgentHost::new("Native Shell", PlatformKind::Mobile, agent)?
        .with_screenshot_directory(PathBuf::from(config.cache_dir));
    let worker = spawn_worker(address, config.token, host)?;
    Ok(Some(SurfaceAttachment::new(
        Some(model),
        Box::new(Attachment {
            alive,
            _worker: worker,
        }),
    )))
}
