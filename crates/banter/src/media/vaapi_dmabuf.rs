//! DMA-BUF → VA-API hardware-frame encode (Linux Wayland M4).
//!
//! Imports PipeWire DMA-BUF planes via `VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME_2`,
//! optionally runs VideoProc (CSC/scale) to NV12, then encodes Constrained
//! Baseline H.264 with `VAEntrypointEncSlice`. Annex-B AUs feed the existing
//! WebRTC path — no Banter CPU BGRA→I420 convert on this path.
//!
//! Sticky-disables on init/import/encode failure so MemFd/CPU remains usable.
//! Headless CI without `/dev/dri` is fine (`available()` is false).

#![cfg(target_os = "linux")]
#![allow(non_snake_case, non_upper_case_globals)] // VA-API C layout / constants.

use std::ffi::c_void;
use std::os::fd::{AsRawFd, RawFd};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use pipewire::spa::param::video::VideoFormat;

use super::capture_metrics;
use super::dmabuf::{OwnedDmaBufFrame, DRM_FORMAT_MOD_INVALID};
use super::encode::{access_unit_is_key, EncodedAu};
use super::egl_dmabuf;

type VAStatus = i32;
type VADisplay = *mut c_void;
type VAConfigID = u32;
type VAContextID = u32;
type VASurfaceID = u32;
type VABufferID = u32;
type VAProfile = i32;
type VAEntrypoint = i32;
type VAGenericValueType = i32;

const VA_STATUS_SUCCESS: VAStatus = 0;
const VA_INVALID_ID: u32 = 0xffff_ffff;

const VA_FOURCC_NV12: u32 = u32::from_le_bytes(*b"NV12");

const VAProfileH264ConstrainedBaseline: VAProfile = 2;
const VAProfileNone: VAProfile = -1;
const VAEntrypointEncSlice: VAEntrypoint = 6;
const VAEntrypointVideoProc: VAEntrypoint = 10;

const VA_RT_FORMAT_YUV420: u32 = 0x00000001;
const VA_RT_FORMAT_RGB32: u32 = 0x00004000;

const VASurfaceAttribPixelFormat: i32 = 1;
const VASurfaceAttribMemoryType: i32 = 6;
const VASurfaceAttribExternalBufferDescriptor: i32 = 7;
const VAGenericValueTypeInteger: VAGenericValueType = 1;
const VAGenericValueTypePointer: VAGenericValueType = 4;

const VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME_2: u32 = 0x4000_0000;

const VAEncCodedBufferType: i32 = 21;
const VAEncSequenceParameterBufferType: i32 = 22;
const VAEncPictureParameterBufferType: i32 = 23;
const VAEncSliceParameterBufferType: i32 = 24;
const VAProcPipelineParameterBufferType: i32 = 41;

const VA_PADDING_LOW: usize = 4;

#[repr(C)]
struct VASurfaceAttrib {
    type_: i32,
    flags: i32,
    value: VAGenericValue,
}

#[repr(C)]
union VAGenericValue {
    i: i32,
    f: f32,
    p: *mut c_void,
    fn_: *mut c_void,
}

#[repr(C)]
struct VADRMPRIMESurfaceDescriptor {
    fourcc: u32,
    width: u32,
    height: u32,
    num_objects: u32,
    objects: [VADRMPRIMEObject; 4],
    num_layers: u32,
    layers: [VADRMPRIMELayer; 4],
}

#[repr(C)]
struct VADRMPRIMEObject {
    fd: i32,
    size: u32,
    drm_format_modifier: u64,
}

#[repr(C)]
struct VADRMPRIMELayer {
    drm_format: u32,
    num_planes: u32,
    object_index: [u32; 4],
    offset: [u32; 4],
    pitch: [u32; 4],
}

#[repr(C)]
struct VARectangle {
    x: i16,
    y: i16,
    width: u16,
    height: u16,
}

#[repr(C)]
struct VAProcPipelineParameterBuffer {
    surface: VASurfaceID,
    surface_region: *mut VARectangle,
    surface_color_standard: i32,
    output_region: *mut VARectangle,
    output_background_color: u32,
    output_color_standard: i32,
    pipeline_flags: u32,
    filter_flags: u32,
    filters: *mut VABufferID,
    num_filters: u32,
    forward_references: *mut VASurfaceID,
    num_forward_references: u32,
    backward_references: *mut VASurfaceID,
    num_backward_references: u32,
    // Additional fields exist in newer libva; zero-pad via struct size from API.
    // We allocate via vaCreateBuffer with sizeof this minimal layout — drivers
    // that require the full struct still accept zeroed trailing padding when
    // buffer size matches VAProcPipelineParameterBuffer from headers.
    rotation_state: u32,
    blend_state: *mut c_void,
    mirror_state: u32,
    additional_outputs: *mut VASurfaceID,
    num_additional_outputs: u32,
    input_surface_flag: u32,
    output_surface_flag: u32,
    reserved: [u32; VA_PADDING_LOW],
}

