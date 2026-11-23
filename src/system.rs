//! Full system status: CPU, memory, GPU, network throughput, uptime and every mounted disk —
//! read straight from Win32 (no extra crates, no drivers, no admin rights).
//! GPU basics come from D3DKMT (any vendor); NVIDIA's NVML adds power and fan speed, but it
//! costs ~20 MB of RAM, so it is loaded only while the full view is open.

use std::time::Instant;

use windows::{
    core::{s, PCWSTR},
    Win32::{
        Foundation::{CloseHandle, FreeLibrary, FILETIME, HMODULE},
        NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2},
        Storage::FileSystem::*,
        System::{
            Ioctl::*,
            LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32},
            Power::{CallNtPowerInformation, ProcessorInformation},
            Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ},
            SystemInformation::{GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX},
            Threading::{GetActiveProcessorCount, GetSystemTimes},
            IO::DeviceIoControl,
        },
    },
};

use crate::wide;

/// Disks change rarely; enumerating volumes every poll would be wasteful.
const DISK_REFRESH_SECS: u64 = 10;

#[derive(Clone, Debug)]
pub struct DiskInfo {
    /// "C:"
    pub mount: String,
    /// Volume label, may be empty.
    pub label: String,
    /// "SSD", "HDD" or "".
    pub kind: &'static str,
    pub removable: bool,
    pub total: u64,
    pub available: u64,
}

#[derive(Clone, Debug)]
pub struct GpuInfo {
    pub name: String,
    /// percent
    pub load: u32,
    pub mem_used: u64,
    pub mem_total: u64,
    /// °C
    pub temp: Option<u32>,
    /// watts
    pub power: Option<f64>,
    /// percent
    pub fan: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub cpu: f32,
    pub cpu_name: String,
    pub threads: u32,
    /// MHz
    pub cpu_freq: u64,
    pub ram_used: u64,
    pub ram_total: u64,
    /// bytes per second since the previous poll
    pub net_down: f64,
    pub net_up: f64,
    pub uptime: u64,
    pub disks: Vec<DiskInfo>,
    pub gpus: Vec<GpuInfo>,
}

pub struct Monitor {
    cpu_prev: (u64, u64),
    cpu_name: String,
    net_prev: (u64, u64),
    net_at: Instant,
    disks: Vec<DiskInfo>,
    disks_at: Option<Instant>,
    kmt: Option<crate::gpu::Kmt>,
    /// Loaded while the full view is open (None without an NVIDIA driver).
    nvml: Option<Nvml>,
    nvml_tried: bool,
    collapsed_at: Option<Instant>,
}

fn ft(f: FILETIME) -> u64 {
    (f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64
}

/// (idle, total) in 100 ns ticks.
fn cpu_times() -> (u64, u64) {
    let (mut i, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe {
        let _ = GetSystemTimes(Some(&mut i), Some(&mut k), Some(&mut u));
    }
    (ft(i), ft(k) + ft(u)) // kernel time includes idle time
}

fn reg_string(key: &str, value: &str) -> Option<String> {
    let (k, v) = (wide(key), wide(value));
    let mut buf = [0u16; 256];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(k.as_ptr()),
            PCWSTR(v.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as _),
            Some(&mut len),
        )
    };
    r.is_ok().then(|| String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]))
}

/// "Intel(R) Core(TM) i7-12700K CPU @ 3.60GHz" -> "Intel Core i7-12700K"
fn clean_cpu_name(raw: &str) -> String {
    let s = raw.replace("(R)", "").replace("(TM)", "").replace("(tm)", "");
    let s = s.split(" CPU @").next().unwrap_or(&s);
    let s = s.split(" @ ").next().unwrap_or(s);
    let s = s.trim_end_matches(|c: char| c.is_whitespace());
    let s = s.strip_suffix(" Processor").unwrap_or(s);
    let words: Vec<&str> = s.split_whitespace().filter(|w| !w.ends_with("-Core")).collect();
    words.join(" ")
}

