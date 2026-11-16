//! Raw HID access through `hid.dll`, `cfgmgr32.dll` and overlapped file I/O.
//!
//! Enumeration only reads descriptors Windows has cached; nothing is sent to a device
//! until a driver explicitly writes to it.

use std::collections::HashMap;
use std::ffi::c_void;
use std::io;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW,
    CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CR_SUCCESS,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetFeature, HidD_GetHidGuid,
    HidD_GetPreparsedData, HidD_GetProductString, HidD_SetFeature, HidP_GetCaps, HIDD_ATTRIBUTES,
    HIDP_CAPS, HIDP_STATUS_SUCCESS, PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_OBJECT_0,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_OVERLAPPED,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Threading::{
    CreateEventW, ResetEvent, WaitForMultipleObjects, WaitForSingleObject,
};
use windows::Win32::System::IO::{CancelIo, GetOverlappedResult, OVERLAPPED};

use crate::hid::{interface_from_path, is_bluetooth_path, vid_pid_from_path, HidInfo};

const WRITE_TIMEOUT_MS: u32 = 1000;
/// `WaitForMultipleObjects` cannot wait on more handles than this.
const MAX_WAIT_HANDLES: usize = 64;

fn millis(duration: Duration) -> u32 {
    duration.as_millis().min(u128::from(u32::MAX - 1)) as u32
}

/// Owned kernel handle.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by CreateFileW/CreateEventW and is closed once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn open_handle(path: &str, access: u32, overlapped: bool) -> io::Result<Handle> {
    let flags = if overlapped {
        FILE_FLAG_OVERLAPPED
    } else {
        FILE_FLAGS_AND_ATTRIBUTES(0)
    };
    // SAFETY: `HSTRING` keeps the path alive and NUL-terminated for the duration of the call.
    let handle = unsafe {
        CreateFileW(
            &HSTRING::from(path),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            flags,
            None,
        )
    };
    handle
        .map(Handle)
        .map_err(|e| io::Error::other(format!("open {path}: {e}")))
}

fn new_event() -> io::Result<Handle> {
    // SAFETY: plain call; manual-reset, initially non-signalled, unnamed.
    unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
        .map(Handle)
        .map_err(|e| io::Error::other(format!("CreateEvent: {e}")))
}

/// Device interface paths of every present HID top-level collection.
pub fn interface_paths() -> Vec<String> {
    // SAFETY: the buffer handed to cfgmgr32 is exactly as long as it asked for.
    unsafe {
        let guid = HidD_GetHidGuid();
        // The list can grow between the two calls; retry a few times.
        for _ in 0..3 {
            let mut size = 0u32;
            let result = CM_Get_Device_Interface_List_SizeW(
                &mut size,
                &guid,
                PCWSTR::null(),
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            );
            if result != CR_SUCCESS || size <= 1 {
                return Vec::new();
            }
            let mut buffer = vec![0u16; size as usize];
            let result = CM_Get_Device_Interface_ListW(
                &guid,
                PCWSTR::null(),
                &mut buffer,
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            );
            if result == CR_SUCCESS {
                return buffer
                    .split(|c| *c == 0)
                    .filter(|part| !part.is_empty())
                    .map(String::from_utf16_lossy)
                    .collect();
            }
        }
        Vec::new()
    }
}

/// Read attributes and report lengths of one collection without sending anything to it.
fn describe(path: &str) -> Option<HidInfo> {
    let (mut vid, mut pid) = vid_pid_from_path(path)?;
    let handle = open_handle(path, 0, false).ok()?;
    // SAFETY: all out-pointers reference live, correctly sized locals; the preparsed
    // data is freed before returning.
    unsafe {
        let mut attributes = HIDD_ATTRIBUTES {
            Size: size_of::<HIDD_ATTRIBUTES>() as u32,
            ..Default::default()
        };
        if HidD_GetAttributes(handle.0, &mut attributes) {
            (vid, pid) = (attributes.VendorID, attributes.ProductID);
        }
        let mut preparsed = PHIDP_PREPARSED_DATA::default();
        if !HidD_GetPreparsedData(handle.0, &mut preparsed) {
            return None;
        }
        let mut caps = HIDP_CAPS::default();
        let status = HidP_GetCaps(preparsed, &mut caps);
        let _ = HidD_FreePreparsedData(preparsed);
        if status != HIDP_STATUS_SUCCESS {
            return None;
        }
        Some(HidInfo {
            path: path.to_string(),
            vid,
            pid,
            interface: interface_from_path(path),
            usage_page: caps.UsagePage,
            usage: caps.Usage,
            input_len: usize::from(caps.InputReportByteLength),
            output_len: usize::from(caps.OutputReportByteLength),
            feature_len: usize::from(caps.FeatureReportByteLength),
            bluetooth: is_bluetooth_path(path),
        })
    }
}