#[repr(C)]
struct VAPictureH264 {
    picture_id: VASurfaceID,
    frame_idx: u32,
    flags: u32,
    TopFieldOrderCnt: i32,
    BottomFieldOrderCnt: i32,
    va_reserved: [u32; VA_PADDING_LOW],
}

#[repr(C)]
struct VAEncSequenceParameterBufferH264 {
    seq_parameter_set_id: u8,
    level_idc: u8,
    intra_period: u32,
    intra_idr_period: u32,
    ip_period: u32,
    bits_per_second: u32,
    max_num_ref_frames: u32,
    picture_width_in_mbs: u16,
    picture_height_in_mbs: u16,
    seq_fields: u32,
    bit_depth_luma_minus8: u8,
    bit_depth_chroma_minus8: u8,
    num_ref_frames_in_pic_order_cnt_cycle: u8,
    offset_for_non_ref_pic: i32,
    offset_for_top_to_bottom_field: i32,
    offset_for_ref_frame: [i32; 256],
    frame_cropping_flag: u8,
    frame_crop_left_offset: u32,
    frame_crop_right_offset: u32,
    frame_crop_top_offset: u32,
    frame_crop_bottom_offset: u32,
    vui_parameters_present_flag: u8,
    vui_fields: u32,
    aspect_ratio_idc: u8,
    sar_width: u32,
    sar_height: u32,
    num_units_in_tick: u32,
    time_scale: u32,
    va_reserved: [u32; VA_PADDING_LOW],
}

#[repr(C)]
struct VAEncPictureParameterBufferH264 {
    CurrPic: VAPictureH264,
    ReferenceFrames: [VAPictureH264; 16],
    coded_buf: VABufferID,
    pic_parameter_set_id: u8,
    seq_parameter_set_id: u8,
    last_picture: u8,
    frame_num: u16,
    pic_init_qp: u8,
    num_ref_idx_l0_active_minus1: u8,
    num_ref_idx_l1_active_minus1: u8,
    chroma_qp_index_offset: i8,
    second_chroma_qp_index_offset: i8,
    pic_fields: u32,
    va_reserved: [u32; VA_PADDING_LOW],
}

#[repr(C)]
struct VAEncSliceParameterBufferH264 {
    macroblock_address: u32,
    num_macroblocks: u32,
    macroblock_info: VABufferID,
    slice_type: u8,
    pic_parameter_set_id: u8,
    idr_pic_id: u16,
    pic_order_cnt_lsb: u16,
    delta_pic_order_cnt_bottom: i32,
    delta_pic_order_cnt: [i32; 2],
    direct_spatial_mv_pred_flag: u8,
    num_ref_idx_active_override_flag: u8,
    num_ref_idx_l0_active_minus1: u8,
    num_ref_idx_l1_active_minus1: u8,
    RefPicList0: [VAPictureH264; 32],
    RefPicList1: [VAPictureH264; 32],
    luma_log2_weight_denom: u8,
    chroma_log2_weight_denom: u8,
    luma_weight_l0_flag: u8,
    luma_weight_l0: [i16; 32],
    luma_offset_l0: [i16; 32],
    chroma_weight_l0_flag: u8,
    chroma_weight_l0: [[i16; 2]; 32],
    chroma_offset_l0: [[i16; 2]; 32],
    luma_weight_l1_flag: u8,
    luma_weight_l1: [i16; 32],
    luma_offset_l1: [i16; 32],
    chroma_weight_l1_flag: u8,
    chroma_weight_l1: [[i16; 2]; 32],
    chroma_offset_l1: [[i16; 2]; 32],
    cabac_init_idc: u8,
    slice_qp_delta: i8,
    disable_deblocking_filter_idc: u8,
    slice_alpha_c0_offset_div2: i8,
    slice_beta_offset_div2: i8,
    va_reserved: [u32; VA_PADDING_LOW],
}

#[repr(C)]
struct VACodedBufferSegment {
    size: u32,
    bit_offset: u32,
    status: u32,
    reserved: u32,
    buf: *mut c_void,
    next: *mut VACodedBufferSegment,
}

