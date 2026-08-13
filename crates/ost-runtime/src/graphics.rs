// SPDX-License-Identifier: Apache-2.0
//! Host graphics-loader probes for normalized OpenUSD runtimes.
//!
//! These checks deliberately stop at loading the API loader. They do not infer
//! that a physical device exists or that OpenUSD can render a frame; those are
//! separate verification stages.

use std::ffi::{c_void, CString};

use ost_platform::OpenUsdVerificationStatus;

/// A graphics API whose loader is required by an OpenUSD compatibility cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsApi {
    OpenGl,
    Vulkan,
}

impl GraphicsApi {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenGl => "opengl",
            Self::Vulkan => "vulkan",
        }
    }
}

/// The observed result of loading one host graphics API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphicsLoaderProbe {
    pub api: GraphicsApi,
    pub passed: bool,
    pub detail: String,
}

/// Probe only the graphics loaders named by a normalized capability set.
///
/// An empty result means the compatibility cell requires no graphics loader.
/// Keeping that distinct from a successful probe prevents a headless build
/// from being stamped as loader-verified without loading anything.
pub fn probe_graphics_loaders(capabilities: &[String]) -> Vec<GraphicsLoaderProbe> {
    required_graphics_apis(capabilities)
        .into_iter()
        .map(probe_graphics_loader)
        .collect()
}

/// Reduce actual loader observations to the independent verification field.
///
/// No required probes means no observation (`None`), not success. Any failed
/// required API fails the loader stage, while physical-device and render state
/// remain outside this function by construction.
pub fn graphics_loader_status(probes: &[GraphicsLoaderProbe]) -> Option<OpenUsdVerificationStatus> {
    if probes.is_empty() {
        None
    } else if probes.iter().all(|probe| probe.passed) {
        Some(OpenUsdVerificationStatus::Passed)
    } else {
        Some(OpenUsdVerificationStatus::Failed)
    }
}

fn required_graphics_apis(capabilities: &[String]) -> Vec<GraphicsApi> {
    let mut required = Vec::new();
    if capabilities.iter().any(|value| value == "opengl") {
        required.push(GraphicsApi::OpenGl);
    }
    if capabilities.iter().any(|value| value == "vulkan") {
        required.push(GraphicsApi::Vulkan);
    }
    required
}

fn probe_graphics_loader(api: GraphicsApi) -> GraphicsLoaderProbe {
    let candidates = loader_candidates(api);
    let symbol = required_loader_symbol(api);
    let mut failures = Vec::new();
    for candidate in candidates {
        match load_library(candidate, symbol) {
            Ok(()) => {
                return GraphicsLoaderProbe {
                    api,
                    passed: true,
                    detail: format!(
                        "loaded host {api_name} loader '{candidate}' and resolved '{symbol}'",
                        api_name = api.as_str()
                    ),
                };
            }
            Err(error) => failures.push(format!("{candidate}: {error}")),
        }
    }
    GraphicsLoaderProbe {
        api,
        passed: false,
        detail: format!(
            "could not load a host {} loader ({})",
            api.as_str(),
            failures.join("; ")
        ),
    }
}

fn required_loader_symbol(api: GraphicsApi) -> &'static str {
    match api {
        GraphicsApi::OpenGl => "glGetString",
        GraphicsApi::Vulkan => "vkGetInstanceProcAddr",
    }
}

#[cfg(target_os = "windows")]
fn loader_candidates(api: GraphicsApi) -> &'static [&'static str] {
    match api {
        GraphicsApi::OpenGl => &["opengl32.dll"],
        GraphicsApi::Vulkan => &["vulkan-1.dll"],
    }
}

#[cfg(target_os = "linux")]
fn loader_candidates(api: GraphicsApi) -> &'static [&'static str] {
    match api {
        GraphicsApi::OpenGl => &["libOpenGL.so.0", "libGL.so.1"],
        GraphicsApi::Vulkan => &["libvulkan.so.1"],
    }
}

#[cfg(target_os = "macos")]
fn loader_candidates(api: GraphicsApi) -> &'static [&'static str] {
    match api {
        GraphicsApi::OpenGl => &["/System/Library/Frameworks/OpenGL.framework/OpenGL"],
        GraphicsApi::Vulkan => &["libvulkan.1.dylib", "libvulkan.dylib"],
    }
}