fn cpu_mhz(threads: u32) -> u64 {
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Ppi {
        number: u32,
        max_mhz: u32,
        current_mhz: u32,
        mhz_limit: u32,
        max_idle: u32,
        current_idle: u32,
    }
    let mut buf = vec![Ppi::default(); threads.max(1) as usize];
    let st = unsafe {
        CallNtPowerInformation(
            ProcessorInformation,
            None,
            0,
            Some(buf.as_mut_ptr() as _),
            (buf.len() * std::mem::size_of::<Ppi>()) as u32,
        )
    };
    if st.is_ok() {
        buf[0].current_mhz as u64
    } else {
        0
    }
}

/// Bytes received / sent by physical adapters since boot.
fn net_totals() -> (u64, u64) {
    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table).is_err() || table.is_null() {
            return (0, 0);
        }
        let n = (*table).NumEntries as usize;
        let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), n);
        let mut out = (0, 0);
        for r in rows {
            let flags = r.InterfaceAndOperStatusFlags._bitfield;
            let hardware = flags & 1 != 0;
            let filter = flags & 2 != 0;
            if hardware && !filter {
                out.0 += r.InOctets;
                out.1 += r.OutOctets;
            }
        }
        FreeMibTable(table as _);
        out
    }
}

fn seek_penalty(root: &str) -> Option<bool> {
    let path = wide(&format!("\\\\.\\{}", root.trim_end_matches('\\')));
    unsafe {
        let h = CreateFileW(
            PCWSTR(path.as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
        .ok()?;
        let q = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceSeekPenaltyProperty,
            QueryType: PropertyStandardQuery,
            ..Default::default()
        };
        let mut d = DEVICE_SEEK_PENALTY_DESCRIPTOR::default();
        let mut ret = 0u32;
        let ok = DeviceIoControl(
            h,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&q as *const _ as _),
            std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(&mut d as *mut _ as _),
            std::mem::size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>() as u32,
            Some(&mut ret),
            None,
        );
        let _ = CloseHandle(h);
        ok.is_ok().then_some(d.IncursSeekPenalty)
    }
}

fn read_disks() -> Vec<DiskInfo> {
    let mut buf = [0u16; 512];
    let n = unsafe { GetLogicalDriveStringsW(Some(&mut buf)) } as usize;
    let mut list: Vec<DiskInfo> = buf[..n.min(buf.len())]
        .split(|&c| c == 0)
        .filter(|s| !s.is_empty())
        .filter_map(|root| {
            let root_s = String::from_utf16_lossy(root);
            let root_w = wide(&root_s);
            let ty = unsafe { GetDriveTypeW(PCWSTR(root_w.as_ptr())) };
            // DRIVE_REMOVABLE = 2, DRIVE_FIXED = 3
            if ty != 2 && ty != 3 {
                return None;
            }
            let (mut avail, mut total) = (0u64, 0u64);
            unsafe { GetDiskFreeSpaceExW(PCWSTR(root_w.as_ptr()), Some(&mut avail), Some(&mut total), None) }.ok()?;
            if total == 0 {
                return None;
            }
            let mut label = [0u16; 261];
            unsafe {
                let _ = GetVolumeInformationW(PCWSTR(root_w.as_ptr()), Some(&mut label), None, None, None, None);
            }
            let ll = label.iter().position(|&c| c == 0).unwrap_or(0);
            let removable = ty == 2;
            Some(DiskInfo {
                mount: root_s.trim_end_matches('\\').to_string(),
                label: String::from_utf16_lossy(&label[..ll]),
                kind: match seek_penalty(&root_s) {
                    Some(true) => "HDD",
                    Some(false) => "SSD",
                    None => "",
                },
                removable,
                total,
                available: avail,
            })
        })
        .collect();
    list.sort_by(|a, b| a.mount.cmp(&b.mount));
    list
}