#[link(name = "va")]
#[link(name = "va-drm")]
extern "C" {
    fn vaGetDisplayDRM(fd: i32) -> VADisplay;
    fn vaInitialize(dpy: VADisplay, major: *mut i32, minor: *mut i32) -> VAStatus;
    fn vaTerminate(dpy: VADisplay) -> VAStatus;
    fn vaQueryConfigProfiles(
        dpy: VADisplay,
        profile_list: *mut VAProfile,
        num_profiles: *mut i32,
    ) -> VAStatus;
    fn vaQueryConfigEntrypoints(
        dpy: VADisplay,
        profile: VAProfile,
        entrypoint_list: *mut VAEntrypoint,
        num_entrypoints: *mut i32,
    ) -> VAStatus;
    fn vaCreateConfig(
        dpy: VADisplay,
        profile: VAProfile,
        entrypoint: VAEntrypoint,
        attrib_list: *mut c_void,
        num_attribs: i32,
        config_id: *mut VAConfigID,
    ) -> VAStatus;
    fn vaDestroyConfig(dpy: VADisplay, config_id: VAConfigID) -> VAStatus;
    fn vaCreateContext(
        dpy: VADisplay,
        config_id: VAConfigID,
        picture_width: i32,
        picture_height: i32,
        flag: i32,
        render_targets: *mut VASurfaceID,
        num_render_targets: i32,
        context: *mut VAContextID,
    ) -> VAStatus;
    fn vaDestroyContext(dpy: VADisplay, context: VAContextID) -> VAStatus;
    fn vaCreateSurfaces(
        dpy: VADisplay,
        format: u32,
        width: u32,
        height: u32,
        surfaces: *mut VASurfaceID,
        num_surfaces: u32,
        attrib_list: *mut VASurfaceAttrib,
        num_attribs: u32,
    ) -> VAStatus;
    fn vaDestroySurfaces(dpy: VADisplay, surfaces: *mut VASurfaceID, num_surfaces: i32) -> VAStatus;
    fn vaCreateBuffer(
        dpy: VADisplay,
        context: VAContextID,
        type_: i32,
        size: u32,
        num_elements: u32,
        data: *mut c_void,
        buf_id: *mut VABufferID,
    ) -> VAStatus;
    fn vaDestroyBuffer(dpy: VADisplay, buf_id: VABufferID) -> VAStatus;
    fn vaMapBuffer(dpy: VADisplay, buf_id: VABufferID, pbuf: *mut *mut c_void) -> VAStatus;
    fn vaUnmapBuffer(dpy: VADisplay, buf_id: VABufferID) -> VAStatus;
    fn vaBeginPicture(dpy: VADisplay, context: VAContextID, render_target: VASurfaceID) -> VAStatus;
    fn vaRenderPicture(
        dpy: VADisplay,
        context: VAContextID,
        buffers: *mut VABufferID,
        num_buffers: i32,
    ) -> VAStatus;
    fn vaEndPicture(dpy: VADisplay, context: VAContextID) -> VAStatus;
    fn vaSyncSurface(dpy: VADisplay, render_target: VASurfaceID) -> VAStatus;
}

fn fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    u32::from(a) | (u32::from(b) << 8) | (u32::from(c) << 16) | (u32::from(d) << 24)
}

fn drm_fourcc_for(format: VideoFormat) -> Option<u32> {
    egl_dmabuf::drm_fourcc_for(format)
}

fn va_fourcc_for_drm(drm: u32) -> u32 {
    // VA fourcc often matches DRM for NV12; RGB uses VA RGB variants.
    if drm == fourcc(b'N', b'V', b'1', b'2') {
        VA_FOURCC_NV12
    } else if drm == fourcc(b'A', b'R', b'2', b'4') || drm == fourcc(b'X', b'R', b'2', b'4') {
        fourcc(b'X', b'R', b'2', b'4') // VA_FOURCC_XRGB
    } else if drm == fourcc(b'A', b'B', b'2', b'4') || drm == fourcc(b'X', b'B', b'2', b'4') {
        fourcc(b'X', b'B', b'2', b'4')
    } else {
        drm
    }
}

static DISABLED: AtomicBool = AtomicBool::new(false);
static HW_OK: AtomicU64 = AtomicU64::new(0);
static HW_FAIL: AtomicU64 = AtomicU64::new(0);
static LOGGED_OK: AtomicBool = AtomicBool::new(false);