/// All HID collections whose vendor id passes `wanted`.
pub fn enumerate(wanted: impl Fn(u16) -> bool) -> Vec<HidInfo> {
    interface_paths()
        .iter()
        .filter(|path| vid_pid_from_path(path).is_some_and(|(vid, _)| wanted(vid)))
        .filter_map(|path| describe(path))
        .filter(|info| wanted(info.vid))
        .collect()
}

/// USB product string, asked once per path. The query goes to the device itself and a
/// stuck firmware takes seconds to time out, so even an empty answer is cached.
pub fn product_string(path: &str) -> String {
    static CACHE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
    let cached = |path: &str| {
        let guard = CACHE.lock().unwrap_or_else(|p| p.into_inner());
        guard.as_ref().and_then(|map| map.get(path).cloned())
    };
    if let Some(name) = cached(path) {
        return name;
    }
    let mut name = String::new();
    if let Ok(handle) = open_handle(path, 0, false) {
        let mut buffer = [0u16; 128];
        // SAFETY: the length passed is the buffer size in bytes.
        let ok = unsafe {
            HidD_GetProductString(
                handle.0,
                buffer.as_mut_ptr().cast::<c_void>(),
                size_of_val(&buffer) as u32,
            )
        };
        if ok {
            let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
            name = String::from_utf16_lossy(&buffer[..end]).trim().to_string();
        }
    }
    let mut guard = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    guard
        .get_or_insert_with(HashMap::new)
        .insert(path.to_string(), name.clone());
    name
}

/// An opened HID collection with time-limited reads and writes.
///
/// A read is kept pending between calls ("armed") so that a reply arriving right after a
/// write is never lost.
pub struct HidDevice {
    pub info: HidInfo,
    handle: Handle,
    read_event: Handle,
    write_event: Handle,
    // Boxed: the kernel keeps pointers to both while a read is pending.
    read_overlapped: Box<OVERLAPPED>,
    read_buffer: Box<[u8]>,
    pending: bool,
}

// SAFETY: the raw pointers inside OVERLAPPED refer to memory owned by this struct, and
// the device is only ever used from one thread at a time (`&mut self`).
unsafe impl Send for HidDevice {}

impl HidDevice {
    /// Open for reading and writing reports.
    pub fn open(info: &HidInfo) -> io::Result<HidDevice> {
        let handle = open_handle(&info.path, (GENERIC_READ | GENERIC_WRITE).0, true)?;
        Self::with_handle(info, handle)
    }

    /// Open for feature reports only. Windows keeps keyboards and mice to itself, but
    /// feature reports still work on a handle opened without any access rights.
    pub fn open_for_features(info: &HidInfo) -> io::Result<HidDevice> {
        let handle = open_handle(&info.path, (GENERIC_READ | GENERIC_WRITE).0, true)
            .or_else(|_| open_handle(&info.path, 0, false))?;
        Self::with_handle(info, handle)
    }

    fn with_handle(info: &HidInfo, handle: Handle) -> io::Result<HidDevice> {
        let read_event = new_event()?;
        let read_overlapped = Box::new(OVERLAPPED {
            hEvent: read_event.0,
            ..Default::default()
        });
        Ok(HidDevice {
            info: info.clone(),
            handle,
            read_event,
            write_event: new_event()?,
            read_overlapped,
            read_buffer: vec![0u8; info.input_len.max(2) + 64].into_boxed_slice(),
            pending: false,
        })
    }

    /// Event that becomes signalled when the pending read completes.
    pub fn read_event(&self) -> HANDLE {
        self.read_event.0
    }