impl Monitor {
    pub fn new() -> Self {
        let raw =
            reg_string(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString").unwrap_or_default();
        Monitor {
            cpu_prev: cpu_times(),
            cpu_name: clean_cpu_name(raw.trim()),
            net_prev: net_totals(),
            net_at: Instant::now(),
            disks: Vec::new(),
            disks_at: None,
            kmt: crate::gpu::Kmt::new(),
            nvml: None,
            nvml_tried: false,
            collapsed_at: None,
        }
    }

    pub fn stats(&mut self) -> Stats {
        let (idle, total) = cpu_times();
        let (di, dt) = (idle.saturating_sub(self.cpu_prev.0), total.saturating_sub(self.cpu_prev.1));
        self.cpu_prev = (idle, total);
        let cpu = if dt > 0 { (1.0 - di as f64 / dt as f64).clamp(0.0, 1.0) as f32 * 100.0 } else { 0.0 };

        let mut mem = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        unsafe {
            let _ = GlobalMemoryStatusEx(&mut mem);
        }

        let secs = self.net_at.elapsed().as_secs_f64().max(0.001);
        let net = net_totals();
        let (down, up) = (net.0.saturating_sub(self.net_prev.0), net.1.saturating_sub(self.net_prev.1));
        self.net_prev = net;
        self.net_at = Instant::now();

        if self.disks_at.is_none_or(|t| t.elapsed().as_secs() >= DISK_REFRESH_SECS) {
            self.disks = read_disks();
            self.disks_at = Some(Instant::now());
        }

        // ALL_PROCESSOR_GROUPS
        let threads = unsafe { GetActiveProcessorCount(0xffff) };
        Stats {
            cpu,
            cpu_name: self.cpu_name.clone(),
            threads,
            cpu_freq: cpu_mhz(threads),
            ram_used: mem.ullTotalPhys - mem.ullAvailPhys,
            ram_total: mem.ullTotalPhys,
            net_down: down as f64 / secs,
            net_up: up as f64 / secs,
            uptime: unsafe { GetTickCount64() } / 1000,
            disks: self.disks.clone(),
            gpus: self.gpus(),
        }
    }

    fn gpus(&mut self) -> Vec<GpuInfo> {
        let basic = self.kmt.as_mut().map(|k| k.gpus()).unwrap_or_default();
        if FULL.load(Relaxed) {
            self.collapsed_at = None;
            if self.nvml.is_none() && !self.nvml_tried {
                self.nvml_tried = true;
                self.nvml = Nvml::load();
            }
            if let Some(n) = &self.nvml {
                let full = n.gpus();
                if !full.is_empty() {
                    return full;
                }
            }
        } else if self.nvml.is_some() || self.nvml_tried {
            // Give the memory back a little after the full view closes.
            let t = *self.collapsed_at.get_or_insert_with(Instant::now);
            if t.elapsed().as_secs() >= 15 {
                self.nvml = None;
                self.nvml_tried = false;
                self.collapsed_at = None;
            }
        }
        basic
    }
}

// ---------- NVIDIA NVML (nvml.dll ships with the driver; loaded at runtime) ----------

type Dev = *mut core::ffi::c_void;
type FnInit = unsafe extern "C" fn() -> i32;
type FnCount = unsafe extern "C" fn(*mut u32) -> i32;
type FnHandle = unsafe extern "C" fn(u32, *mut Dev) -> i32;
type FnName = unsafe extern "C" fn(Dev, *mut u8, u32) -> i32;
type FnU32Pair = unsafe extern "C" fn(Dev, *mut [u32; 2]) -> i32;
type FnMem = unsafe extern "C" fn(Dev, *mut [u64; 3]) -> i32;
type FnTemp = unsafe extern "C" fn(Dev, u32, *mut u32) -> i32;
type FnU32 = unsafe extern "C" fn(Dev, *mut u32) -> i32;
type FnFan2 = unsafe extern "C" fn(Dev, u32, *mut u32) -> i32;

struct Nvml {
    lib: HMODULE,
    shutdown: FnInit,
    count: FnCount,
    handle: FnHandle,
    name: FnName,
    util: FnU32Pair,
    mem: FnMem,
    temp: FnTemp,
    power: FnU32,
    fan: Option<FnFan2>,
}

unsafe impl Send for Nvml {}

impl Drop for Nvml {
    fn drop(&mut self) {
        unsafe {
            (self.shutdown)();
            let _ = FreeLibrary(self.lib);
        }
    }
}

impl Nvml {
    fn load() -> Option<Nvml> {
        unsafe {
            let lib: HMODULE = LoadLibraryExW(PCWSTR(wide("nvml.dll").as_ptr()), None, LOAD_LIBRARY_SEARCH_SYSTEM32)
                .or_else(|_| {
                    let pf = std::env::var("ProgramW6432").unwrap_or_else(|_| r"C:\Program Files".into());
                    let p = wide(&format!(r"{pf}\NVIDIA Corporation\NVSMI\nvml.dll"));
                    LoadLibraryExW(PCWSTR(p.as_ptr()), None, Default::default())
                })
                .ok()?;
            macro_rules! sym {
                ($name:expr, $t:ty) => {
                    std::mem::transmute::<_, $t>(GetProcAddress(lib, $name)?)
                };
            }
            let init = sym!(s!("nvmlInit_v2"), FnInit);
            if init() != 0 {
                return None;
            }
            Some(Nvml {
                lib,
                shutdown: sym!(s!("nvmlShutdown"), FnInit),
                count: sym!(s!("nvmlDeviceGetCount_v2"), FnCount),
                handle: sym!(s!("nvmlDeviceGetHandleByIndex_v2"), FnHandle),
                name: sym!(s!("nvmlDeviceGetName"), FnName),
                util: sym!(s!("nvmlDeviceGetUtilizationRates"), FnU32Pair),
                mem: sym!(s!("nvmlDeviceGetMemoryInfo"), FnMem),
                temp: sym!(s!("nvmlDeviceGetTemperature"), FnTemp),
                power: sym!(s!("nvmlDeviceGetPowerUsage"), FnU32),
                fan: GetProcAddress(lib, s!("nvmlDeviceGetFanSpeed_v2")).map(|f| std::mem::transmute::<_, FnFan2>(f)),
            })
        }
    }