/// True when a DRM render node exists and VA-API has not been sticky-disabled.
pub fn available() -> bool {
    if DISABLED.load(Ordering::Relaxed) {
        return false;
    }
    if std::env::var("BANTER_VAAPI_DMABUF")
        .map(|v| v == "0" || v.eq_ignore_ascii_case("off") || v.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
    {
        return false;
    }
    std::path::Path::new("/dev/dri/renderD128").exists()
        || std::path::Path::new("/dev/dri/renderD129").exists()
}

/// Prefer advertising DMA-BUF ahead of MemFd when the HW path can run.
pub fn prefer_dmabuf_enum() -> bool {
    available() && super::dmabuf::advertise_dmabuf()
}

pub fn hw_frames_ok() -> u64 {
    HW_OK.load(Ordering::Relaxed)
}

pub fn hw_frames_fail() -> u64 {
    HW_FAIL.load(Ordering::Relaxed)
}

pub fn clear_counters() {
    HW_OK.store(0, Ordering::Relaxed);
    HW_FAIL.store(0, Ordering::Relaxed);
    LOGGED_OK.store(false, Ordering::Relaxed);
}

fn sticky_disable(reason: &str) {
    if !DISABLED.swap(true, Ordering::Relaxed) {
        eprintln!("banter: VA-API DMA-BUF path disabled ({reason}); MemFd/CPU fallback stays");
        capture_metrics::note_vaapi_hw_fail();
        HW_FAIL.fetch_add(1, Ordering::Relaxed);
    }
}

struct VaDisplayGuard {
    dpy: VADisplay,
    drm_fd: RawFd,
}

// VA display/context handles are used only under SESSION mutex.
unsafe impl Send for VaDisplayGuard {}
unsafe impl Send for VaSession {}

impl Drop for VaDisplayGuard {
    fn drop(&mut self) {
        unsafe {
            if !self.dpy.is_null() {
                let _ = vaTerminate(self.dpy);
            }
            if self.drm_fd >= 0 {
                libc::close(self.drm_fd);
            }
        }
    }
}

fn open_display() -> Result<VaDisplayGuard, String> {
    let mut last = "no /dev/dri/renderD*".to_string();
    for path in ["/dev/dri/renderD128", "/dev/dri/renderD129", "/dev/dri/card0"] {
        let c = std::ffi::CString::new(path).expect("path");
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if fd < 0 {
            last = format!("open {path} failed");
            continue;
        }
        let dpy = unsafe { vaGetDisplayDRM(fd) };
        if dpy.is_null() {
            unsafe { libc::close(fd) };
            last = format!("vaGetDisplayDRM({path}) null");
            continue;
        }
        let mut major = 0;
        let mut minor = 0;
        let st = unsafe { vaInitialize(dpy, &mut major, &mut minor) };
        if st != VA_STATUS_SUCCESS {
            unsafe {
                let _ = vaTerminate(dpy);
                libc::close(fd);
            }
            last = format!("vaInitialize failed status={st}");
            continue;
        }
        return Ok(VaDisplayGuard { dpy, drm_fd: fd });
    }
    Err(last)
}

fn has_entrypoint(dpy: VADisplay, profile: VAProfile, want: VAEntrypoint) -> bool {
    let mut entries = [0i32; 32];
    let mut n = 0i32;
    let st = unsafe { vaQueryConfigEntrypoints(dpy, profile, entries.as_mut_ptr(), &mut n) };
    if st != VA_STATUS_SUCCESS {
        return false;
    }
    entries[..n.max(0) as usize].iter().any(|&e| e == want)
}

struct LayerEnc {
    width: u32,
    height: u32,
    config: VAConfigID,
    context: VAContextID,
    recon: VASurfaceID,
    nv12: VASurfaceID,
    coded: VABufferID,
    frame_num: u16,
    idr_pic_id: u16,
    bps: u32,
    gop: u32,
}

struct VaSession {
    display: VaDisplayGuard,
    vpp_config: VAConfigID,
    vpp_context: VAContextID,
    layers: Vec<LayerEnc>,
}

impl Drop for VaSession {
    fn drop(&mut self) {
        unsafe {
            let dpy = self.display.dpy;
            for layer in &mut self.layers {
                if layer.coded != VA_INVALID_ID {
                    let _ = vaDestroyBuffer(dpy, layer.coded);
                }
                let mut surfaces = [layer.recon, layer.nv12];
                let _ = vaDestroySurfaces(dpy, surfaces.as_mut_ptr(), 2);
                if layer.context != VA_INVALID_ID {
                    let _ = vaDestroyContext(dpy, layer.context);
                }
                if layer.config != VA_INVALID_ID {
                    let _ = vaDestroyConfig(dpy, layer.config);
                }
            }
            if self.vpp_context != VA_INVALID_ID {
                let _ = vaDestroyContext(dpy, self.vpp_context);
            }
            if self.vpp_config != VA_INVALID_ID {
                let _ = vaDestroyConfig(dpy, self.vpp_config);
            }
        }
    }
}

static SESSION: OnceLock<Mutex<Option<Result<VaSession, String>>>> = OnceLock::new();

fn session_slot() -> &'static Mutex<Option<Result<VaSession, String>>> {
    SESSION.get_or_init(|| Mutex::new(None))
}

fn ensure_session(
    layers: &[(u32, u32, u32)],
    fps: f32,
    keyframe_ms: u64,
) -> Result<(), String> {
    let mut guard = session_slot().lock().expect("va session");
    if let Some(Ok(sess)) = guard.as_mut() {
        if sess.layers.len() == layers.len()
            && sess
                .layers
                .iter()
                .zip(layers.iter())
                .all(|(l, (w, h, b))| l.width == *w && l.height == *h && l.bps == *b)
        {
            return Ok(());
        }
        // Size/bitrate change — rebuild.
        *guard = None;
    }
    if let Some(Err(e)) = guard.as_ref() {
        return Err(e.clone());
    }
    match build_session(layers, fps, keyframe_ms) {
        Ok(sess) => {
            eprintln!(
                "banter: VA-API DMA-BUF encode ready ({} layer(s))",
                sess.layers.len()
            );
            *guard = Some(Ok(sess));
            Ok(())
        }
        Err(e) => {
            sticky_disable(&e);
            *guard = Some(Err(e.clone()));
            Err(e)
        }
    }
}

