//! Speaker loopback and per-app loopback.
//!
//! Render devices are captured with WASAPI loopback. Application rows use the
//! process-loopback activation so only that process tree is mixed in. Both
//! paths write 48 kHz mono i16 into the shared source map. Nothing here
//! invents samples when a device or process cannot be opened.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::core::{implement, IUnknown, Interface, HRESULT, PROPVARIANT};
use windows::Win32::Media::Audio::{
    eConsole, eRender, ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    DEVICE_STATE_ACTIVE, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
    VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemAlloc, CoTaskMemFree, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};

use crate::logic::resample_mono_i16;
use crate::settings::process_loopback_format;

pub struct Loop {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Loop {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn render_devices() -> Vec<String> {
    unsafe { render_devices_inner().unwrap_or_default() }
}

pub fn start_render(
    id: String,
    name: String,
    pcm: Arc<Mutex<std::collections::HashMap<String, VecDeque<i16>>>>,
) -> Result<Loop, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let thread = thread::Builder::new()
        .name("banter-loopback".into())
        .spawn(move || {
            if let Err(err) = unsafe { pump_render(&name, &id, &pcm, &stop_thread, ready_tx) } {
                let _ = err;
            }
        })
        .map_err(|err| err.to_string())?;
    match ready_rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(())) => Ok(Loop {
            stop,
            thread: Some(thread),
        }),
        Ok(Err(err)) => {
            stop.store(true, Ordering::Relaxed);
            let _ = thread.join();
            Err(err)
        }
        Err(_) => {
            stop.store(true, Ordering::Relaxed);
            let _ = thread.join();
            Err("speaker loopback timed out".into())
        }
    }
}

pub fn start_process(
    id: String,
    pid: u32,
    pcm: Arc<Mutex<std::collections::HashMap<String, VecDeque<i16>>>>,
) -> Result<Loop, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let thread = thread::Builder::new()
        .name("banter-app-audio".into())
        .spawn(move || {
            if let Err(err) = unsafe { pump_process(pid, &id, &pcm, &stop_thread, ready_tx) } {
                let _ = err;
            }
        })
        .map_err(|err| err.to_string())?;
    match ready_rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(())) => Ok(Loop {
            stop,
            thread: Some(thread),
        }),
        Ok(Err(err)) => {
            stop.store(true, Ordering::Relaxed);
            let _ = thread.join();
            Err(err)
        }
        Err(_) => {
            stop.store(true, Ordering::Relaxed);
            let _ = thread.join();
            Err("process loopback timed out".into())
        }
    }
}

unsafe fn com_mta() {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
}

unsafe fn render_devices_inner() -> Result<Vec<String>, String> {
    com_mta();
    let enumerator: IMMDeviceEnumerator =
        CoCreateInstance(&MMDeviceEnumerator, None::<&IUnknown>, CLSCTX_ALL)
            .map_err(|err| err.to_string())?;
    let collection = enumerator
        .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
        .map_err(|err| err.to_string())?;
    let count = collection.GetCount().map_err(|err| err.to_string())?;
    let mut names = Vec::new();
    for index in 0..count {
        let device = collection.Item(index).map_err(|err| err.to_string())?;
        let store = device
            .OpenPropertyStore(STGM_READ)
            .map_err(|err| err.to_string())?;
        let key = windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
        let value = store.GetValue(&key).map_err(|err| err.to_string())?;
        let name = property_string(&value);
        if !name.is_empty() && !names.iter().any(|existing: &String| existing == &name) {
            names.push(name);
        }
    }
    Ok(names)
}

fn property_string(value: &PROPVARIANT) -> String {
    let shown = value.to_string();
    if !shown.is_empty() {
        return shown;
    }
    unsafe {
        let raw = value.as_raw();
        if raw.Anonymous.Anonymous.vt == 31 {
            let ptr = raw.Anonymous.Anonymous.Anonymous.pwszVal;
            if !ptr.is_null() {
                let mut len = 0usize;
                while *ptr.add(len) != 0 {
                    len += 1;
                    if len > 512 {
                        break;
                    }
                }
                return String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
            }
        }
    }
    String::new()
}