#[cfg(target_os = "windows")]
fn load_library(name: &str, required_symbol: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> i32;
    }

    const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0000_0800;

    let symbol = CString::new(required_symbol)
        .map_err(|_| format!("required symbol '{required_symbol}' contains NUL"))?;
    let wide: Vec<u16> = std::ffi::OsStr::new(name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `wide` is NUL-terminated and remains alive for the call. A
    // non-null module handle is released exactly once before returning.
    let module = unsafe {
        LoadLibraryExW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if module.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: `module` is a live handle and `symbol` is NUL-terminated. The
    // returned address is used only as proof that the loader exports the
    // required API entry point.
    let address = unsafe { GetProcAddress(module, symbol.as_ptr().cast::<u8>()) };
    if address.is_null() {
        let error = std::io::Error::last_os_error();
        unsafe {
            FreeLibrary(module);
        }
        return Err(format!(
            "required symbol '{required_symbol}' is unavailable: {error}"
        ));
    }
    unsafe {
        FreeLibrary(module);
    }
    Ok(())
}

#[cfg(unix)]
fn load_library(name: &str, required_symbol: &str) -> Result<(), String> {
    use std::ffi::CStr;

    #[cfg(target_os = "linux")]
    #[link(name = "dl")]
    extern "C" {
        fn dlopen(filename: *const i8, flags: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const i8) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> i32;
        fn dlerror() -> *const i8;
    }
    #[cfg(target_os = "macos")]
    extern "C" {
        fn dlopen(filename: *const i8, flags: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const i8) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> i32;
        fn dlerror() -> *const i8;
    }

    const RTLD_NOW: i32 = 2;
    const RTLD_LOCAL: i32 = 0;
    let name = CString::new(name).map_err(|_| "library name contains NUL".to_string())?;
    let symbol = CString::new(required_symbol)
        .map_err(|_| format!("required symbol '{required_symbol}' contains NUL"))?;
    // SAFETY: `name` is a valid NUL-terminated C string. A non-null handle is
    // closed exactly once. `dlerror`'s pointer is copied before another loader
    // call can invalidate it.
    let handle = unsafe { dlopen(name.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
    if handle.is_null() {
        let detail = unsafe {
            let error = dlerror();
            if error.is_null() {
                "unknown dynamic-loader error".to_string()
            } else {
                CStr::from_ptr(error).to_string_lossy().into_owned()
            }
        };
        return Err(detail);
    }
    // SAFETY: clear the thread-local loader error, then query the live handle
    // with a NUL-terminated symbol. Copy any error before calling `dlclose`.
    unsafe {
        dlerror();
    }
    let address = unsafe { dlsym(handle, symbol.as_ptr()) };
    let symbol_error = unsafe { dlerror() };
    if address.is_null() || !symbol_error.is_null() {
        let detail = if symbol_error.is_null() {
            "symbol address was null".to_string()
        } else {
            unsafe { CStr::from_ptr(symbol_error) }
                .to_string_lossy()
                .into_owned()
        };
        unsafe {
            dlclose(handle);
        }
        return Err(format!(
            "required symbol '{required_symbol}' is unavailable: {detail}"
        ));
    }
    unsafe {
        dlclose(handle);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_requirements_are_ordered_and_duplicate_free() {
        assert_eq!(required_graphics_apis(&[]), []);
        assert_eq!(
            required_graphics_apis(&["usd-core".into(), "opengl".into()]),
            [GraphicsApi::OpenGl]
        );
        assert_eq!(required_loader_symbol(GraphicsApi::OpenGl), "glGetString");
        assert_eq!(
            required_loader_symbol(GraphicsApi::Vulkan),
            "vkGetInstanceProcAddr"
        );
        assert_eq!(
            required_graphics_apis(&["vulkan".into(), "opengl".into(), "vulkan".into(),]),
            [GraphicsApi::OpenGl, GraphicsApi::Vulkan]
        );
    }

    #[test]
    fn the_platform_opengl_loader_can_be_observed() {
        let probe = probe_graphics_loader(GraphicsApi::OpenGl);
        // Minimal/headless CI images may intentionally carry no OpenGL loader;
        // either outcome is an observation, not a reason for this unit test to
        // depend on the machine image.
        assert_eq!(probe.api, GraphicsApi::OpenGl);
        assert!(probe.detail.contains("opengl"));
    }

    #[test]
    fn an_open_library_without_the_required_entry_point_is_rejected() {
        let Some(candidate) = loader_candidates(GraphicsApi::OpenGl)
            .iter()
            .find(|candidate| load_library(candidate, "glGetString").is_ok())
        else {
            // A minimal/headless image may carry no OpenGL loader at all.
            return;
        };
        let error = load_library(candidate, "ostDefinitelyMissingLoaderEntryPoint").unwrap_err();
        assert!(error.contains("required symbol"), "{error}");
    }

    #[test]
    fn loader_status_never_turns_no_observation_into_success() {
        assert_eq!(graphics_loader_status(&[]), None);
        assert_eq!(
            graphics_loader_status(&[GraphicsLoaderProbe {
                api: GraphicsApi::OpenGl,
                passed: true,
                detail: "loaded".into(),
            }]),
            Some(OpenUsdVerificationStatus::Passed)
        );
        assert_eq!(
            graphics_loader_status(&[
                GraphicsLoaderProbe {
                    api: GraphicsApi::OpenGl,
                    passed: true,
                    detail: "loaded".into(),
                },
                GraphicsLoaderProbe {
                    api: GraphicsApi::Vulkan,
                    passed: false,
                    detail: "missing".into(),
                },
            ]),
            Some(OpenUsdVerificationStatus::Failed)
        );
    }
}