fn build_session(
    layers: &[(u32, u32, u32)],
    fps: f32,
    keyframe_ms: u64,
) -> Result<VaSession, String> {
    let display = open_display()?;
    let dpy = display.dpy;
    if !has_entrypoint(dpy, VAProfileH264ConstrainedBaseline, VAEntrypointEncSlice)
        && !has_entrypoint(dpy, VAProfileNone, VAEntrypointEncSlice)
    {
        // Some drivers only list Baseline under a different profile id; still try create.
    }
    let mut vpp_config = VA_INVALID_ID;
    let mut vpp_context = VA_INVALID_ID;
    if has_entrypoint(dpy, VAProfileNone, VAEntrypointVideoProc) {
        let st = unsafe {
            vaCreateConfig(
                dpy,
                VAProfileNone,
                VAEntrypointVideoProc,
                ptr::null_mut(),
                0,
                &mut vpp_config,
            )
        };
        if st == VA_STATUS_SUCCESS {
            let st = unsafe {
                vaCreateContext(
                    dpy,
                    vpp_config,
                    0,
                    0,
                    0,
                    ptr::null_mut(),
                    0,
                    &mut vpp_context,
                )
            };
            if st != VA_STATUS_SUCCESS {
                let _ = unsafe { vaDestroyConfig(dpy, vpp_config) };
                vpp_config = VA_INVALID_ID;
                vpp_context = VA_INVALID_ID;
            }
        }
    }

    let gop = ((fps.max(1.0) * (keyframe_ms.max(500) as f32 / 1000.0)) as u32).max(1);
    let mut built = Vec::new();
    for &(width, height, bps) in layers {
        let w = width & !1;
        let h = height & !1;
        if w < 128 || h < 128 {
            return Err(format!("VA-API encode size {w}x{h} below 128"));
        }
        let mut config = VA_INVALID_ID;
        let st = unsafe {
            vaCreateConfig(
                dpy,
                VAProfileH264ConstrainedBaseline,
                VAEntrypointEncSlice,
                ptr::null_mut(),
                0,
                &mut config,
            )
        };
        if st != VA_STATUS_SUCCESS {
            return Err(format!("vaCreateConfig EncSlice failed status={st}"));
        }
        let mut nv12 = VA_INVALID_ID;
        let mut recon = VA_INVALID_ID;
        let st = unsafe {
            vaCreateSurfaces(
                dpy,
                VA_RT_FORMAT_YUV420,
                w,
                h,
                &mut nv12,
                1,
                ptr::null_mut(),
                0,
            )
        };
        if st != VA_STATUS_SUCCESS {
            let _ = unsafe { vaDestroyConfig(dpy, config) };
            return Err(format!("vaCreateSurfaces nv12 failed status={st}"));
        }
        let st = unsafe {
            vaCreateSurfaces(
                dpy,
                VA_RT_FORMAT_YUV420,
                w,
                h,
                &mut recon,
                1,
                ptr::null_mut(),
                0,
            )
        };
        if st != VA_STATUS_SUCCESS {
            let mut s = [nv12];
            let _ = unsafe { vaDestroySurfaces(dpy, s.as_mut_ptr(), 1) };
            let _ = unsafe { vaDestroyConfig(dpy, config) };
            return Err(format!("vaCreateSurfaces recon failed status={st}"));
        }
        let mut context = VA_INVALID_ID;
        let mut targets = [recon, nv12];
        let st = unsafe {
            vaCreateContext(
                dpy,
                config,
                w as i32,
                h as i32,
                0,
                targets.as_mut_ptr(),
                2,
                &mut context,
            )
        };
        if st != VA_STATUS_SUCCESS {
            let mut s = [recon, nv12];
            let _ = unsafe { vaDestroySurfaces(dpy, s.as_mut_ptr(), 2) };
            let _ = unsafe { vaDestroyConfig(dpy, config) };
            return Err(format!("vaCreateContext failed status={st}"));
        }
        let coded_size = (w as u64 * h as u64 * 3 / 2).max(1 << 20) as u32;
        let mut coded = VA_INVALID_ID;
        let st = unsafe {
            vaCreateBuffer(
                dpy,
                context,
                VAEncCodedBufferType,
                coded_size,
                1,
                ptr::null_mut(),
                &mut coded,
            )
        };
        if st != VA_STATUS_SUCCESS {
            let _ = unsafe { vaDestroyContext(dpy, context) };
            let mut s = [recon, nv12];
            let _ = unsafe { vaDestroySurfaces(dpy, s.as_mut_ptr(), 2) };
            let _ = unsafe { vaDestroyConfig(dpy, config) };
            return Err(format!("vaCreateBuffer coded failed status={st}"));
        }
        built.push(LayerEnc {
            width: w,
            height: h,
            config,
            context,
            recon,
            nv12,
            coded,
            frame_num: 0,
            idr_pic_id: 0,
            bps,
            gop,
        });
    }
    Ok(VaSession {
        display,
        vpp_config,
        vpp_context,
        layers: built,
    })
}