    /// Start a read unless one is already pending.
    pub fn arm(&mut self) -> io::Result<()> {
        if self.pending || self.info.input_len == 0 {
            return Ok(());
        }
        // SAFETY: buffer and OVERLAPPED are heap allocations owned by `self`; `Drop`
        // cancels the read and waits for it before they are freed.
        unsafe {
            let _ = ResetEvent(self.read_event.0);
            let length = self.info.input_len.min(self.read_buffer.len());
            let started = ReadFile(
                self.handle.0,
                Some(&mut self.read_buffer[..length]),
                None,
                Some(&mut *self.read_overlapped),
            );
            if let Err(error) = started {
                if error.code() != ERROR_IO_PENDING.to_hresult() {
                    return Err(io::Error::other(format!("ReadFile: {error}")));
                }
            }
        }
        self.pending = true;
        Ok(())
    }

    /// Fetch the report of a completed read (raw, including the report id byte).
    pub fn collect(&mut self) -> io::Result<Vec<u8>> {
        let mut transferred = 0u32;
        self.pending = false;
        // SAFETY: same handle and OVERLAPPED the read was started with.
        unsafe {
            GetOverlappedResult(
                self.handle.0,
                &*self.read_overlapped,
                &mut transferred,
                false,
            )
        }
        .map_err(|e| io::Error::other(format!("read failed: {e}")))?;
        Ok(self.read_buffer[..(transferred as usize).min(self.read_buffer.len())].to_vec())
    }

    /// Next input report, or `None` if nothing arrives in time.
    pub fn read(&mut self, timeout: Duration) -> io::Result<Option<Vec<u8>>> {
        if self.info.input_len == 0 {
            return Ok(None);
        }
        self.arm()?;
        // SAFETY: waiting on an event handle owned by `self`.
        if unsafe { WaitForSingleObject(self.read_event.0, millis(timeout)) } != WAIT_OBJECT_0 {
            return Ok(None);
        }
        self.collect().map(Some)
    }

    /// Send an output report. `data[0]` is the report id (0 if the device has none); the
    /// report is zero-padded to the length the collection declares.
    pub fn write(&mut self, data: &[u8]) -> io::Result<()> {
        let size = self.info.output_len;
        if size == 0 || data.len() > size {
            return Err(io::Error::other("collection has no matching output report"));
        }
        let mut report = vec![0u8; size];
        report[..data.len()].copy_from_slice(data);
        let mut overlapped = OVERLAPPED {
            hEvent: self.write_event.0,
            ..Default::default()
        };
        let mut transferred = 0u32;
        // SAFETY: `report` and `overlapped` outlive the operation: on every path we wait
        // for completion (or cancel and then wait) before returning.
        unsafe {
            let _ = ResetEvent(self.write_event.0);
            if let Err(error) = WriteFile(self.handle.0, Some(&report), None, Some(&mut overlapped))
            {
                if error.code() != ERROR_IO_PENDING.to_hresult() {
                    return Err(io::Error::other(format!("WriteFile: {error}")));
                }
            }
            if WaitForSingleObject(self.write_event.0, WRITE_TIMEOUT_MS) != WAIT_OBJECT_0 {
                let _ = CancelIo(self.handle.0);
                let _ = GetOverlappedResult(self.handle.0, &overlapped, &mut transferred, true);
                return Err(io::Error::other("WriteFile timed out"));
            }
            GetOverlappedResult(self.handle.0, &overlapped, &mut transferred, false)
                .map_err(|e| io::Error::other(format!("write failed: {e}")))
        }
    }

    /// Send a feature report (`data[0]` is the report id), zero-padded to the declared length.
    pub fn set_feature(&self, data: &[u8]) -> bool {
        let size = self.info.feature_len;
        if size == 0 || data.len() > size {
            return false;
        }
        let mut report = vec![0u8; size];
        report[..data.len()].copy_from_slice(data);
        // SAFETY: pointer and length describe `report`.
        unsafe { HidD_SetFeature(self.handle.0, report.as_ptr().cast::<c_void>(), size as u32) }
    }

