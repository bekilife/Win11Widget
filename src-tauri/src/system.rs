//! Full system status: CPU, memory, GPU (NVIDIA via NVML), network throughput, uptime and every mounted disk.

use std::{sync::Mutex, time::Instant};

use nvml_wrapper::{enum_wrappers::device::TemperatureSensor, Nvml};
use serde::Serialize;
use sysinfo::{DiskKind, Disks, Networks, System};

/// Disks change rarely; enumerating volumes every poll would be wasteful.
const DISK_REFRESH_SECS: u64 = 10;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DiskInfo {
    /// "C:"
    mount: String,
    /// Volume label, may be empty.
    label: String,
    /// "SSD", "HDD" or "".
    kind: String,
    file_system: String,
    removable: bool,
    total: u64,
    available: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    name: String,
    /// percent
    load: u32,
    mem_used: u64,
    mem_total: u64,
    /// °C
    temp: Option<u32>,
    /// watts
    power: Option<f64>,
    /// percent
    fan: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    cpu: f32,
    cpu_name: String,
    threads: usize,
    /// MHz
    cpu_freq: u64,
    ram_used: u64,
    ram_total: u64,
    /// bytes per second since the previous poll
    net_down: f64,
    net_up: f64,
    uptime: u64,
    disks: Vec<DiskInfo>,
    gpus: Vec<GpuInfo>,
}

pub struct Monitor {
    inner: Mutex<Inner>,
}

struct Inner {
    sys: System,
    nets: Networks,
    net_at: Instant,
    disks: Vec<DiskInfo>,
    disks_at: Option<Instant>,
    /// None when there is no NVIDIA driver (AMD/Intel GPUs are not supported yet).
    nvml: Option<Nvml>,
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

fn read_gpus(nvml: &Nvml) -> Vec<GpuInfo> {
    let count = nvml.device_count().unwrap_or(0);
    (0..count)
        .filter_map(|i| nvml.device_by_index(i).ok())
        .map(|d| {
            let mem = d.memory_info().ok();
            GpuInfo {
                name: d.name().unwrap_or_default().trim_start_matches("NVIDIA ").to_string(),
                load: d.utilization_rates().map(|u| u.gpu).unwrap_or(0),
                mem_used: mem.as_ref().map_or(0, |m| m.used),
                mem_total: mem.as_ref().map_or(0, |m| m.total),
                temp: d.temperature(TemperatureSensor::Gpu).ok(),
                power: d.power_usage().ok().map(|mw| mw as f64 / 1000.0),
                fan: d.fan_speed(0).ok(),
            }
        })
        .collect()
}

fn read_disks() -> Vec<DiskInfo> {
    let mut list: Vec<DiskInfo> = Disks::new_with_refreshed_list()
        .list()
        .iter()
        .filter(|d| d.total_space() > 0)
        .map(|d| DiskInfo {
            mount: d.mount_point().to_string_lossy().trim_end_matches('\\').to_string(),
            label: d.name().to_string_lossy().to_string(),
            kind: match d.kind() {
                DiskKind::SSD => "SSD".into(),
                DiskKind::HDD => "HDD".into(),
                _ => String::new(),
            },
            file_system: d.file_system().to_string_lossy().to_string(),
            removable: d.is_removable(),
            total: d.total_space(),
            available: d.available_space(),
        })
        .collect();
    list.sort_by(|a, b| a.mount.cmp(&b.mount));
    list.dedup_by(|a, b| a.mount == b.mount);
    list
}

impl Monitor {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu_all(); // brand, frequency and a baseline for usage
        sys.refresh_memory();
        Monitor {
            inner: Mutex::new(Inner {
                sys,
                nets: Networks::new_with_refreshed_list(),
                net_at: Instant::now(),
                disks: Vec::new(),
                disks_at: None,
                nvml: Nvml::init().ok(),
            }),
        }
    }

    pub fn stats(&self) -> Stats {
        let mut g = self.inner.lock().unwrap();
        g.sys.refresh_cpu_usage();
        g.sys.refresh_memory();

        let secs = g.net_at.elapsed().as_secs_f64().max(0.001);
        g.nets.refresh(true);
        g.net_at = Instant::now();
        let (down, up) = g.nets.list().values().fold((0u64, 0u64), |(d, u), n| (d + n.received(), u + n.transmitted()));

        if g.disks_at.is_none_or(|t| t.elapsed().as_secs() >= DISK_REFRESH_SECS) {
            g.disks = read_disks();
            g.disks_at = Some(Instant::now());
        }

        let cpus = g.sys.cpus();
        Stats {
            cpu: g.sys.global_cpu_usage(),
            cpu_name: cpus.first().map(|c| clean_cpu_name(c.brand())).unwrap_or_default(),
            threads: cpus.len(),
            cpu_freq: cpus.first().map(|c| c.frequency()).unwrap_or(0),
            ram_used: g.sys.used_memory(),
            ram_total: g.sys.total_memory(),
            net_down: down as f64 / secs,
            net_up: up as f64 / secs,
            uptime: System::uptime(),
            disks: g.disks.clone(),
            gpus: g.nvml.as_ref().map(read_gpus).unwrap_or_default(),
        }
    }
}
