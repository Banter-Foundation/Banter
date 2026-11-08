//! Windows Media Foundation H.264 encoder.
//!
//! The probe asks for a synchronous hardware MFT and sets the output profile
//! to Constrained Baseline (`eAVEncH264VProfile_ConstrainedBase`). The first
//! access unit is still checked by the caller. A missing MFT, an async-only
//! MFT, or any other profile falls through to OpenH264. Async hardware MFTs
//! are not driven: they need a Media Foundation event loop this process does
//! not run. Mid-stream `force_key` prefers `CODECAPI_AVEncVideoForceKeyFrame`
//! when the MFT exposes `ICodecAPI`; size changes and CodecAPI failure reopen
//! the MFT for a fresh IDR. A buffered first AU is flushed with a duplicate
//! frame before falling back.

use std::ptr;

use windows::core::{Interface, GUID, VARIANT};
use windows::Win32::Media::MediaFoundation::{
    eAVEncH264VProfile_ConstrainedBase, ICodecAPI, IMFActivate, IMFMediaBuffer, IMFSample,
    IMFTransform, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video,
    MFShutdown, MFStartup, MFTEnumEx, MFVideoFormat_H264, MFVideoFormat_NV12,
    MFVideoInterlace_Progressive, CODECAPI_AVEncVideoForceKeyFrame, MFSTARTUP_NOSOCKET,
    MFT_CATEGORY_VIDEO_DECODER, MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG, MFT_ENUM_FLAG_HARDWARE,
    MFT_ENUM_FLAG_SYNCMFT, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_START_OF_STREAM,
    MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO,
    MF_E_TRANSFORM_NEED_MORE_INPUT, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
    MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE, MF_MT_SUBTYPE, MF_VERSION,
};
use windows::Win32::System::Com::CoTaskMemFree;

/// `CODECAPI_AVLowLatencyMode` — prefer low delay over throughput (OBS-style interactive).
const CODECAPI_AV_LOW_LATENCY_MODE: GUID =
    GUID::from_u128(0x9c27891a_ed7a_40e1_88e8_d227f6639a54);
/// `CODECAPI_AVEncMPVDefaultBPictureCount` — force zero B-frames when the MFT honors it.
const CODECAPI_AV_ENC_MPV_DEFAULT_B_PICTURE_COUNT: GUID =
    GUID::from_u128(0xb098d157_9db1_4d2e_b8a1_d3a0f6e4b1b7);

use super::encode::{access_unit_is_key, Backend, EncodedAu, VideoEncoder};

pub struct MfEncoder {
    transform: IMFTransform,
    width: u32,
    height: u32,
    fps: f32,
    bps: u32,
    stamp: i64,
    provides: bool,
}

// Each encoder is owned by one media worker; COM pointers are not moved across threads while in use.
unsafe impl Send for MfEncoder {}

pub fn hardware_present() -> bool {
    unsafe { enumerate().map(|count| count > 0).unwrap_or(false) }
}

pub fn open(width: u32, height: u32, fps: f32, bps: u32) -> Result<MfEncoder, String> {
    unsafe { open_inner(width, height, fps, bps) }
}

impl VideoEncoder for MfEncoder {
    fn backend(&self) -> Backend {
        Backend::MediaFoundation
    }

    fn encode(
        &mut self,
        i420: &[u8],
        width: u32,
        height: u32,
        force_key: bool,
    ) -> Result<EncodedAu, String> {
        // Size change always reopens. force_key prefers CodecAPI mid-stream IDR;
        // reopen remains the fallback when CodecAPI is missing or the AU is not a
        // key (BAN-33 / BAN-35 late-join path). Returning Err used to permanently
        // swap LayerEncoder to OpenH264 mid-GOP.
        if width != self.width || height != self.height {
            let replacement = open(width, height, self.fps, self.bps)?;
            *self = replacement;
        } else if force_key {
            match unsafe { self.encode_forced_key(i420) } {
                Ok(au) => return Ok(au),
                Err(_) => {
                    let replacement = open(width, height, self.fps, self.bps)?;
                    *self = replacement;
                }
            }
        }
        unsafe { self.encode_inner(i420) }
    }
}

impl Drop for MfEncoder {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
        }
    }
}