    fn gpus(&self) -> Vec<GpuInfo> {
        unsafe {
            let mut n = 0u32;
            if (self.count)(&mut n) != 0 {
                return Vec::new();
            }
            (0..n)
                .filter_map(|i| {
                    let mut d: Dev = std::ptr::null_mut();
                    if (self.handle)(i, &mut d) != 0 {
                        return None;
                    }
                    let mut name = [0u8; 96];
                    (self.name)(d, name.as_mut_ptr(), name.len() as u32);
                    let nl = name.iter().position(|&c| c == 0).unwrap_or(0);
                    let name = String::from_utf8_lossy(&name[..nl]);
                    let mut util = [0u32; 2];
                    let load = if (self.util)(d, &mut util) == 0 { util[0] } else { 0 };
                    let mut mem = [0u64; 3]; // total, free, used
                    let mem_ok = (self.mem)(d, &mut mem) == 0;
                    let mut t = 0u32;
                    let temp = ((self.temp)(d, 0, &mut t) == 0).then_some(t);
                    let mut mw = 0u32;
                    let power = ((self.power)(d, &mut mw) == 0).then_some(mw as f64 / 1000.0);
                    let mut f = 0u32;
                    let fan = self.fan.and_then(|fan| (fan(d, 0, &mut f) == 0).then_some(f));
                    Some(GpuInfo {
                        name: name.trim_start_matches("NVIDIA ").to_string(),
                        load,
                        mem_used: if mem_ok { mem[2] } else { 0 },
                        mem_total: if mem_ok { mem[0] } else { 0 },
                        temp,
                        power,
                        fan,
                    })
                })
                .collect()
        }
    }
}

// ---------- polling thread ----------

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

static ACTIVE: AtomicBool = AtomicBool::new(false);
static FULL: AtomicBool = AtomicBool::new(false);

/// Polling is paused while the system widget is hidden.
pub fn set_active(on: bool) {
    ACTIVE.store(on, Relaxed);
}

/// The full (hover) view is open: include NVIDIA power / fan data.
pub fn set_full(on: bool) {
    FULL.store(on, Relaxed);
}

/// Poll every 2 s on a background thread (disk and GPU queries can block briefly).
/// Sending on the returned channel triggers an immediate poll.
pub fn spawn(f: impl Fn(Stats) + Send + 'static) -> std::sync::mpsc::Sender<()> {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let mut m = Monitor::new();
        loop {
            if ACTIVE.load(Relaxed) {
                f(m.stats());
            }
            if let Err(std::sync::mpsc::RecvTimeoutError::Disconnected) =
                rx.recv_timeout(std::time::Duration::from_secs(2))
            {
                return;
            }
        }
    });
    tx
}
