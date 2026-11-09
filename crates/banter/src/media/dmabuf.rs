//! PipeWire DMA-BUF plane parsing and negotiation helpers (Linux Wayland).
//!
//! Milestone 2 of the capture plan: advertise DMA-BUF + MemFd, parse per-plane
//! descriptors and modifiers. M3 proves EGL import; M4 owns DMA-BUF FDs for
//! VA-API hardware-frame encode (`vaapi_dmabuf`) with MemFd/CPU fallback.
//!
//! Do **not** `mmap` a DMA-BUF as a linear pixel array unless the producer set
//! `SPA_DATA_FLAG_MAPPABLE` and the modifier is known-linear — tiled/compressed
//! modifiers are common on AMD/Intel.

#![cfg(target_os = "linux")]

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use pipewire::spa::buffer::{Data, DataType};
use pipewire::spa::param::video::VideoFormat;

/// `DRM_FORMAT_MOD_INVALID` — modifier-less / unknown.
pub const DRM_FORMAT_MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;
/// `DRM_FORMAT_MOD_LINEAR`.
pub const DRM_FORMAT_MOD_LINEAR: u64 = 0;

/// `SPA_DATA_FLAG_MAPPABLE` (bit 3) — not always exposed on older libspa bitflags.
pub const SPA_DATA_FLAG_MAPPABLE: u32 = 1 << 3;

/// `SPA_PARAM_BUFFERS_dataType` key (enum spa_param_buffers).
pub const PARAM_BUFFERS_DATA_TYPE: u32 = 6;

#[derive(Clone, Debug)]
pub struct DmaBufPlane {
    pub fd: i32,
    pub offset: u32,
    pub stride: i32,
    pub size: u32,
    pub flags: u32,
}

#[derive(Clone, Debug)]
pub struct DmaBufInfo {
    pub width: u32,
    pub height: u32,
    pub format: VideoFormat,
    pub modifier: u64,
    pub planes: Vec<DmaBufPlane>,
}

/// DMA-BUF planes with `dup`'d FDs so PipeWire can requeue the source buffer.
pub struct OwnedDmaBufFrame {
    pub width: u32,
    pub height: u32,
    pub format: VideoFormat,
    pub modifier: u64,
    pub planes: Vec<OwnedDmaBufPlane>,
}

pub struct OwnedDmaBufPlane {
    pub fd: OwnedFd,
    pub offset: u32,
    pub stride: i32,
    pub size: u32,
    pub flags: u32,
}

impl OwnedDmaBufFrame {
    /// Duplicate plane FDs from a PipeWire-owned [`DmaBufInfo`].
    pub fn dup_from(info: &DmaBufInfo) -> Result<Self, String> {
        if info.planes.is_empty() {
            return Err("DMA-BUF has no planes".into());
        }
        let mut planes = Vec::with_capacity(info.planes.len());
        for p in &info.planes {
            if p.fd < 0 {
                return Err("DMA-BUF plane fd is invalid".into());
            }
            let dup = unsafe { libc::dup(p.fd) };
            if dup < 0 {
                return Err("dup(DMA-BUF fd) failed".into());
            }
            planes.push(OwnedDmaBufPlane {
                fd: unsafe { OwnedFd::from_raw_fd(dup) },
                offset: p.offset,
                stride: p.stride,
                size: p.size,
                flags: p.flags,
            });
        }
        Ok(Self {
            width: info.width,
            height: info.height,
            format: info.format,
            modifier: info.modifier,
            planes,
        })
    }

    pub fn plane_raw_fds(&self) -> Vec<i32> {
        self.planes.iter().map(|p| p.fd.as_raw_fd()).collect()
    }
}

impl DmaBufInfo {
    #[allow(dead_code)]
    pub fn all_mappable(&self) -> bool {
        !self.planes.is_empty()
            && self
                .planes
                .iter()
                .all(|p| p.flags & SPA_DATA_FLAG_MAPPABLE != 0)
    }

    pub fn modifier_is_linearish(&self) -> bool {
        self.modifier == DRM_FORMAT_MOD_LINEAR || self.modifier == DRM_FORMAT_MOD_INVALID
    }

    pub fn summary(&self) -> String {
        format!(
            "{}x{} {:?} mod={:#x} planes={} mappable={}",
            self.width,
            self.height,
            self.format,
            self.modifier,
            self.planes.len(),
            self.all_mappable()
        )
    }
}

/// Collect DMA-BUF planes from a dequeued PipeWire buffer's `spa_data` array.
///
/// The FDs are owned by PipeWire for the buffer's lifetime — callers must `dup`
/// before the buffer is requeued if they need to hold them (EGL import).
pub fn parse_planes(datas: &[Data]) -> Vec<DmaBufPlane> {
    let mut planes = Vec::new();
    for data in datas {
        if data.type_() != DataType::DmaBuf {
            continue;
        }
        let raw = data.as_raw();
        let chunk = data.chunk();
        let fd = data.fd();
        if fd < 0 {
            continue;
        }
        planes.push(DmaBufPlane {
            fd,
            offset: chunk.offset(),
            stride: chunk.stride(),
            size: if chunk.size() > 0 {
                chunk.size()
            } else {
                raw.maxsize
            },
            flags: raw.flags,
        });
    }
    planes
}

pub fn data_type_mask_dmabuf() -> i32 {
    1 << DataType::DmaBuf.as_raw()
}

pub fn data_type_mask_mem() -> i32 {
    (1 << DataType::MemFd.as_raw()) | (1 << DataType::MemPtr.as_raw())
}

pub fn data_type_mask_all() -> i32 {
    data_type_mask_dmabuf() | data_type_mask_mem()
}

/// `BANTER_DMABUF=0` disables advertising DMA-BUF EnumFormats (MemFd-only).
pub fn advertise_dmabuf() -> bool {
    match std::env::var("BANTER_DMABUF") {
        Ok(v) if v == "0" || v.eq_ignore_ascii_case("off") || v.eq_ignore_ascii_case("false") => {
            false
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_cover_expected_bits() {
        assert_eq!(data_type_mask_dmabuf() & data_type_mask_mem(), 0);
        assert_eq!(
            data_type_mask_all(),
            data_type_mask_dmabuf() | data_type_mask_mem()
        );
    }

    #[test]
    fn linearish_modifiers() {
        let info = DmaBufInfo {
            width: 2,
            height: 2,
            format: VideoFormat::BGRA,
            modifier: DRM_FORMAT_MOD_LINEAR,
            planes: vec![],
        };
        assert!(info.modifier_is_linearish());
    }
}