unsafe fn enumerate() -> Result<u32, String> {
    MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).map_err(|err| err.to_string())?;
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_NV12,
    };
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };
    let flags = MFT_ENUM_FLAG(MFT_ENUM_FLAG_HARDWARE.0 | MFT_ENUM_FLAG_SYNCMFT.0);
    let mut activates: *mut Option<IMFActivate> = ptr::null_mut();
    let mut count = 0u32;
    let listed = MFTEnumEx(
        MFT_CATEGORY_VIDEO_ENCODER,
        flags,
        Some(&input),
        Some(&output),
        &mut activates,
        &mut count,
    );
    if !activates.is_null() {
        for index in 0..count {
            let _ = (*activates.add(index as usize)).take();
        }
        CoTaskMemFree(Some(activates.cast()));
    }
    listed.map_err(|err| err.to_string())?;
    Ok(count)
}

unsafe fn open_inner(width: u32, height: u32, fps: f32, bps: u32) -> Result<MfEncoder, String> {
    MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).map_err(|err| err.to_string())?;
    let activate = first_activate()?;
    let transform: IMFTransform = activate.ActivateObject().map_err(|err| err.to_string())?;
    let output = video_type(MFVideoFormat_H264, width, height, fps)?;
    output
        .SetUINT32(&MF_MT_AVG_BITRATE, bps)
        .map_err(|err| err.to_string())?;
    output
        .SetUINT32(
            &MF_MT_MPEG2_PROFILE,
            eAVEncH264VProfile_ConstrainedBase.0 as u32,
        )
        .map_err(|err| err.to_string())?;
    transform
        .SetOutputType(0, &output, 0)
        .map_err(|err| err.to_string())?;
    let input = video_type(MFVideoFormat_NV12, width, height, fps)?;
    transform
        .SetInputType(0, &input, 0)
        .map_err(|err| err.to_string())?;
    // Best-effort interactive CodecAPI knobs (same intent as ffmpeg `-tune ull` /
    // `-bf 0`). Unsupported properties are ignored so older MFTs still open.
    apply_interactive_codecapi(&transform);
    transform
        .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
        .map_err(|err| err.to_string())?;
    transform
        .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
        .map_err(|err| err.to_string())?;
    let info = transform
        .GetOutputStreamInfo(0)
        .map_err(|err| err.to_string())?;
    Ok(MfEncoder {
        transform,
        width,
        height,
        fps,
        bps,
        stamp: 0,
        provides: info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0,
    })
}

unsafe fn apply_interactive_codecapi(transform: &IMFTransform) {
    let Ok(codec) = transform.cast::<ICodecAPI>() else {
        return;
    };
    let _ = codec.SetValue(&CODECAPI_AV_LOW_LATENCY_MODE, &VARIANT::from(true));
    let _ = codec.SetValue(
        &CODECAPI_AV_ENC_MPV_DEFAULT_B_PICTURE_COUNT,
        &VARIANT::from(0u32),
    );
}

unsafe fn first_activate() -> Result<IMFActivate, String> {
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_NV12,
    };
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };
    let flags = MFT_ENUM_FLAG(MFT_ENUM_FLAG_HARDWARE.0 | MFT_ENUM_FLAG_SYNCMFT.0);
    let mut activates: *mut Option<IMFActivate> = ptr::null_mut();
    let mut count = 0u32;
    MFTEnumEx(
        MFT_CATEGORY_VIDEO_ENCODER,
        flags,
        Some(&input),
        Some(&output),
        &mut activates,
        &mut count,
    )
    .map_err(|err| err.to_string())?;
    if activates.is_null() || count == 0 {
        return Err("no synchronous hardware H.264 encoder".into());
    }
    let chosen = (*activates)
        .take()
        .ok_or_else(|| "hardware encoder did not activate".to_string())?;
    for index in 1..count {
        let _ = (*activates.add(index as usize)).take();
    }
    CoTaskMemFree(Some(activates.cast()));
    Ok(chosen)
}