    /// Read a feature report; the returned buffer starts with the report id.
    pub fn get_feature(&self, report_id: u8) -> Option<Vec<u8>> {
        let size = self.info.feature_len;
        if size == 0 {
            return None;
        }
        let mut report = vec![0u8; size];
        report[0] = report_id;
        // SAFETY: pointer and length describe `report`.
        let ok = unsafe {
            HidD_GetFeature(
                self.handle.0,
                report.as_mut_ptr().cast::<c_void>(),
                size as u32,
            )
        };
        ok.then_some(report)
    }
}

impl Drop for HidDevice {
    fn drop(&mut self) {
        if self.pending {
            let mut transferred = 0u32;
            // SAFETY: cancel the pending read and wait until the kernel no longer touches
            // the buffer and OVERLAPPED, both of which are freed right after this.
            unsafe {
                let _ = CancelIo(self.handle.0);
                let _ = GetOverlappedResult(
                    self.handle.0,
                    &*self.read_overlapped,
                    &mut transferred,
                    true,
                );
            }
        }
    }
}

/// Wait for the first input report on any of the devices. Returns the index of the
/// device and its report, `Ok(None)` on timeout and `Err(index)` if a device failed.
pub fn wait_any(
    devices: &mut [&mut HidDevice],
    timeout: Duration,
) -> Result<Option<(usize, Vec<u8>)>, usize> {
    let mut owners = Vec::new();
    let mut events = Vec::new();
    for (index, device) in devices.iter_mut().enumerate() {
        if device.info.input_len == 0 || events.len() == MAX_WAIT_HANDLES {
            continue;
        }
        device.arm().map_err(|_| index)?;
        owners.push(index);
        events.push(device.read_event());
    }
    if events.is_empty() {
        std::thread::sleep(timeout);
        return Ok(None);
    }
    // SAFETY: every handle is an event owned by one of `devices`, which outlive the call.
    let result = unsafe { WaitForMultipleObjects(&events, false, millis(timeout)) };
    let slot = result.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
    let Some(index) = owners.get(slot).copied() else {
        return Ok(None);
    };
    devices[index]
        .collect()
        .map(|report| Some((index, report)))
        .map_err(|_| index)
}

/// How a query is delivered to the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Output,
    Feature,
    /// Output report, falling back to a feature report.
    Either,
}

/// A device opened for a question-and-answer exchange.
///
/// Windows delivers a reply through whichever collection declares that report, which need
/// not be the one the request was written to. A session therefore writes to one collection
/// and listens on all vendor-defined collections of the same device.
pub struct Session {
    devices: Vec<HidDevice>,
    writer: usize,
}

impl Session {
    pub fn open(group: &[HidInfo], writer: &HidInfo) -> Option<Session> {
        let mut devices = Vec::new();
        let mut writer_index = None;
        for info in group {
            let is_writer = info.path == writer.path;
            if !is_writer && (info.input_len == 0 || info.usage_page < 0xFF00) {
                continue;
            }
            if let Ok(device) = HidDevice::open(info) {
                if is_writer {
                    writer_index = Some(devices.len());
                }
                devices.push(device);
            }
        }
        writer_index.map(|writer| Session { devices, writer })
    }

    /// Send `packet` and return the first report accepted by `matches`. Reports are passed
    /// without the leading zero of devices that do not use report ids.
    pub fn ask(
        &mut self,
        packet: &[u8],
        via: Via,
        timeout: Duration,
        matches: impl Fn(&[u8]) -> bool,
    ) -> Option<Vec<u8>> {
        for device in &mut self.devices {
            device.arm().ok()?;
        }
        let writer = &mut self.devices[self.writer];
        let sent = match via {
            Via::Output => writer.write(packet).is_ok(),
            Via::Feature => writer.set_feature(packet),
            Via::Either => writer.write(packet).is_ok() || writer.set_feature(packet),
        };
        if !sent {
            return None;
        }
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.checked_duration_since(Instant::now())?;
            let mut all: Vec<&mut HidDevice> = self.devices.iter_mut().collect();
            let (_, report) = wait_any(&mut all, remaining).ok()??;
            let report = crate::hid::strip_report_id_zero(&report);
            if matches(report) {
                return Some(report.to_vec());
            }
        }
    }
}
