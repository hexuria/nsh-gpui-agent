//! Android support for the GPUI-agent plug: a load-time surface hook, the
//! runtime Java bridge and the platform's strict native driver.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
mod native;

use nsh_core::prelude::*;

/// The crate's UI set entry. Registering it in an app keeps this crate in
/// the generated Android library; the constructor below installs the hook
/// when that library is loaded.
#[native_module("gpui-agent/android")]
fn native(_registry: &mut Registry) {
    #[cfg(target_os = "android")]
    android::register();
}

/// Installs the surface hook as the shared library is loaded. Waiting for
/// the native-module entry would be too late: the development host asks its
/// hook before it builds the session that enumerates UI sets.
#[cfg(target_os = "android")]
#[ctor::ctor(unsafe)]
fn install() {
    android::register();
}