unsafe fn pump_render(
    name: &str,
    id: &str,
    pcm: &Mutex<std::collections::HashMap<String, VecDeque<i16>>>,
    stop: &AtomicBool,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    com_mta();
    let enumerator: IMMDeviceEnumerator =
        CoCreateInstance(&MMDeviceEnumerator, None::<&IUnknown>, CLSCTX_ALL).map_err(|err| {
            let message = err.to_string();
            let _ = ready.send(Err(message.clone()));
            message
        })?;
    let device = match find_render(&enumerator, name) {
        Ok(device) => device,
        Err(err) => {
            let _ = ready.send(Err(err.clone()));
            return Err(err);
        }
    };
    let client: IAudioClient = match device.Activate(CLSCTX_ALL, None) {
        Ok(client) => client,
        Err(err) => {
            let message = err.to_string();
            let _ = ready.send(Err(message.clone()));
            return Err(message);
        }
    };
    let format = match client.GetMixFormat() {
        Ok(format) => format,
        Err(err) => {
            let message = err.to_string();
            let _ = ready.send(Err(message.clone()));
            return Err(message);
        }
    };
    if let Err(err) = client.Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_LOOPBACK,
        0,
        0,
        format,
        None,
    ) {
        CoTaskMemFree(Some(format as *const std::ffi::c_void));
        let message = err.to_string();
        let _ = ready.send(Err(message.clone()));
        return Err(message);
    }
    let capture: IAudioCaptureClient = match client.GetService() {
        Ok(capture) => capture,
        Err(err) => {
            CoTaskMemFree(Some(format as *const std::ffi::c_void));
            let message = err.to_string();
            let _ = ready.send(Err(message.clone()));
            return Err(message);
        }
    };
    if let Err(err) = client.Start() {
        CoTaskMemFree(Some(format as *const std::ffi::c_void));
        let message = err.to_string();
        let _ = ready.send(Err(message.clone()));
        return Err(message);
    }
    let _ = ready.send(Ok(()));
    let result = pump_capture(&client, &capture, format, id, pcm, stop);
    let _ = client.Stop();
    CoTaskMemFree(Some(format as *const std::ffi::c_void));
    result
}

unsafe fn find_render(
    enumerator: &IMMDeviceEnumerator,
    name: &str,
) -> Result<windows::Win32::Media::Audio::IMMDevice, String> {
    if name.is_empty() {
        return enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|err| err.to_string());
    }
    let collection = enumerator
        .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
        .map_err(|err| err.to_string())?;
    let count = collection.GetCount().map_err(|err| err.to_string())?;
    for index in 0..count {
        let device = collection.Item(index).map_err(|err| err.to_string())?;
        let store = device
            .OpenPropertyStore(STGM_READ)
            .map_err(|err| err.to_string())?;
        let key = windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
        let value = store.GetValue(&key).map_err(|err| err.to_string())?;
        if property_string(&value) == name {
            return Ok(device);
        }
    }
    Err(format!("speakers {name} are not available"))
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct ProcessHandler(Arc<ProcessSlot>);

