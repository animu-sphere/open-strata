// SPDX-License-Identifier: Apache-2.0
//! Host graphics probes for normalized OpenUSD runtimes.
//!
//! Loader and physical-device checks are deliberately separate. Loading an API
//! entry point does not prove that a device exists, so the latter creates the
//! smallest native API object needed to enumerate real devices. Rendering is
//! still a separate OpenUSD-owned probe in the CLI.

use std::ffi::{c_void, CString};

use ost_platform::{OpenUsdVariantId, OpenUsdVerificationStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsVerifierKind {
    NoGraphics,
    OpenGl,
    Vulkan,
    Metal,
}

pub fn graphics_verifier_kind(
    variant: OpenUsdVariantId,
    capabilities: &[String],
) -> Option<GraphicsVerifierKind> {
    match variant.canonical(capabilities)? {
        OpenUsdVariantId::Core => Some(GraphicsVerifierKind::NoGraphics),
        OpenUsdVariantId::Gl => Some(GraphicsVerifierKind::OpenGl),
        OpenUsdVariantId::Vulkan => Some(GraphicsVerifierKind::Vulkan),
        OpenUsdVariantId::Metal => Some(GraphicsVerifierKind::Metal),
        OpenUsdVariantId::Headless | OpenUsdVariantId::Standard => unreachable!(),
    }
}

/// A graphics API whose loader is required by an OpenUSD compatibility cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsApi {
    OpenGl,
    Vulkan,
    Metal,
}

impl GraphicsApi {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenGl => "opengl",
            Self::Vulkan => "vulkan",
            Self::Metal => "metal",
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

/// The observed result of enumerating a physical device for one graphics API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphicsDeviceProbe {
    pub api: GraphicsApi,
    pub passed: bool,
    pub skipped: bool,
    pub detail: String,
}