fn import_prime(dpy: VADisplay, frame: &OwnedDmaBufFrame) -> Result<VASurfaceID, String> {
    let drm = drm_fourcc_for(frame.format).ok_or_else(|| {
        format!("no DRM fourcc for PipeWire format {:?}", frame.format)
    })?;
    let va_fcc = va_fourcc_for_drm(drm);
    let mut desc: VADRMPRIMESurfaceDescriptor = unsafe { std::mem::zeroed() };
    desc.fourcc = va_fcc;
    desc.width = frame.width;
    desc.height = frame.height;
    desc.num_objects = frame.planes.len().min(4) as u32;
    desc.num_layers = 1;
    desc.layers[0].drm_format = drm;
    desc.layers[0].num_planes = desc.num_objects;
    for (i, plane) in frame.planes.iter().take(4).enumerate() {
        desc.objects[i].fd = plane.fd.as_raw_fd();
        desc.objects[i].size = plane.size.max(1);
        desc.objects[i].drm_format_modifier = if frame.modifier == DRM_FORMAT_MOD_INVALID {
            0
        } else {
            frame.modifier
        };
        desc.layers[0].object_index[i] = i as u32;
        desc.layers[0].offset[i] = plane.offset;
        desc.layers[0].pitch[i] = plane.stride.max(0) as u32;
    }

    // flags: VA_SURFACE_ATTRIB_SETTABLE = 0x00000002
    let mem_type = VASurfaceAttrib {
        type_: VASurfaceAttribMemoryType,
        flags: 2,
        value: VAGenericValue {
            i: VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME_2 as i32,
        },
    };
    let ext = VASurfaceAttrib {
        type_: VASurfaceAttribExternalBufferDescriptor,
        flags: 2,
        value: VAGenericValue {
            p: &mut desc as *mut _ as *mut c_void,
        },
    };
    let pix = VASurfaceAttrib {
        type_: VASurfaceAttribPixelFormat,
        flags: 2,
        value: VAGenericValue {
            i: va_fcc as i32,
        },
    };
    let mut attribs = [mem_type, ext, pix];
    let rt = if frame.format == VideoFormat::NV12 || frame.format == VideoFormat::I420 {
        VA_RT_FORMAT_YUV420
    } else {
        VA_RT_FORMAT_RGB32
    };
    let mut surface = VA_INVALID_ID;
    let st = unsafe {
        vaCreateSurfaces(
            dpy,
            rt,
            frame.width,
            frame.height,
            &mut surface,
            1,
            attribs.as_mut_ptr(),
            attribs.len() as u32,
        )
    };
    if st != VA_STATUS_SUCCESS {
        return Err(format!(
            "vaCreateSurfaces PRIME import failed status={st} fmt={drm:#x} mod={:#x}",
            frame.modifier
        ));
    }
    Ok(surface)
}

fn vpp_copy(
    dpy: VADisplay,
    vpp_ctx: VAContextID,
    src: VASurfaceID,
    dst: VASurfaceID,
    sw: u32,
    sh: u32,
    dw: u32,
    dh: u32,
) -> Result<(), String> {
    if vpp_ctx == VA_INVALID_ID {
        return Err("VA VideoProc entrypoint missing".into());
    }
    let mut src_reg = VARectangle {
        x: 0,
        y: 0,
        width: sw as u16,
        height: sh as u16,
    };
    let mut dst_reg = VARectangle {
        x: 0,
        y: 0,
        width: dw as u16,
        height: dh as u16,
    };
    let mut param: VAProcPipelineParameterBuffer = unsafe { std::mem::zeroed() };
    param.surface = src;
    param.surface_region = &mut src_reg;
    param.output_region = &mut dst_reg;
    let mut buf = VA_INVALID_ID;
    let st = unsafe {
        vaCreateBuffer(
            dpy,
            vpp_ctx,
            VAProcPipelineParameterBufferType,
            std::mem::size_of::<VAProcPipelineParameterBuffer>() as u32,
            1,
            &mut param as *mut _ as *mut c_void,
            &mut buf,
        )
    };
    if st != VA_STATUS_SUCCESS {
        return Err(format!("vaCreateBuffer VPP failed status={st}"));
    }
    let st = unsafe { vaBeginPicture(dpy, vpp_ctx, dst) };
    if st != VA_STATUS_SUCCESS {
        let _ = unsafe { vaDestroyBuffer(dpy, buf) };
        return Err(format!("vaBeginPicture VPP failed status={st}"));
    }
    let mut buffers = [buf];
    let st = unsafe { vaRenderPicture(dpy, vpp_ctx, buffers.as_mut_ptr(), 1) };
    let st2 = unsafe { vaEndPicture(dpy, vpp_ctx) };
    let _ = unsafe { vaDestroyBuffer(dpy, buf) };
    if st != VA_STATUS_SUCCESS || st2 != VA_STATUS_SUCCESS {
        return Err(format!("VPP render failed status={st}/{st2}"));
    }
    let st = unsafe { vaSyncSurface(dpy, dst) };
    if st != VA_STATUS_SUCCESS {
        return Err(format!("vaSyncSurface VPP failed status={st}"));
    }
    Ok(())
}

