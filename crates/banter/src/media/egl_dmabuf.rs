//! Headless EGL DMA-BUF import proof (Linux Wayland capture M3).
//!
//! Opens `/dev/dri/renderD128` via GBM, creates an EGL display/context, and
//! tries `eglCreateImageKHR` with `EGL_LINUX_DMA_BUF_EXT`. Success destroys the
//! image immediately — this milestone proves import + metrics; GPU convert and
//! VA-API hardware frames (M4) stay deferred. MemFd/MemPtr remain the encode path.
//!
//! On init/import failure the session sticks to "disabled" so we do not retry
//! every PipeWire frame. No GPU in CI is fine — MemFd fallback still works.

#![cfg(target_os = "linux")]

use std::ffi::{c_void, CStr, CString};
use std::os::fd::RawFd;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use pipewire::spa::param::video::VideoFormat;

use super::capture_metrics;
use super::dmabuf::{DmaBufInfo, DRM_FORMAT_MOD_INVALID};

// --- EGL / GBM constants (from eglext.h / egl.h / gbm.h) ---

const EGL_NONE: i32 = 0x3038;
const EGL_OPENGL_ES_API: u32 = 0x30A0;
const EGL_CONTEXT_CLIENT_VERSION: i32 = 0x3098;
const EGL_NO_CONTEXT: *mut c_void = ptr::null_mut();
const EGL_NO_DISPLAY: *mut c_void = ptr::null_mut();
const EGL_NO_IMAGE_KHR: *mut c_void = ptr::null_mut();
const EGL_DEFAULT_DISPLAY: *mut c_void = ptr::null_mut();

const EGL_LINUX_DMA_BUF_EXT: u32 = 0x3270;
const EGL_LINUX_DRM_FOURCC_EXT: i32 = 0x3271;
const EGL_DMA_BUF_PLANE0_FD_EXT: i32 = 0x3272;
const EGL_DMA_BUF_PLANE0_OFFSET_EXT: i32 = 0x3273;
const EGL_DMA_BUF_PLANE0_PITCH_EXT: i32 = 0x3274;
const EGL_DMA_BUF_PLANE1_FD_EXT: i32 = 0x3275;
const EGL_DMA_BUF_PLANE1_OFFSET_EXT: i32 = 0x3276;
const EGL_DMA_BUF_PLANE1_PITCH_EXT: i32 = 0x3277;
const EGL_DMA_BUF_PLANE2_FD_EXT: i32 = 0x3278;
const EGL_DMA_BUF_PLANE2_OFFSET_EXT: i32 = 0x3279;
const EGL_DMA_BUF_PLANE2_PITCH_EXT: i32 = 0x327A;
const EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT: i32 = 0x3443;
const EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT: i32 = 0x3444;
const EGL_DMA_BUF_PLANE1_MODIFIER_LO_EXT: i32 = 0x3445;
const EGL_DMA_BUF_PLANE1_MODIFIER_HI_EXT: i32 = 0x3446;
const EGL_DMA_BUF_PLANE2_MODIFIER_LO_EXT: i32 = 0x3447;
const EGL_DMA_BUF_PLANE2_MODIFIER_HI_EXT: i32 = 0x3448;
const EGL_WIDTH: i32 = 0x3057;
const EGL_HEIGHT: i32 = 0x3056;

const EGL_PLATFORM_GBM_KHR: u32 = 0x31D7;
const EGL_PLATFORM_SURFACELESS_MESA: u32 = 0x31DD;

type EGLDisplay = *mut c_void;
type EGLContext = *mut c_void;
type EGLImageKHR = *mut c_void;
type EGLClientBuffer = *mut c_void;
type EGLint = i32;
type EGLBoolean = u32;
type EGLenum = u32;