/// Static musl binaries cannot dynamically load host graphics libraries.
/// Keep that packaging limitation explicit so runtime validation can skip the
/// host-loader probes instead of reporting a false runtime failure.
pub fn graphics_loader_probes_supported() -> bool {
    !cfg!(all(target_os = "linux", target_env = "musl"))
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

pub fn probe_graphics_loaders_for_variant(
    variant: OpenUsdVariantId,
    capabilities: &[String],
) -> Vec<GraphicsLoaderProbe> {
    verifier_graphics_apis(variant, capabilities)
        .into_iter()
        .map(probe_graphics_loader)
        .collect()
}

/// Enumerate physical devices for every graphics API required by a normalized
/// compatibility cell.
///
/// The currently approved OpenUSD cells are Linux x86_64. Their OpenGL path is
/// probed through GLX, while Vulkan uses the loader API directly and therefore
/// needs no SDK utility. Other hosts retain a truthful failed observation if a
/// future cell reaches this function before its native context adapter exists.
pub fn probe_graphics_devices(capabilities: &[String]) -> Vec<GraphicsDeviceProbe> {
    required_graphics_apis(capabilities)
        .into_iter()
        .map(probe_graphics_device)
        .collect()
}

pub fn probe_graphics_devices_for_variant(
    variant: OpenUsdVariantId,
    capabilities: &[String],
) -> Vec<GraphicsDeviceProbe> {
    verifier_graphics_apis(variant, capabilities)
        .into_iter()
        .map(probe_graphics_device)
        .collect()
}

fn verifier_graphics_apis(variant: OpenUsdVariantId, capabilities: &[String]) -> Vec<GraphicsApi> {
    match graphics_verifier_kind(variant, capabilities) {
        Some(GraphicsVerifierKind::NoGraphics) | None => Vec::new(),
        Some(GraphicsVerifierKind::OpenGl) => vec![GraphicsApi::OpenGl],
        Some(GraphicsVerifierKind::Vulkan) => vec![GraphicsApi::OpenGl, GraphicsApi::Vulkan],
        Some(GraphicsVerifierKind::Metal) => vec![GraphicsApi::Metal],
    }
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

/// Reduce actual device observations to the independent verification field.
pub fn graphics_device_status(probes: &[GraphicsDeviceProbe]) -> Option<OpenUsdVerificationStatus> {
    if probes.is_empty() || probes.iter().any(|probe| probe.skipped) {
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
    if capabilities.iter().any(|value| value == "metal") {
        required.push(GraphicsApi::Metal);
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

fn probe_graphics_device(api: GraphicsApi) -> GraphicsDeviceProbe {
    #[cfg(target_os = "linux")]
    if api == GraphicsApi::OpenGl
        && std::env::var_os("DISPLAY").is_none()
        && std::env::var_os("WAYLAND_DISPLAY").is_none()
    {
        return GraphicsDeviceProbe {
            api,
            passed: false,
            skipped: true,
            detail: "no DISPLAY or WAYLAND_DISPLAY is available for an OpenGL context".into(),
        };
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    if api == GraphicsApi::OpenGl {
        return GraphicsDeviceProbe {
            api,
            passed: false,
            skipped: true,
            detail:
                "physical OpenGL device probing is currently supported for approved Linux cells"
                    .into(),
        };
    }
    let result = match api {
        GraphicsApi::OpenGl => {
            return probe_opengl_device().unwrap_or_else(|detail| GraphicsDeviceProbe {
                api,
                passed: false,
                skipped: false,
                detail,
            });
        }
        GraphicsApi::Vulkan => probe_vulkan_device(),
        GraphicsApi::Metal => probe_metal_device(),
    };
    match result {
        Ok(detail) => GraphicsDeviceProbe {
            api,
            passed: true,
            skipped: false,
            detail,
        },
        Err(detail) => GraphicsDeviceProbe {
            api,
            passed: false,
            skipped: false,
            detail,
        },
    }
}

#[cfg(target_os = "linux")]
fn probe_opengl_device() -> Result<GraphicsDeviceProbe, String> {
    use std::ffi::CStr;

    type Display = c_void;
    type GlxFbConfig = *mut c_void;
    type GlxContext = *mut c_void;
    type GlxPbuffer = usize;

    type XOpenDisplay = unsafe extern "C" fn(*const i8) -> *mut Display;
    type XDefaultScreen = unsafe extern "C" fn(*mut Display) -> i32;
    type XCloseDisplay = unsafe extern "C" fn(*mut Display) -> i32;
    type XFree = unsafe extern "C" fn(*mut c_void) -> i32;
    type GlxChooseFbConfig =
        unsafe extern "C" fn(*mut Display, i32, *const i32, *mut i32) -> *mut GlxFbConfig;
    type GlxCreatePbuffer =
        unsafe extern "C" fn(*mut Display, GlxFbConfig, *const i32) -> GlxPbuffer;
    type GlxCreateNewContext =
        unsafe extern "C" fn(*mut Display, GlxFbConfig, i32, GlxContext, i32) -> GlxContext;
    type GlxMakeContextCurrent =
        unsafe extern "C" fn(*mut Display, GlxPbuffer, GlxPbuffer, GlxContext) -> i32;
    type GlxDestroyPbuffer = unsafe extern "C" fn(*mut Display, GlxPbuffer);
    type GlxDestroyContext = unsafe extern "C" fn(*mut Display, GlxContext);
    type GlGetString = unsafe extern "C" fn(u32) -> *const u8;

    const GLX_RENDER_TYPE: i32 = 0x8011;
    const GLX_RGBA_BIT: i32 = 0x0001;
    const GLX_DRAWABLE_TYPE: i32 = 0x8010;
    const GLX_PBUFFER_BIT: i32 = 0x0004;
    const GLX_X_RENDERABLE: i32 = 0x8012;
    const GLX_RGBA_TYPE: i32 = 0x8014;
    const GLX_PBUFFER_HEIGHT: i32 = 0x8040;
    const GLX_PBUFFER_WIDTH: i32 = 0x8041;
    const GL_RENDERER: u32 = 0x1f01;

    let x11 = NativeLibrary::open(&["libX11.so.6"])?;
    let gl = NativeLibrary::open(&["libGL.so.1", "libOpenGL.so.0"])?;
    // SAFETY: every symbol is resolved from the live library that defines the
    // matching C ABI. The handles outlive all calls below.
    let x_open_display: XOpenDisplay = unsafe { x11.function("XOpenDisplay")? };
    let x_default_screen: XDefaultScreen = unsafe { x11.function("XDefaultScreen")? };
    let x_close_display: XCloseDisplay = unsafe { x11.function("XCloseDisplay")? };
    let x_free: XFree = unsafe { x11.function("XFree")? };
    let choose: GlxChooseFbConfig = unsafe { gl.function("glXChooseFBConfig")? };
    let create_pbuffer: GlxCreatePbuffer = unsafe { gl.function("glXCreatePbuffer")? };
    let create_context: GlxCreateNewContext = unsafe { gl.function("glXCreateNewContext")? };
    let make_current: GlxMakeContextCurrent = unsafe { gl.function("glXMakeContextCurrent")? };
    let destroy_pbuffer: GlxDestroyPbuffer = unsafe { gl.function("glXDestroyPbuffer")? };
    let destroy_context: GlxDestroyContext = unsafe { gl.function("glXDestroyContext")? };
    let get_string: GlGetString = unsafe { gl.function("glGetString")? };

    // SAFETY: the null name requests DISPLAY. Every successful resource is
    // released on all later paths before its defining library is dropped.
    let display = unsafe { x_open_display(std::ptr::null()) };
    if display.is_null() {
        return Err("could not open an X11 display for the OpenGL device probe".into());
    }
    let screen = unsafe { x_default_screen(display) };
    let attributes = [
        GLX_X_RENDERABLE,
        1,
        GLX_DRAWABLE_TYPE,
        GLX_PBUFFER_BIT,
        GLX_RENDER_TYPE,
        GLX_RGBA_BIT,
        0,
    ];
    let mut count = 0;
    let configs = unsafe { choose(display, screen, attributes.as_ptr(), &mut count) };
    if configs.is_null() || count == 0 {
        unsafe {
            x_close_display(display);
        }
        return Err("GLX reported no framebuffer configuration for a pbuffer".into());
    }
    let config = unsafe { *configs };
    let pbuffer_attributes = [GLX_PBUFFER_WIDTH, 1, GLX_PBUFFER_HEIGHT, 1, 0];
    let pbuffer = unsafe { create_pbuffer(display, config, pbuffer_attributes.as_ptr()) };
    let context =
        unsafe { create_context(display, config, GLX_RGBA_TYPE, std::ptr::null_mut(), 1) };
    unsafe {
        x_free(configs.cast::<c_void>());
    }
    if pbuffer == 0 || context.is_null() {
        unsafe {
            if pbuffer != 0 {
                destroy_pbuffer(display, pbuffer);
            }
            if !context.is_null() {
                destroy_context(display, context);
            }
            x_close_display(display);
        }
        return Err("GLX could not create a physical-device context".into());
    }
    let current = unsafe { make_current(display, pbuffer, pbuffer, context) } != 0;
    let renderer = if current {
        let value = unsafe { get_string(GL_RENDERER) };
        if value.is_null() {
            None
        } else {
            Some(
                unsafe { CStr::from_ptr(value.cast::<i8>()) }
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    } else {
        None
    };
    unsafe {
        make_current(display, 0, 0, std::ptr::null_mut());
        destroy_context(display, context);
        destroy_pbuffer(display, pbuffer);
        x_close_display(display);
    }
    match renderer {
        Some(renderer) => Ok(opengl_device_observation("GLX", &renderer)),
        None => Err("GLX created a context but no physical OpenGL renderer was observable".into()),
    }
}

#[cfg(target_os = "windows")]
fn probe_opengl_device() -> Result<GraphicsDeviceProbe, String> {
    use std::ffi::CStr;

    type Hwnd = *mut c_void;
    type Hdc = *mut c_void;
    type Hglrc = *mut c_void;

    #[repr(C)]
    struct PixelFormatDescriptor {
        size: u16,
        version: u16,
        flags: u32,
        pixel_type: u8,
        color_bits: u8,
        red_bits: u8,
        red_shift: u8,
        green_bits: u8,
        green_shift: u8,
        blue_bits: u8,
        blue_shift: u8,
        alpha_bits: u8,
        alpha_shift: u8,
        accum_bits: u8,
        accum_red_bits: u8,
        accum_green_bits: u8,
        accum_blue_bits: u8,
        accum_alpha_bits: u8,
        depth_bits: u8,
        stencil_bits: u8,
        aux_buffers: u8,
        layer_type: u8,
        reserved: u8,
        layer_mask: u32,
        visible_mask: u32,
        damage_mask: u32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn CreateWindowExW(
            ex_style: u32,
            class_name: *const u16,
            window_name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            parent: Hwnd,
            menu: *mut c_void,
            instance: *mut c_void,
            parameter: *mut c_void,
        ) -> Hwnd;
        fn DestroyWindow(window: Hwnd) -> i32;
        fn GetDC(window: Hwnd) -> Hdc;
        fn ReleaseDC(window: Hwnd, dc: Hdc) -> i32;
    }
    #[link(name = "gdi32")]
    extern "system" {
        fn ChoosePixelFormat(dc: Hdc, descriptor: *const PixelFormatDescriptor) -> i32;
        fn SetPixelFormat(dc: Hdc, format: i32, descriptor: *const PixelFormatDescriptor) -> i32;
    }
    #[link(name = "opengl32")]
    extern "system" {
        fn wglCreateContext(dc: Hdc) -> Hglrc;
        fn wglDeleteContext(context: Hglrc) -> i32;
        fn wglMakeCurrent(dc: Hdc, context: Hglrc) -> i32;
        fn glGetString(name: u32) -> *const u8;
    }

    const WS_POPUP: u32 = 0x8000_0000;
    const PFD_DOUBLEBUFFER: u32 = 0x0000_0001;
    const PFD_DRAW_TO_WINDOW: u32 = 0x0000_0004;
    const PFD_SUPPORT_OPENGL: u32 = 0x0000_0020;
    const PFD_TYPE_RGBA: u8 = 0;
    const PFD_MAIN_PLANE: u8 = 0;
    const GL_RENDERER: u32 = 0x1f01;

    let class_name: Vec<u16> = "STATIC".encode_utf16().chain(std::iter::once(0)).collect();
    let window_name: Vec<u16> = "OpenStrata OpenGL probe"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let descriptor = PixelFormatDescriptor {
        size: std::mem::size_of::<PixelFormatDescriptor>() as u16,
        version: 1,
        flags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
        pixel_type: PFD_TYPE_RGBA,
        color_bits: 24,
        red_bits: 0,
        red_shift: 0,
        green_bits: 0,
        green_shift: 0,
        blue_bits: 0,
        blue_shift: 0,
        alpha_bits: 8,
        alpha_shift: 0,
        accum_bits: 0,
        accum_red_bits: 0,
        accum_green_bits: 0,
        accum_blue_bits: 0,
        accum_alpha_bits: 0,
        depth_bits: 24,
        stencil_bits: 8,
        aux_buffers: 0,
        layer_type: PFD_MAIN_PLANE,
        reserved: 0,
        layer_mask: 0,
        visible_mask: 0,
        damage_mask: 0,
    };

    // SAFETY: all handles are created by the matching Win32 APIs and released
    // before returning. The built-in STATIC class avoids registering global
    // process state solely for this hidden one-pixel context.
    unsafe {
        let window = CreateWindowExW(
            0,
            class_name.as_ptr(),
            window_name.as_ptr(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        if window.is_null() {
            return Err(format!(
                "Win32 could not create the hidden OpenGL probe window: {}",
                std::io::Error::last_os_error()
            ));
        }
        let dc = GetDC(window);
        if dc.is_null() {
            DestroyWindow(window);
            return Err(format!(
                "Win32 could not acquire a device context for OpenGL: {}",
                std::io::Error::last_os_error()
            ));
        }
        let format = ChoosePixelFormat(dc, &descriptor);
        if format == 0 || SetPixelFormat(dc, format, &descriptor) == 0 {
            let error = std::io::Error::last_os_error();
            ReleaseDC(window, dc);
            DestroyWindow(window);
            return Err(format!(
                "Win32 could not set an OpenGL pixel format: {error}"
            ));
        }
        let context = wglCreateContext(dc);
        if context.is_null() || wglMakeCurrent(dc, context) == 0 {
            let error = std::io::Error::last_os_error();
            if !context.is_null() {
                wglDeleteContext(context);
            }
            ReleaseDC(window, dc);
            DestroyWindow(window);
            return Err(format!(
                "WGL could not create a current OpenGL context: {error}"
            ));
        }
        let value = glGetString(GL_RENDERER);
        let renderer = if value.is_null() {
            None
        } else {
            Some(
                CStr::from_ptr(value.cast::<i8>())
                    .to_string_lossy()
                    .into_owned(),
            )
        };
        wglMakeCurrent(std::ptr::null_mut(), std::ptr::null_mut());
        wglDeleteContext(context);
        ReleaseDC(window, dc);
        DestroyWindow(window);
        renderer
            .map(|renderer| opengl_device_observation("WGL", &renderer))
            .ok_or_else(|| {
                "WGL created a context but no physical OpenGL renderer was observable".into()
            })
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn probe_opengl_device() -> Result<GraphicsDeviceProbe, String> {
    Err("physical OpenGL device probing is currently supported for approved Linux cells".into())
}

#[cfg(any(target_os = "linux", target_os = "windows", test))]
fn opengl_device_observation(context: &str, renderer: &str) -> GraphicsDeviceProbe {
    // A current context is not necessarily a physical-device observation.
    // GDI Generic cannot run Storm; Mesa software renderers can support modern
    // OpenGL but still do not prove the physical-device field we are measuring.
    let name = renderer.to_ascii_lowercase();
    let skipped = name == "gdi generic"
        || ["llvmpipe", "softpipe"]
            .iter()
            .any(|software| name == *software || name.starts_with(&format!("{software} ")));
    GraphicsDeviceProbe {
        api: GraphicsApi::OpenGl,
        passed: !skipped,
        skipped,
        detail: if skipped {
            format!(
                "{context} exposes software renderer '{renderer}'; \
                 no physical OpenGL device was observed on this host"
            )
        } else {
            format!("created a {context} context on '{renderer}'")
        },
    }
}

fn probe_vulkan_device() -> Result<String, String> {
    #[repr(C)]
    struct VkApplicationInfo {
        structure_type: u32,
        next: *const c_void,
        application_name: *const i8,
        application_version: u32,
        engine_name: *const i8,
        engine_version: u32,
        api_version: u32,
    }
    #[repr(C)]
    struct VkInstanceCreateInfo {
        structure_type: u32,
        next: *const c_void,
        flags: u32,
        application: *const VkApplicationInfo,
        layer_count: u32,
        layers: *const *const i8,
        extension_count: u32,
        extensions: *const *const i8,
    }
    type VkInstance = *mut c_void;
    type VkPhysicalDevice = *mut c_void;
    type VkCreateInstance = unsafe extern "system" fn(
        *const VkInstanceCreateInfo,
        *const c_void,
        *mut VkInstance,
    ) -> i32;
    type VkDestroyInstance = unsafe extern "system" fn(VkInstance, *const c_void);
    type VkEnumeratePhysicalDevices =
        unsafe extern "system" fn(VkInstance, *mut u32, *mut VkPhysicalDevice) -> i32;

    const VK_SUCCESS: i32 = 0;
    const VK_STRUCTURE_TYPE_APPLICATION_INFO: u32 = 0;
    const VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO: u32 = 1;
    const VK_API_VERSION_1_0: u32 = 1 << 22;

    let library = NativeLibrary::open(loader_candidates(GraphicsApi::Vulkan))?;
    // SAFETY: signatures are the Vulkan 1.0 loader ABI, resolved from the live
    // loader library retained through instance destruction.
    let create: VkCreateInstance = unsafe { library.function("vkCreateInstance")? };
    let enumerate: VkEnumeratePhysicalDevices =
        unsafe { library.function("vkEnumeratePhysicalDevices")? };
    let destroy: VkDestroyInstance = unsafe { library.function("vkDestroyInstance")? };
    let name = CString::new("OpenStrata device probe").expect("literal has no NUL");
    let application = VkApplicationInfo {
        structure_type: VK_STRUCTURE_TYPE_APPLICATION_INFO,
        next: std::ptr::null(),
        application_name: name.as_ptr(),
        application_version: 1,
        engine_name: name.as_ptr(),
        engine_version: 1,
        api_version: VK_API_VERSION_1_0,
    };
    let create_info = VkInstanceCreateInfo {
        structure_type: VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
        next: std::ptr::null(),
        flags: 0,
        application: &application,
        layer_count: 0,
        layers: std::ptr::null(),
        extension_count: 0,
        extensions: std::ptr::null(),
    };
    let mut instance = std::ptr::null_mut();
    let result = unsafe { create(&create_info, std::ptr::null(), &mut instance) };
    if result != VK_SUCCESS || instance.is_null() {
        return Err(format!(
            "Vulkan could not create an instance for device enumeration (VkResult {result})"
        ));
    }
    let mut count = 0;
    let result = unsafe { enumerate(instance, &mut count, std::ptr::null_mut()) };
    unsafe {
        destroy(instance, std::ptr::null());
    }
    if result != VK_SUCCESS {
        return Err(format!(
            "Vulkan physical-device enumeration failed (VkResult {result})"
        ));
    }
    if count == 0 {
        return Err("Vulkan reported zero physical devices".into());
    }
    Ok(format!("Vulkan enumerated {count} physical device(s)"))
}

#[cfg(target_os = "macos")]
fn probe_metal_device() -> Result<String, String> {
    type MtlCreateSystemDefaultDevice = unsafe extern "C" fn() -> *mut c_void;
    let library = NativeLibrary::open(loader_candidates(GraphicsApi::Metal))?;
    let create: MtlCreateSystemDefaultDevice =
        unsafe { library.function("MTLCreateSystemDefaultDevice")? };
    let device = unsafe { create() };
    if device.is_null() {
        Err("Metal returned no system default MTLDevice".into())
    } else {
        Ok("Metal created the system default MTLDevice".into())
    }
}

#[cfg(not(target_os = "macos"))]
fn probe_metal_device() -> Result<String, String> {
    Err("Metal device probing is available only on macOS".into())
}

fn required_loader_symbol(api: GraphicsApi) -> &'static str {
    match api {
        GraphicsApi::OpenGl => "glGetString",
        GraphicsApi::Vulkan => "vkGetInstanceProcAddr",
        GraphicsApi::Metal => "MTLCreateSystemDefaultDevice",
    }
}

#[cfg(target_os = "windows")]
fn loader_candidates(api: GraphicsApi) -> &'static [&'static str] {
    match api {
        GraphicsApi::OpenGl => &["opengl32.dll"],
        GraphicsApi::Vulkan => &["vulkan-1.dll"],
        GraphicsApi::Metal => &[],
    }
}

#[cfg(target_os = "linux")]
fn loader_candidates(api: GraphicsApi) -> &'static [&'static str] {
    match api {
        GraphicsApi::OpenGl => &["libOpenGL.so.0", "libGL.so.1"],
        GraphicsApi::Vulkan => &["libvulkan.so.1"],
        GraphicsApi::Metal => &[],
    }
}

#[cfg(target_os = "macos")]
fn loader_candidates(api: GraphicsApi) -> &'static [&'static str] {
    match api {
        GraphicsApi::OpenGl => &["/System/Library/Frameworks/OpenGL.framework/OpenGL"],
        GraphicsApi::Vulkan => &["libvulkan.1.dylib", "libvulkan.dylib"],
        GraphicsApi::Metal => &["/System/Library/Frameworks/Metal.framework/Metal"],
    }
}

/// A native dynamic-library handle kept alive while typed API functions run.
struct NativeLibrary {
    handle: *mut c_void,
}

impl NativeLibrary {
    fn open(candidates: &[&str]) -> Result<Self, String> {
        let mut failures = Vec::new();
        for candidate in candidates {
            match open_native_library(candidate) {
                Ok(handle) => return Ok(Self { handle }),
                Err(error) => failures.push(format!("{candidate}: {error}")),
            }
        }
        Err(format!(
            "could not load a native graphics library ({})",
            failures.join("; ")
        ))
    }

    /// Resolve a function with the caller-declared native ABI.
    ///
    /// # Safety
    ///
    /// `T` must be the exact function-pointer signature exported under `name`.
    unsafe fn function<T: Copy>(&self, name: &str) -> Result<T, String> {
        let address = native_symbol(self.handle, name)?;
        if std::mem::size_of::<T>() != std::mem::size_of::<*mut c_void>() {
            return Err(format!("function pointer '{name}' has an unexpected size"));
        }
        // SAFETY: the size check above and this method's contract establish
        // that `T` is the function-pointer representation of `address`.
        Ok(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&address) })
    }
}

impl Drop for NativeLibrary {
    fn drop(&mut self) {
        // SAFETY: `handle` was returned by `open_native_library` and is closed
        // once, after all functions borrowed from the library have returned.
        unsafe {
            close_native_library(self.handle);
        }
    }
}

#[cfg(target_os = "windows")]
fn open_native_library(name: &str) -> Result<*mut c_void, String> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    }
    const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0000_0800;
    let wide: Vec<u16> = std::ffi::OsStr::new(name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        LoadLibraryExW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if handle.is_null() {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(handle)
    }
}

#[cfg(target_os = "windows")]
fn native_symbol(handle: *mut c_void, name: &str) -> Result<*mut c_void, String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    }
    let name = CString::new(name).map_err(|_| "symbol name contains NUL".to_string())?;
    let address = unsafe { GetProcAddress(handle, name.as_ptr().cast::<u8>()) };
    if address.is_null() {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(address)
    }
}

#[cfg(target_os = "windows")]
unsafe fn close_native_library(handle: *mut c_void) {
    #[link(name = "kernel32")]
    extern "system" {
        fn FreeLibrary(module: *mut c_void) -> i32;
    }
    unsafe {
        FreeLibrary(handle);
    }
}

#[cfg(unix)]
fn open_native_library(name: &str) -> Result<*mut c_void, String> {
    use std::ffi::CStr;

    #[cfg(target_os = "linux")]
    #[link(name = "dl")]
    extern "C" {
        fn dlopen(filename: *const i8, flags: i32) -> *mut c_void;
        fn dlerror() -> *const i8;
    }
    #[cfg(target_os = "macos")]
    extern "C" {
        fn dlopen(filename: *const i8, flags: i32) -> *mut c_void;
        fn dlerror() -> *const i8;
    }
    const RTLD_NOW: i32 = 2;
    const RTLD_LOCAL: i32 = 0;
    let name = CString::new(name).map_err(|_| "library name contains NUL".to_string())?;
    let handle = unsafe { dlopen(name.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
    if handle.is_null() {
        let error = unsafe { dlerror() };
        Err(if error.is_null() {
            "unknown dynamic-loader error".into()
        } else {
            unsafe { CStr::from_ptr(error) }
                .to_string_lossy()
                .into_owned()
        })
    } else {
        Ok(handle)
    }
}

#[cfg(unix)]
fn native_symbol(handle: *mut c_void, name: &str) -> Result<*mut c_void, String> {
    use std::ffi::CStr;

    #[cfg(target_os = "linux")]
    #[link(name = "dl")]
    extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const i8) -> *mut c_void;
        fn dlerror() -> *const i8;
    }
    #[cfg(target_os = "macos")]
    extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const i8) -> *mut c_void;
        fn dlerror() -> *const i8;
    }
    let name = CString::new(name).map_err(|_| "symbol name contains NUL".to_string())?;
    unsafe {
        dlerror();
    }
    let address = unsafe { dlsym(handle, name.as_ptr()) };
    let error = unsafe { dlerror() };
    if address.is_null() || !error.is_null() {
        Err(if error.is_null() {
            "symbol address was null".into()
        } else {
            unsafe { CStr::from_ptr(error) }
                .to_string_lossy()
                .into_owned()
        })
    } else {
        Ok(address)
    }
}

#[cfg(unix)]
unsafe fn close_native_library(handle: *mut c_void) {
    #[cfg(target_os = "linux")]
    #[link(name = "dl")]
    extern "C" {
        fn dlclose(handle: *mut c_void) -> i32;
    }
    #[cfg(target_os = "macos")]
    extern "C" {
        fn dlclose(handle: *mut c_void) -> i32;
    }
    unsafe {
        dlclose(handle);
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

    #[test]
    fn device_status_never_turns_no_observation_into_success() {
        assert_eq!(graphics_device_status(&[]), None);
        assert_eq!(
            graphics_device_status(&[GraphicsDeviceProbe {
                api: GraphicsApi::Vulkan,
                passed: true,
                skipped: false,
                detail: "one device".into(),
            }]),
            Some(OpenUsdVerificationStatus::Passed)
        );
        assert_eq!(
            graphics_device_status(&[
                GraphicsDeviceProbe {
                    api: GraphicsApi::OpenGl,
                    passed: true,
                    skipped: false,
                    detail: "renderer".into(),
                },
                GraphicsDeviceProbe {
                    api: GraphicsApi::Vulkan,
                    passed: false,
                    skipped: false,
                    detail: "zero devices".into(),
                },
            ]),
            Some(OpenUsdVerificationStatus::Failed)
        );
        assert_eq!(
            graphics_device_status(&[GraphicsDeviceProbe {
                api: GraphicsApi::OpenGl,
                passed: false,
                skipped: true,
                detail: "headless".into(),
            }]),
            None
        );
    }

    #[test]
    fn software_renderers_are_not_physical_device_observations() {
        for (context, renderer) in [
            ("WGL", "GDI Generic"),
            ("GLX", "llvmpipe (LLVM 20.1.0, 256 bits)"),
            ("GLX", "softpipe"),
        ] {
            let probe = opengl_device_observation(context, renderer);
            assert!(probe.skipped);
            assert!(!probe.passed);
            assert!(probe.detail.contains(renderer));
            assert_eq!(graphics_device_status(&[probe]), None);
        }

        for renderer in [
            "NVIDIA RTX A5000/PCIe/SSE2",
            "AMD Radeon RX 7900 XTX (radeonsi, navi31, LLVM 20.1.0)",
        ] {
            let probe = opengl_device_observation("WGL", renderer);
            assert!(!probe.skipped);
            assert!(probe.passed);
            assert_eq!(
                graphics_device_status(&[probe]),
                Some(OpenUsdVerificationStatus::Passed)
            );
        }
    }

    #[test]
    fn the_platform_vulkan_device_can_be_observed_without_an_sdk_utility() {
        let probe = probe_graphics_device(GraphicsApi::Vulkan);
        assert_eq!(probe.api, GraphicsApi::Vulkan);
        assert!(probe.detail.contains("Vulkan") || probe.detail.contains("vulkan"));
    }
}