fn encode_layer(
    dpy: VADisplay,
    layer: &mut LayerEnc,
    input: VASurfaceID,
    force_key: bool,
) -> Result<EncodedAu, String> {
    let idr = force_key || layer.frame_num == 0 || (layer.frame_num as u32 % layer.gop == 0);
    let mut seq: VAEncSequenceParameterBufferH264 = unsafe { std::mem::zeroed() };
    seq.level_idc = 31;
    seq.intra_period = layer.gop;
    seq.intra_idr_period = layer.gop;
    seq.ip_period = 1;
    seq.bits_per_second = layer.bps;
    seq.max_num_ref_frames = 1;
    seq.picture_width_in_mbs = ((layer.width + 15) / 16) as u16;
    seq.picture_height_in_mbs = ((layer.height + 15) / 16) as u16;
    // frame_mbs_only_flag=1, chroma_format_idc=1
    seq.seq_fields = (1 << 2) | (1 << 0);

    let mut pic: VAEncPictureParameterBufferH264 = unsafe { std::mem::zeroed() };
    pic.CurrPic.picture_id = layer.recon;
    pic.CurrPic.frame_idx = layer.frame_num as u32;
    pic.CurrPic.flags = if idr { 0x20 } else { 0x20 }; // short-term ref-ish; drivers vary
    for r in &mut pic.ReferenceFrames {
        r.picture_id = VA_INVALID_ID;
        r.flags = 0x8000_0000; // VA_PICTURE_H264_INVALID
    }
    pic.coded_buf = layer.coded;
    pic.frame_num = layer.frame_num;
    pic.pic_init_qp = 26;
    let mut pic_fields: u32 = 0;
    if idr {
        pic_fields |= 1; // idr_pic_flag
        layer.idr_pic_id = layer.idr_pic_id.wrapping_add(1);
    }
    pic_fields |= 1 << 1; // reference_pic_flag
    pic.pic_fields = pic_fields;

    let mut slice: VAEncSliceParameterBufferH264 = unsafe { std::mem::zeroed() };
    slice.macroblock_address = 0;
    slice.num_macroblocks =
        (seq.picture_width_in_mbs as u32) * (seq.picture_height_in_mbs as u32);
    slice.slice_type = if idr { 2 } else { 0 }; // I=2, P=0 in VA
    slice.idr_pic_id = layer.idr_pic_id;
    slice.pic_order_cnt_lsb = (layer.frame_num as u16).wrapping_mul(2);
    for r in &mut slice.RefPicList0 {
        r.picture_id = VA_INVALID_ID;
        r.flags = 0x8000_0000;
    }
    for r in &mut slice.RefPicList1 {
        r.picture_id = VA_INVALID_ID;
        r.flags = 0x8000_0000;
    }

    let mut seq_buf = VA_INVALID_ID;
    let mut pic_buf = VA_INVALID_ID;
    let mut slice_buf = VA_INVALID_ID;
    let st = unsafe {
        vaCreateBuffer(
            dpy,
            layer.context,
            VAEncSequenceParameterBufferType,
            std::mem::size_of::<VAEncSequenceParameterBufferH264>() as u32,
            1,
            &mut seq as *mut _ as *mut c_void,
            &mut seq_buf,
        )
    };
    if st != VA_STATUS_SUCCESS {
        return Err(format!("seq buffer status={st}"));
    }
    let st = unsafe {
        vaCreateBuffer(
            dpy,
            layer.context,
            VAEncPictureParameterBufferType,
            std::mem::size_of::<VAEncPictureParameterBufferH264>() as u32,
            1,
            &mut pic as *mut _ as *mut c_void,
            &mut pic_buf,
        )
    };
    if st != VA_STATUS_SUCCESS {
        let _ = unsafe { vaDestroyBuffer(dpy, seq_buf) };
        return Err(format!("pic buffer status={st}"));
    }
    let st = unsafe {
        vaCreateBuffer(
            dpy,
            layer.context,
            VAEncSliceParameterBufferType,
            std::mem::size_of::<VAEncSliceParameterBufferH264>() as u32,
            1,
            &mut slice as *mut _ as *mut c_void,
            &mut slice_buf,
        )
    };
    if st != VA_STATUS_SUCCESS {
        let _ = unsafe { vaDestroyBuffer(dpy, seq_buf) };
        let _ = unsafe { vaDestroyBuffer(dpy, pic_buf) };
        return Err(format!("slice buffer status={st}"));
    }

    let st = unsafe { vaBeginPicture(dpy, layer.context, input) };
    if st != VA_STATUS_SUCCESS {
        let _ = unsafe { vaDestroyBuffer(dpy, seq_buf) };
        let _ = unsafe { vaDestroyBuffer(dpy, pic_buf) };
        let _ = unsafe { vaDestroyBuffer(dpy, slice_buf) };
        return Err(format!("vaBeginPicture encode status={st}"));
    }
    let mut buffers = [seq_buf, pic_buf, slice_buf];
    let st = unsafe { vaRenderPicture(dpy, layer.context, buffers.as_mut_ptr(), 3) };
    let st2 = unsafe { vaEndPicture(dpy, layer.context) };
    let _ = unsafe { vaDestroyBuffer(dpy, seq_buf) };
    let _ = unsafe { vaDestroyBuffer(dpy, pic_buf) };
    let _ = unsafe { vaDestroyBuffer(dpy, slice_buf) };
    if st != VA_STATUS_SUCCESS || st2 != VA_STATUS_SUCCESS {
        return Err(format!("encode render status={st}/{st2}"));
    }
    let st = unsafe { vaSyncSurface(dpy, layer.recon) };
    if st != VA_STATUS_SUCCESS {
        return Err(format!("vaSyncSurface encode status={st}"));
    }

    let mut mapped: *mut c_void = ptr::null_mut();
    let st = unsafe { vaMapBuffer(dpy, layer.coded, &mut mapped) };
    if st != VA_STATUS_SUCCESS || mapped.is_null() {
        return Err(format!("vaMapBuffer coded status={st}"));
    }
    let mut bytes = Vec::new();
    unsafe {
        let mut seg = mapped as *mut VACodedBufferSegment;
        while !seg.is_null() {
            let s = &*seg;
            if !s.buf.is_null() && s.size > 0 {
                let slice = std::slice::from_raw_parts(s.buf as *const u8, s.size as usize);
                bytes.extend_from_slice(slice);
            }
            seg = s.next;
        }
    }
    let _ = unsafe { vaUnmapBuffer(dpy, layer.coded) };
    if bytes.is_empty() {
        return Err("VA encode produced empty AU".into());
    }
    // Ensure Annex-B start code present.
    if bytes.len() < 4 || !(bytes.starts_with(&[0, 0, 0, 1]) || bytes.starts_with(&[0, 0, 1])) {
        let mut prefixed = Vec::with_capacity(bytes.len() + 4);
        prefixed.extend_from_slice(&[0, 0, 0, 1]);
        prefixed.extend_from_slice(&bytes);
        bytes = prefixed;
    }
    layer.frame_num = layer.frame_num.wrapping_add(1);
    Ok(EncodedAu {
        keyframe: idr || access_unit_is_key(&bytes),
        bytes,
    })
}