#[link(name = "EGL")]
#[link(name = "gbm")]
extern "C" {
    fn gbm_create_device(fd: RawFd) -> *mut c_void;
    fn gbm_device_destroy(gbm: *mut c_void);

    fn eglGetError() -> EGLint;
    fn eglGetDisplay(display_id: *mut c_void) -> EGLDisplay;
    fn eglGetPlatformDisplay(platform: EGLenum, native_display: *mut c_void, attrib_list: *const isize) -> EGLDisplay;
    fn eglInitialize(dpy: EGLDisplay, major: *mut EGLint, minor: *mut EGLint) -> EGLBoolean;
    fn eglTerminate(dpy: EGLDisplay) -> EGLBoolean;
    fn eglBindAPI(api: EGLenum) -> EGLBoolean;
    fn eglCreateContext(
        dpy: EGLDisplay,
        config: *mut c_void,
        share: EGLContext,
        attrib_list: *const EGLint,
    ) -> EGLContext;
    fn eglDestroyContext(dpy: EGLDisplay, ctx: EGLContext) -> EGLBoolean;
    fn eglMakeCurrent(
        dpy: EGLDisplay,
        draw: *mut c_void,
        read: *mut c_void,
        ctx: EGLContext,
    ) -> EGLBoolean;
    fn eglQueryString(dpy: EGLDisplay, name: EGLint) -> *const i8;
    fn eglGetProcAddress(procname: *const i8) -> *mut c_void;
}

type CreateImageKhrFn = unsafe extern "C" fn(
    EGLDisplay,
    EGLContext,
    EGLenum,
    EGLClientBuffer,
    *const EGLint,
) -> EGLImageKHR;
type DestroyImageKhrFn = unsafe extern "C" fn(EGLDisplay, EGLImageKHR) -> EGLBoolean;

const EGL_EXTENSIONS: EGLint = 0x3055;

fn fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    u32::from(a) | (u32::from(b) << 8) | (u32::from(c) << 16) | (u32::from(d) << 24)
}

/// Map PipeWire video format → DRM fourcc for EGL_LINUX_DRM_FOURCC_EXT.
pub fn drm_fourcc_for(format: VideoFormat) -> Option<u32> {
    // SPA BGRA is little-endian B,G,R,A in memory → DRM_FORMAT_ARGB8888 channel order
    // is A,R,G,B in MSB-first docs but LE byte order matches BGRA memory layout.
    if format == VideoFormat::BGRA {
        Some(fourcc(b'A', b'R', b'2', b'4')) // DRM_FORMAT_ARGB8888
    } else if format == VideoFormat::BGRx {
        Some(fourcc(b'X', b'R', b'2', b'4')) // DRM_FORMAT_XRGB8888
    } else if format == VideoFormat::RGBA {
        Some(fourcc(b'A', b'B', b'2', b'4')) // DRM_FORMAT_ABGR8888
    } else if format == VideoFormat::RGBx {
        Some(fourcc(b'X', b'B', b'2', b'4')) // DRM_FORMAT_XBGR8888
    } else if format == VideoFormat::NV12 {
        Some(fourcc(b'N', b'V', b'1', b'2'))
    } else if format == VideoFormat::I420 {
        Some(fourcc(b'Y', b'U', b'1', b'2')) // DRM_FORMAT_YUV420
    } else {
        None
    }
}

struct EglSession {
    display: EGLDisplay,
    context: EGLContext,
    gbm: *mut c_void,
    drm_fd: RawFd,
    create_image: CreateImageKhrFn,
    destroy_image: DestroyImageKhrFn,
    has_modifiers: bool,
}

// Safety: EGL calls are serialized through SESSION mutex.
unsafe impl Send for EglSession {}

enum SessionState {
    Ready(EglSession),
    #[allow(dead_code)] // Message retained for diagnostics / future UI.
    Failed(String),
}

static SESSION: OnceLock<Mutex<SessionState>> = OnceLock::new();
static IMPORT_OK: AtomicU64 = AtomicU64::new(0);
static IMPORT_FAIL: AtomicU64 = AtomicU64::new(0);
static LOGGED_OK: AtomicBool = AtomicBool::new(false);
static EARLY_FAIL: AtomicBool = AtomicBool::new(false);