unsafe fn video_type(
    subtype: windows::core::GUID,
    width: u32,
    height: u32,
    fps: f32,
) -> Result<windows::Win32::Media::MediaFoundation::IMFMediaType, String> {
    let kind = MFCreateMediaType().map_err(|err| err.to_string())?;
    kind.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
        .map_err(|err| err.to_string())?;
    kind.SetGUID(&MF_MT_SUBTYPE, &subtype)
        .map_err(|err| err.to_string())?;
    kind.SetUINT64(&MF_MT_FRAME_SIZE, ((height as u64) << 32) | width as u64)
        .map_err(|err| err.to_string())?;
    let rate = fps.max(1.0).round() as u64;
    kind.SetUINT64(&MF_MT_FRAME_RATE, (rate << 32) | 1)
        .map_err(|err| err.to_string())?;
    kind.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
        .map_err(|err| err.to_string())?;
    Ok(kind)
}

impl MfEncoder {
    /// Ask the MFT for an IDR via CodecAPI, then encode. Fails if CodecAPI is
    /// unavailable or the resulting AU is not a keyframe so the caller can reopen.
    unsafe fn encode_forced_key(&mut self, i420: &[u8]) -> Result<EncodedAu, String> {
        let codec: ICodecAPI = self
            .transform
            .cast()
            .map_err(|err| format!("ICodecAPI: {err}"))?;
        codec
            .IsSupported(&CODECAPI_AVEncVideoForceKeyFrame)
            .map_err(|err| format!("ForceKeyFrame unsupported: {err}"))?;
        let value = VARIANT::from(1u32);
        codec
            .SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &value)
            .map_err(|err| format!("ForceKeyFrame set: {err}"))?;
        let au = self.encode_inner(i420)?;
        if au.keyframe || access_unit_is_key(&au.bytes) {
            Ok(au)
        } else {
            Err("CodecAPI ForceKeyFrame did not produce an IDR".into())
        }
    }

    unsafe fn encode_inner(&mut self, i420: &[u8]) -> Result<EncodedAu, String> {
        let nv12 = i420_to_nv12(i420, self.width, self.height);
        if nv12.is_empty() {
            return Err("media foundation encoder got a short picture".into());
        }
        // Some sync MFTs hold the first AU until a second input (same class of bug as
        // VA-API async_depth on Linux). Feed once, and on NEED_MORE_INPUT duplicate the
        // frame to flush before giving up to OpenH264.
        let au = match self.push_frame(&nv12)? {
            Some(bytes) => bytes,
            None => {
                match self.push_frame(&nv12)? {
                    Some(bytes) => bytes,
                    None => {
                        return Err("media foundation encoder buffered the frame".into());
                    }
                }
            }
        };
        if au.is_empty() {
            return Err("media foundation encoder produced an empty sample".into());
        }
        Ok(EncodedAu {
            keyframe: access_unit_is_key(&au),
            bytes: au,
        })
    }

    unsafe fn push_frame(&mut self, nv12: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let sample = sample_from(nv12, self.stamp, self.fps)?;
        let step = (10_000_000.0 / self.fps.max(1.0)) as i64;
        self.stamp = self.stamp.saturating_add(step);
        self.transform
            .ProcessInput(0, &sample, 0)
            .map_err(|err| err.to_string())?;
        let mut status = 0u32;
        let mut buffers = [output_buffer(self.provides, nv12.len())?];
        let produced = self.transform.ProcessOutput(0, &mut buffers, &mut status);
        match produced {
            Ok(()) => Ok(Some(read_sample(buffers[0].pSample.as_ref())?)),
            Err(err) if err.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => Ok(None),
            Err(err) => Err(err.to_string()),
        }
    }
}

unsafe fn output_buffer(provides: bool, size: usize) -> Result<MFT_OUTPUT_DATA_BUFFER, String> {
    let sample = if provides {
        None
    } else {
        Some(sample_from(&vec![0u8; size.max(64 * 1024)], 0, 30.0)?)
    };
    Ok(MFT_OUTPUT_DATA_BUFFER {
        dwStreamID: 0,
        pSample: std::mem::ManuallyDrop::new(sample),
        dwStatus: 0,
        pEvents: std::mem::ManuallyDrop::new(None),
    })
}