struct ProcessSlot {
    client: Mutex<Option<IAudioClient>>,
    error: Mutex<Option<String>>,
    done: AtomicBool,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for ProcessHandler {
    fn ActivateCompleted(
        &self,
        activateoperation: Option<&IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        let Some(operation) = activateoperation else {
            *self.0.error.lock().expect("loopback error") =
                Some("process loopback returned nothing".into());
            self.0.done.store(true, Ordering::SeqCst);
            return Ok(());
        };
        unsafe {
            let mut code = HRESULT(0);
            let mut unknown: Option<IUnknown> = None;
            operation.GetActivateResult(&mut code, &mut unknown)?;
            if code.is_err() {
                *self.0.error.lock().expect("loopback error") =
                    Some(format!("process loopback failed ({code:?})"));
            } else if let Some(unknown) = unknown {
                match unknown.cast::<IAudioClient>() {
                    Ok(client) => *self.0.client.lock().expect("loopback client") = Some(client),
                    Err(err) => {
                        *self.0.error.lock().expect("loopback error") = Some(err.to_string())
                    }
                }
            }
        }
        self.0.done.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn process_loopback_wave_format() -> WAVEFORMATEX {
    let spec = process_loopback_format();
    WAVEFORMATEX {
        wFormatTag: spec.format_tag,
        nChannels: spec.channels,
        nSamplesPerSec: spec.sample_rate,
        nAvgBytesPerSec: spec.avg_bytes_per_sec,
        nBlockAlign: spec.block_align,
        wBitsPerSample: spec.bits_per_sample,
        cbSize: 0,
    }
}

unsafe fn pump_process(
    pid: u32,
    id: &str,
    pcm: &Mutex<std::collections::HashMap<String, VecDeque<i16>>>,
    stop: &AtomicBool,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    com_mta();
    let fail = |ready: &std::sync::mpsc::Sender<Result<(), String>>,
                message: String|
     -> Result<(), String> {
        let _ = ready.send(Err(message.clone()));
        Err(message)
    };
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS::default();
    params.ActivationType = AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK;
    params.Anonymous.ProcessLoopbackParams.TargetProcessId = pid;
    params.Anonymous.ProcessLoopbackParams.ProcessLoopbackMode =
        PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE;
    let bytes = std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>();
    let memory = CoTaskMemAlloc(bytes);
    if memory.is_null() {
        return fail(
            &ready,
            "could not allocate process loopback parameters".into(),
        );
    }
    std::ptr::copy_nonoverlapping(&params as *const _ as *const u8, memory as *mut u8, bytes);
    let raw = windows::core::imp::PROPVARIANT {
        Anonymous: windows::core::imp::PROPVARIANT_0 {
            Anonymous: windows::core::imp::PROPVARIANT_0_0 {
                vt: 65,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: windows::core::imp::PROPVARIANT_0_0_0 {
                    blob: windows::core::imp::BLOB {
                        cbSize: bytes as u32,
                        pBlobData: memory as *mut u8,
                    },
                },
            },
        },
    };
    let activation = PROPVARIANT::from_raw(raw);
    let slot = Arc::new(ProcessSlot {
        client: Mutex::new(None),
        error: Mutex::new(None),
        done: AtomicBool::new(false),
    });
    let handler: IActivateAudioInterfaceCompletionHandler =
        ProcessHandler(Arc::clone(&slot)).into();
    if let Err(err) = ActivateAudioInterfaceAsync(
        VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
        &IAudioClient::IID,
        Some(&activation),
        &handler,
    ) {
        return fail(&ready, err.to_string());
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !slot.done.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    if let Some(err) = slot.error.lock().expect("loopback error").clone() {
        return fail(&ready, err);
    }
    let client = match slot.client.lock().expect("loopback client").take() {
        Some(client) => client,
        None => return fail(&ready, "process loopback did not start".into()),
    };
    // GetMixFormat is unsupported on VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK and returns E_INVALIDARG
    // (0x80070057). Use the Microsoft ApplicationLoopback PCM format with LOOPBACK + AUTOCONVERTPCM.
    let format = process_loopback_wave_format();
    let flags = AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM;
    if let Err(err) = client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 0, 0, &format, None) {
        return fail(&ready, err.to_string());
    }
    let capture: IAudioCaptureClient = match client.GetService() {
        Ok(capture) => capture,
        Err(err) => return fail(&ready, err.to_string()),
    };
    if let Err(err) = client.Start() {
        return fail(&ready, err.to_string());
    }
    let _ = ready.send(Ok(()));
    let result = pump_capture(&client, &capture, &format, id, pcm, stop);
    let _ = client.Stop();
    result
}

unsafe fn pump_capture(
    _client: &IAudioClient,
    capture: &IAudioCaptureClient,
    format: *const WAVEFORMATEX,
    id: &str,
    pcm: &Mutex<std::collections::HashMap<String, VecDeque<i16>>>,
    stop: &AtomicBool,
) -> Result<(), String> {
    let info = format_info(format);
    while !stop.load(Ordering::Relaxed) {
        let packet = capture.GetNextPacketSize().unwrap_or(0);
        if packet == 0 {
            thread::sleep(Duration::from_millis(5));
            continue;
        }
        let mut data = std::ptr::null_mut();
        let mut frames = 0u32;
        let mut flags = 0u32;
        if capture
            .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
            .is_err()
        {
            thread::sleep(Duration::from_millis(5));
            continue;
        }
        if frames > 0 && !data.is_null() {
            let mono = mono_from_packet(data, frames, info);
            let samples = resample_mono_i16(&mono, info.rate, 48_000);
            let mut map = pcm.lock().expect("source audio");
            let queue = map.entry(id.to_string()).or_default();
            queue.extend(samples);
            while queue.len() > super::pcm_backlog_samples() {
                queue.pop_front();
            }
        }
        let _ = capture.ReleaseBuffer(frames);
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct FormatInfo {
    channels: u16,
    bits: u16,
    rate: u32,
    float: bool,
}

unsafe fn format_info(format: *const WAVEFORMATEX) -> FormatInfo {
    let header = std::ptr::read_unaligned(format);
    let mut info = FormatInfo {
        channels: header.nChannels.max(1),
        bits: header.wBitsPerSample,
        rate: header.nSamplesPerSec.max(1),
        float: header.wFormatTag == 3,
    };
    if header.wFormatTag == 0xFFFE {
        let extended = std::ptr::read_unaligned(format as *const WAVEFORMATEXTENSIBLE);
        info.float = extended.SubFormat.data1 == 3;
        info.bits = extended.Format.wBitsPerSample;
        info.channels = extended.Format.nChannels.max(1);
        info.rate = extended.Format.nSamplesPerSec.max(1);
    }
    info
}

unsafe fn mono_from_packet(data: *const u8, frames: u32, info: FormatInfo) -> Vec<i16> {
    let channels = info.channels.max(1) as usize;
    let mut mono = Vec::with_capacity(frames as usize);
    if info.float || info.bits == 32 {
        let samples = std::slice::from_raw_parts(data as *const f32, frames as usize * channels);
        for frame in samples.chunks(channels) {
            let sum: f32 = frame.iter().copied().sum();
            let value = (sum / channels as f32).clamp(-1.0, 1.0);
            mono.push((value * i16::MAX as f32) as i16);
        }
    } else {
        let samples = std::slice::from_raw_parts(data as *const i16, frames as usize * channels);
        for frame in samples.chunks(channels) {
            let sum: i32 = frame.iter().map(|sample| *sample as i32).sum();
            mono.push((sum / channels as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16);
        }
    }
    mono
}