/// Encode one DMA-BUF frame into Annex-B AUs for each (width,height,bps) layer.
pub fn encode_owned_frame(
    frame: &OwnedDmaBufFrame,
    layers: &[(u32, u32, u32)],
    fps: f32,
    keyframe_ms: u64,
    force_key: bool,
) -> Result<Vec<EncodedAu>, String> {
    if !available() {
        return Err("VA-API DMA-BUF path unavailable".into());
    }
    ensure_session(layers, fps, keyframe_ms)?;
    let mut guard = session_slot().lock().expect("va session");
    let sess = match guard.as_mut() {
        Some(Ok(s)) => s,
        Some(Err(e)) => return Err(e.clone()),
        None => return Err("VA session missing".into()),
    };
    let dpy = sess.display.dpy;
    let started = Instant::now();
    let src = match import_prime(dpy, frame) {
        Ok(s) => s,
        Err(e) => {
            sticky_disable(&e);
            return Err(e);
        }
    };

    let mut out = Vec::with_capacity(sess.layers.len());
    let mut encode_err: Option<String> = None;
    for i in 0..sess.layers.len() {
        let (w, h) = {
            let layer = &sess.layers[i];
            (layer.width, layer.height)
        };
        // VPP into layer NV12 when size/format needs convert; same-size NV12 can skip.
        let need_vpp =
            frame.format != VideoFormat::NV12 || frame.width != w || frame.height != h;
        let input = if need_vpp {
            let nv12 = sess.layers[i].nv12;
            if let Err(e) = vpp_copy(
                dpy,
                sess.vpp_context,
                src,
                nv12,
                frame.width,
                frame.height,
                w,
                h,
            ) {
                encode_err = Some(e);
                break;
            }
            nv12
        } else {
            src
        };
        match encode_layer(dpy, &mut sess.layers[i], input, force_key) {
            Ok(au) => out.push(au),
            Err(e) => {
                encode_err = Some(e);
                break;
            }
        }
    }

    unsafe {
        let mut s = [src];
        let _ = vaDestroySurfaces(dpy, s.as_mut_ptr(), 1);
    }

    if let Some(e) = encode_err {
        sticky_disable(&e);
        return Err(e);
    }
    HW_OK.fetch_add(1, Ordering::Relaxed);
    capture_metrics::note_vaapi_hw_ok();
    let _ = started; // VPP+encode wall time is recorded by the encode loop.
    if !LOGGED_OK.swap(true, Ordering::Relaxed) {
        eprintln!(
            "banter: VA-API DMA-BUF encode ok ({}x{} {:?} → {} layer(s))",
            frame.width,
            frame.height,
            frame.format,
            out.len()
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefer_respects_disable_env() {
        // available() may be false in CI without /dev/dri — just ensure no panic.
        let _ = available();
        let _ = prefer_dmabuf_enum();
        let _ = hw_frames_ok();
    }

    #[test]
    fn fourcc_nv12_matches_egl() {
        assert_eq!(
            drm_fourcc_for(VideoFormat::NV12),
            egl_dmabuf::drm_fourcc_for(VideoFormat::NV12)
        );
    }
}