unsafe fn sample_from(bytes: &[u8], stamp: i64, fps: f32) -> Result<IMFSample, String> {
    let buffer = MFCreateMemoryBuffer(bytes.len() as u32).map_err(|err| err.to_string())?;
    copy_in(&buffer, bytes)?;
    let sample = MFCreateSample().map_err(|err| err.to_string())?;
    sample.AddBuffer(&buffer).map_err(|err| err.to_string())?;
    sample.SetSampleTime(stamp).map_err(|err| err.to_string())?;
    let duration = (10_000_000.0 / fps.max(1.0)) as i64;
    sample
        .SetSampleDuration(duration)
        .map_err(|err| err.to_string())?;
    Ok(sample)
}

unsafe fn copy_in(buffer: &IMFMediaBuffer, bytes: &[u8]) -> Result<(), String> {
    let mut ptr = ptr::null_mut();
    buffer
        .Lock(&mut ptr, None, None)
        .map_err(|err| err.to_string())?;
    if !ptr.is_null() {
        ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
    }
    buffer
        .SetCurrentLength(bytes.len() as u32)
        .map_err(|err| err.to_string())?;
    buffer.Unlock().map_err(|err| err.to_string())?;
    Ok(())
}

unsafe fn read_sample(sample: Option<&IMFSample>) -> Result<Vec<u8>, String> {
    let Some(sample) = sample else {
        return Ok(Vec::new());
    };
    let buffer = sample
        .ConvertToContiguousBuffer()
        .map_err(|err| err.to_string())?;
    let mut ptr = ptr::null_mut();
    let mut current = 0u32;
    buffer
        .Lock(&mut ptr, None, Some(&mut current))
        .map_err(|err| err.to_string())?;
    let bytes = if ptr.is_null() {
        Vec::new()
    } else {
        std::slice::from_raw_parts(ptr, current as usize).to_vec()
    };
    buffer.Unlock().map_err(|err| err.to_string())?;
    Ok(bytes)
}

fn i420_to_nv12(i420: &[u8], width: u32, height: u32) -> Vec<u8> {
    let y = (width * height) as usize;
    let uv = y / 4;
    if i420.len() < y + uv * 2 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(y + uv * 2);
    out.extend_from_slice(&i420[..y]);
    let u = &i420[y..y + uv];
    let v = &i420[y + uv..y + uv * 2];
    for index in 0..uv {
        out.push(u[index]);
        out.push(v[index]);
    }
    out
}

/// Synchronous hardware H.264 decoder. Opened only when a hardware MFT
/// advertises an NV12 output type up front. A transform that needs the first
/// sample before it will name its output, or that rejects an access unit,
/// returns an error and the caller stays on OpenH264.
pub struct MfDecoder {
    transform: IMFTransform,
    provides: bool,
}

// Each decoder is owned by one media worker; COM pointers are not moved across threads while in use.
unsafe impl Send for MfDecoder {}

pub fn open_decoder() -> Result<MfDecoder, String> {
    unsafe { open_decoder_inner() }
}

impl MfDecoder {
    pub fn decode(&mut self, annexb: &[u8]) -> Result<Option<(u32, u32, Vec<u8>)>, String> {
        unsafe { self.decode_inner(annexb) }
    }
}

unsafe fn open_decoder_inner() -> Result<MfDecoder, String> {
    MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).map_err(|err| err.to_string())?;
    let activate = first_decoder()?;
    let transform: IMFTransform = activate.ActivateObject().map_err(|err| err.to_string())?;
    let input = MFCreateMediaType().map_err(|err| err.to_string())?;
    input
        .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
        .map_err(|err| err.to_string())?;
    input
        .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)
        .map_err(|err| err.to_string())?;
    transform
        .SetInputType(0, &input, 0)
        .map_err(|err| err.to_string())?;
    let output = transform
        .GetOutputAvailableType(0, 0)
        .map_err(|err| err.to_string())?;
    transform
        .SetOutputType(0, &output, 0)
        .map_err(|err| err.to_string())?;
    transform
        .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
        .map_err(|err| err.to_string())?;
    transform
        .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
        .map_err(|err| err.to_string())?;
    let info = transform
        .GetOutputStreamInfo(0)
        .map_err(|err| err.to_string())?;
    Ok(MfDecoder {
        transform,
        provides: info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0,
    })
}