fn session_lock() -> &'static Mutex<SessionState> {
    SESSION.get_or_init(|| {
        Mutex::new(match open_session() {
            Ok(s) => SessionState::Ready(s),
            Err(e) => {
                eprintln!("banter: EGL DMA-BUF init failed ({e}); MemFd encode fallback stays");
                capture_metrics::note_egl_import_fail();
                IMPORT_FAIL.fetch_add(1, Ordering::Relaxed);
                SessionState::Failed(e)
            }
        })
    })
}

/// Result of one DMA-BUF → EGLImage proof attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportAttempt {
    /// `eglCreateImageKHR` succeeded (image destroyed immediately).
    Success,
    /// Session sticky-disabled after init/import failure — skip further attempts.
    Disabled,
    /// Fresh failure; session is now sticky-disabled.
    Failed(String),
}

fn open_render_fd() -> Result<RawFd, String> {
    for path in [
        "/dev/dri/renderD128",
        "/dev/dri/renderD129",
        "/dev/dri/card0",
    ] {
        let c_path = CString::new(path).expect("path");
        let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if fd >= 0 {
            return Ok(fd);
        }
    }
    Err("no usable /dev/dri/renderD* (need GPU render node)".into())
}

fn open_session() -> Result<EglSession, String> {
    unsafe {
        let drm_fd = open_render_fd()?;
        let gbm = gbm_create_device(drm_fd);
        let mut display = EGL_NO_DISPLAY;
        if !gbm.is_null() {
            display = eglGetPlatformDisplay(EGL_PLATFORM_GBM_KHR, gbm, ptr::null());
        }
        if display == EGL_NO_DISPLAY || display.is_null() {
            // Surfaceless fallback (Mesa) when GBM platform display fails.
            display = eglGetPlatformDisplay(
                EGL_PLATFORM_SURFACELESS_MESA,
                EGL_DEFAULT_DISPLAY,
                ptr::null(),
            );
        }
        if display == EGL_NO_DISPLAY || display.is_null() {
            display = eglGetDisplay(EGL_DEFAULT_DISPLAY);
        }
        if display == EGL_NO_DISPLAY || display.is_null() {
            if !gbm.is_null() {
                gbm_device_destroy(gbm);
            }
            libc::close(drm_fd);
            return Err(format!("eglGetDisplay failed (err={})", eglGetError()));
        }
        let mut major = 0;
        let mut minor = 0;
        if eglInitialize(display, &mut major, &mut minor) == 0 {
            let err = eglGetError();
            eglTerminate(display);
            if !gbm.is_null() {
                gbm_device_destroy(gbm);
            }
            libc::close(drm_fd);
            return Err(format!("eglInitialize failed (err={err})"));
        }
        let exts = {
            let p = eglQueryString(display, EGL_EXTENSIONS);
            if p.is_null() {
                String::new()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        if !exts.split_whitespace().any(|e| e == "EGL_EXT_image_dma_buf_import") {
            eglTerminate(display);
            if !gbm.is_null() {
                gbm_device_destroy(gbm);
            }
            libc::close(drm_fd);
            return Err("EGL_EXT_image_dma_buf_import not present".into());
        }
        let has_modifiers = exts
            .split_whitespace()
            .any(|e| e == "EGL_EXT_image_dma_buf_import_modifiers");
        let _ = eglBindAPI(EGL_OPENGL_ES_API);
        let attribs = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
        let context = eglCreateContext(display, ptr::null_mut(), EGL_NO_CONTEXT, attribs.as_ptr());
        if context.is_null() {
            let err = eglGetError();
            eglTerminate(display);
            if !gbm.is_null() {
                gbm_device_destroy(gbm);
            }
            libc::close(drm_fd);
            return Err(format!("eglCreateContext failed (err={err})"));
        }
        // Make current so some drivers accept CreateImage; surfaceless OK.
        let _ = eglMakeCurrent(display, ptr::null_mut(), ptr::null_mut(), context);

        let create_image = {
            let name = b"eglCreateImageKHR\0";
            let p = eglGetProcAddress(name.as_ptr().cast());
            if p.is_null() {
                eglDestroyContext(display, context);
                eglTerminate(display);
                if !gbm.is_null() {
                    gbm_device_destroy(gbm);
                }
                libc::close(drm_fd);
                return Err("eglCreateImageKHR missing".into());
            }
            std::mem::transmute::<_, CreateImageKhrFn>(p)
        };
        let destroy_image = {
            let name = b"eglDestroyImageKHR\0";
            let p = eglGetProcAddress(name.as_ptr().cast());
            if p.is_null() {
                eglDestroyContext(display, context);
                eglTerminate(display);
                if !gbm.is_null() {
                    gbm_device_destroy(gbm);
                }
                libc::close(drm_fd);
                return Err("eglDestroyImageKHR missing".into());
            }
            std::mem::transmute::<_, DestroyImageKhrFn>(p)
        };

        eprintln!(
            "banter: EGL DMA-BUF ready (EGL {major}.{minor}, modifiers={has_modifiers})"
        );
        Ok(EglSession {
            display,
            context,
            gbm,
            drm_fd,
            create_image,
            destroy_image,
            has_modifiers,
        })
    }
}

impl Drop for EglSession {
    fn drop(&mut self) {
        unsafe {
            let _ = eglMakeCurrent(
                self.display,
                ptr::null_mut(),
                ptr::null_mut(),
                EGL_NO_CONTEXT,
            );
            let _ = eglDestroyContext(self.display, self.context);
            let _ = eglTerminate(self.display);
            if !self.gbm.is_null() {
                gbm_device_destroy(self.gbm);
            }
            if self.drm_fd >= 0 {
                libc::close(self.drm_fd);
            }
        }
    }
}

fn push_plane_attrs(
    attrs: &mut Vec<EGLint>,
    index: usize,
    fd: i32,
    offset: u32,
    pitch: i32,
    modifier: u64,
    with_mod: bool,
) {
    let (fd_k, off_k, pitch_k, mod_lo_k, mod_hi_k) = match index {
        0 => (
            EGL_DMA_BUF_PLANE0_FD_EXT,
            EGL_DMA_BUF_PLANE0_OFFSET_EXT,
            EGL_DMA_BUF_PLANE0_PITCH_EXT,
            EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT,
            EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT,
        ),
        1 => (
            EGL_DMA_BUF_PLANE1_FD_EXT,
            EGL_DMA_BUF_PLANE1_OFFSET_EXT,
            EGL_DMA_BUF_PLANE1_PITCH_EXT,
            EGL_DMA_BUF_PLANE1_MODIFIER_LO_EXT,
            EGL_DMA_BUF_PLANE1_MODIFIER_HI_EXT,
        ),
        2 => (
            EGL_DMA_BUF_PLANE2_FD_EXT,
            EGL_DMA_BUF_PLANE2_OFFSET_EXT,
            EGL_DMA_BUF_PLANE2_PITCH_EXT,
            EGL_DMA_BUF_PLANE2_MODIFIER_LO_EXT,
            EGL_DMA_BUF_PLANE2_MODIFIER_HI_EXT,
        ),
        _ => return,
    };
    attrs.extend_from_slice(&[fd_k, fd, off_k, offset as EGLint, pitch_k, pitch]);
    // Pass modifier attrs when the extension exists (LINEAR=0 is valid; INVALID often omitted).
    if with_mod && modifier != DRM_FORMAT_MOD_INVALID {
        attrs.extend_from_slice(&[
            mod_lo_k,
            (modifier & 0xffff_ffff) as EGLint,
            mod_hi_k,
            ((modifier >> 32) & 0xffff_ffff) as EGLint,
        ]);
    }
}

/// Try to import `info` as an EGLImage and destroy it (M3 proof; encode still MemFd).
pub fn try_import(info: &DmaBufInfo) -> ImportAttempt {
    let Some(fourcc) = drm_fourcc_for(info.format) else {
        return early_fail(format!(
            "no DRM fourcc mapping for PipeWire format {:?}",
            info.format
        ));
    };
    if info.planes.is_empty() {
        return early_fail("DMA-BUF has no planes".into());
    }

    let mut guard = session_lock().lock().expect("egl session");
    let session = match &mut *guard {
        SessionState::Ready(s) => s,
        SessionState::Failed(_) => return ImportAttempt::Disabled,
    };

    let mut attrs: Vec<EGLint> = vec![
        EGL_WIDTH,
        info.width as EGLint,
        EGL_HEIGHT,
        info.height as EGLint,
        EGL_LINUX_DRM_FOURCC_EXT,
        fourcc as EGLint,
    ];
    let with_mod = session.has_modifiers;
    for (i, plane) in info.planes.iter().enumerate().take(3) {
        push_plane_attrs(
            &mut attrs,
            i,
            plane.fd,
            plane.offset,
            plane.stride,
            info.modifier,
            with_mod,
        );
    }
    attrs.push(EGL_NONE);

    unsafe {
        let image = (session.create_image)(
            session.display,
            EGL_NO_CONTEXT, // DMA-BUF images use NO_CONTEXT per EGL_EXT_image_dma_buf_import
            EGL_LINUX_DMA_BUF_EXT,
            ptr::null_mut(),
            attrs.as_ptr(),
        );
        if image.is_null() || image == EGL_NO_IMAGE_KHR {
            let err = eglGetError();
            let msg = format!(
                "eglCreateImageKHR failed err={err} fourcc={fourcc:#x} mod={:#x} planes={}",
                info.modifier,
                info.planes.len()
            );
            IMPORT_FAIL.fetch_add(1, Ordering::Relaxed);
            capture_metrics::note_egl_import_fail();
            *guard = SessionState::Failed(msg.clone());
            return ImportAttempt::Failed(msg);
        }
        let _ = (session.destroy_image)(session.display, image);
    }
    IMPORT_OK.fetch_add(1, Ordering::Relaxed);
    capture_metrics::note_egl_import_ok();
    if !LOGGED_OK.swap(true, Ordering::Relaxed) {
        eprintln!(
            "banter: EGL DMA-BUF import ok ({}x{} {:?} mod={:#x} planes={})",
            info.width,
            info.height,
            info.format,
            info.modifier,
            info.planes.len()
        );
    }
    ImportAttempt::Success
}

/// Sticky fail without opening an EGL session (bad format / empty planes).
fn early_fail(msg: String) -> ImportAttempt {
    if EARLY_FAIL.swap(true, Ordering::Relaxed) {
        return ImportAttempt::Disabled;
    }
    IMPORT_FAIL.fetch_add(1, Ordering::Relaxed);
    capture_metrics::note_egl_import_fail();
    ImportAttempt::Failed(msg)
}

pub fn import_ok() -> u64 {
    IMPORT_OK.load(Ordering::Relaxed)
}

#[allow(dead_code)] // Surfaced via capture_metrics; kept for direct probing.
pub fn import_fail() -> u64 {
    IMPORT_FAIL.load(Ordering::Relaxed)
}

pub fn clear_counters() {
    IMPORT_OK.store(0, Ordering::Relaxed);
    IMPORT_FAIL.store(0, Ordering::Relaxed);
    LOGGED_OK.store(false, Ordering::Relaxed);
    // Keep EARLY_FAIL / SESSION sticky across Go Live — driver capability does not change.
}

#[cfg(test)]
mod tests {
    use super::*;
    use pipewire::spa::param::video::VideoFormat;

    #[test]
    fn fourcc_bgra_is_argb8888() {
        let c = drm_fourcc_for(VideoFormat::BGRA).expect("bgra");
        assert_eq!(c, fourcc(b'A', b'R', b'2', b'4'));
    }

    #[test]
    fn fourcc_nv12() {
        assert_eq!(
            drm_fourcc_for(VideoFormat::NV12),
            Some(fourcc(b'N', b'V', b'1', b'2'))
        );
    }
}