unsafe fn first_decoder() -> Result<IMFActivate, String> {
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_NV12,
    };
    let flags = MFT_ENUM_FLAG(MFT_ENUM_FLAG_HARDWARE.0 | MFT_ENUM_FLAG_SYNCMFT.0);
    let mut activates: *mut Option<IMFActivate> = ptr::null_mut();
    let mut count = 0u32;
    MFTEnumEx(
        MFT_CATEGORY_VIDEO_DECODER,
        flags,
        Some(&input),
        Some(&output),
        &mut activates,
        &mut count,
    )
    .map_err(|err| err.to_string())?;
    if activates.is_null() || count == 0 {
        return Err("no synchronous hardware H.264 decoder".into());
    }
    let chosen = (*activates)
        .take()
        .ok_or_else(|| "hardware decoder did not activate".to_string())?;
    for index in 1..count {
        let _ = (*activates.add(index as usize)).take();
    }
    CoTaskMemFree(Some(activates.cast()));
    Ok(chosen)
}

impl MfDecoder {
    unsafe fn decode_inner(
        &mut self,
        annexb: &[u8],
    ) -> Result<Option<(u32, u32, Vec<u8>)>, String> {
        let avcc = annexb_to_avcc(annexb);
        if avcc.is_empty() {
            return Ok(None);
        }
        let sample = sample_from(&avcc, 0, 30.0)?;
        self.transform
            .ProcessInput(0, &sample, 0)
            .map_err(|err| err.to_string())?;
        let mut status = 0u32;
        let mut buffers = [output_buffer(self.provides, 8 * 1024 * 1024)?];
        let produced = self.transform.ProcessOutput(0, &mut buffers, &mut status);
        let nv12 = match produced {
            Ok(()) => read_sample(buffers[0].pSample.as_ref())?,
            Err(err) if err.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(None),
            Err(err) => return Err(err.to_string()),
        };
        let kind = self
            .transform
            .GetOutputCurrentType(0)
            .map_err(|err| err.to_string())?;
        let packed = kind
            .GetUINT64(&MF_MT_FRAME_SIZE)
            .map_err(|err| err.to_string())?;
        let width = (packed & 0xffff_ffff) as u32;
        let height = (packed >> 32) as u32;
        if width < 2 || height < 2 {
            return Err("hardware decoder did not report a picture size".into());
        }
        let rgba = nv12_to_rgba(&nv12, width, height);
        if rgba.is_empty() {
            return Err("hardware decoder produced a short picture".into());
        }
        Ok(Some((width, height, rgba)))
    }
}

fn annexb_to_avcc(annexb: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut offset = 0;
    while offset + 3 < annexb.len() {
        let header = if annexb[offset..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if annexb[offset..].starts_with(&[0, 0, 1]) {
            3
        } else {
            offset += 1;
            continue;
        };
        let start = offset + header;
        let mut end = start + 1;
        while end + 3 < annexb.len()
            && !annexb[end..].starts_with(&[0, 0, 1])
            && !annexb[end..].starts_with(&[0, 0, 0, 1])
        {
            end += 1;
        }
        if end + 3 >= annexb.len() {
            end = annexb.len();
        }
        let nal = &annexb[start..end];
        out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        out.extend_from_slice(nal);
        offset = end;
    }
    out
}

fn nv12_to_rgba(nv12: &[u8], width: u32, height: u32) -> Vec<u8> {
    let width = width as usize;
    let height = height as usize;
    let y_size = width * height;
    if nv12.len() < y_size + y_size / 2 {
        return Vec::new();
    }
    let mut rgba = vec![0u8; width * height * 4];
    for row in 0..height {
        for col in 0..width {
            let y = nv12[row * width + col] as i32;
            let uv = y_size + (row / 2) * width + (col & !1);
            let u = nv12[uv] as i32 - 128;
            let v = nv12.get(uv + 1).copied().unwrap_or(128) as i32 - 128;
            let c = y - 16;
            let r = (298 * c + 409 * v + 128) >> 8;
            let g = (298 * c - 100 * u - 208 * v + 128) >> 8;
            let b = (298 * c + 516 * u + 128) >> 8;
            let index = (row * width + col) * 4;
            rgba[index] = r.clamp(0, 255) as u8;
            rgba[index + 1] = g.clamp(0, 255) as u8;
            rgba[index + 2] = b.clamp(0, 255) as u8;
            rgba[index + 3] = 255;
        }
    }
    rgba
}
