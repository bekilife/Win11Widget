//! GPU load, VRAM, temperature, power and fan through D3DKMT — the kernel graphics interface
//! Task Manager uses. Works for any vendor and costs almost no memory (NVIDIA's NVML would
//! add ~20 MB that the driver never gives back).

use windows::{
    Wdk::Graphics::Direct3D::*,
    Win32::Foundation::{LUID, NTSTATUS},
};

use crate::system::GpuInfo;

struct Adapter {
    handle: u32,
    luid: LUID,
    name: String,
    nodes: u32,
    segments: u32,
    /// node running times (100 ns) at the previous poll
    prev: Vec<i64>,
    prev_at: std::time::Instant,
    vram_total: u64,
    max_fan_rpm: u32,
}

pub struct Kmt {
    adapters: Vec<Adapter>,
}

fn ok(s: NTSTATUS) -> bool {
    s.0 >= 0
}

unsafe fn query_info<T: Default>(h: u32, ty: KMTQUERYADAPTERINFOTYPE) -> Option<T> {
    let mut out = T::default();
    let mut q = D3DKMT_QUERYADAPTERINFO {
        hAdapter: h,
        Type: ty,
        pPrivateDriverData: &mut out as *mut T as _,
        PrivateDriverDataSize: std::mem::size_of::<T>() as u32,
    };
    ok(D3DKMTQueryAdapterInfo(&mut q)).then_some(out)
}

unsafe fn stats(luid: LUID, ty: D3DKMT_QUERYSTATISTICS_TYPE, id: Option<u32>) -> Option<D3DKMT_QUERYSTATISTICS> {
    let mut q: D3DKMT_QUERYSTATISTICS = std::mem::zeroed();
    q.Type = ty;
    q.AdapterLuid = luid;
    if let Some(id) = id {
        // QueryNode and QuerySegment share the same first field
        q.Anonymous.QueryNode = D3DKMT_QUERYSTATISTICS_QUERY_NODE { NodeId: id };
    }
    ok(D3DKMTQueryStatistics(&q)).then_some(q)
}

impl Kmt {
    pub fn new() -> Option<Kmt> {
        unsafe {
            let mut e = D3DKMT_ENUMADAPTERS2 { NumAdapters: 0, pAdapters: std::ptr::null_mut() };
            if !ok(D3DKMTEnumAdapters2(&mut e)) || e.NumAdapters == 0 {
                return None;
            }
            let mut list = vec![D3DKMT_ADAPTERINFO::default(); e.NumAdapters as usize];
            e.pAdapters = list.as_mut_ptr();
            if !ok(D3DKMTEnumAdapters2(&mut e)) {
                return None;
            }
            list.truncate(e.NumAdapters as usize);
            let mut adapters = Vec::new();
            for a in list {
                // bit 0 RenderSupported, bit 2 SoftwareDevice
                let ty: u32 = query_info(a.hAdapter, KMTQAITYPE_ADAPTERTYPE).unwrap_or(0);
                if ty & 1 == 0 || ty & 4 != 0 {
                    continue;
                }
                let Some(info) = stats(a.AdapterLuid, D3DKMT_QUERYSTATISTICS_ADAPTER, None) else { continue };
                let ai = info.QueryResult.AdapterInformation;
                let mut vram_total = 0u64;
                for s in 0..ai.NbSegments {
                    if let Some(q) = stats(a.AdapterLuid, D3DKMT_QUERYSTATISTICS_SEGMENT, Some(s)) {
                        let si = q.QueryResult.SegmentInformation;
                        if si.Aperture == 0 {
                            vram_total += si.CommitLimit;
                        }
                    }
                }
                // Integrated GPUs only have shared (aperture) memory: not shown, like before.
                if vram_total == 0 {
                    continue;
                }
                let reg: D3DKMT_ADAPTERREGISTRYINFO =
                    query_info(a.hAdapter, KMTQAITYPE_ADAPTERREGISTRYINFO).unwrap_or_default();
                let n = reg.AdapterString.iter().position(|&c| c == 0).unwrap_or(0);
                let name = String::from_utf16_lossy(&reg.AdapterString[..n]);
                let mut ad = Adapter {
                    handle: a.hAdapter,
                    luid: a.AdapterLuid,
                    name: name.trim_start_matches("NVIDIA ").trim().to_string(),
                    nodes: ai.NodeCount,
                    segments: ai.NbSegments,
                    prev: Vec::new(),
                    prev_at: std::time::Instant::now(),
                    vram_total,
                    max_fan_rpm: query_info::<D3DKMT_ADAPTER_PERFDATACAPS>(a.hAdapter, KMTQAITYPE_ADAPTERPERFDATA_CAPS)
                        .map_or(0, |c| c.MaxFanRPM),
                };
                ad.prev = ad.node_times();
                adapters.push(ad);
            }
            (!adapters.is_empty()).then_some(Kmt { adapters })
        }
    }

    pub fn gpus(&mut self) -> Vec<GpuInfo> {
        self.adapters.iter_mut().map(Adapter::read).collect()
    }
}

impl Adapter {
    fn node_times(&self) -> Vec<i64> {
        (0..self.nodes)
            .map(|n| unsafe {
                stats(self.luid, D3DKMT_QUERYSTATISTICS_NODE, Some(n))
                    .map(|q| q.QueryResult.NodeInformation.GlobalInformation.RunningTime)
                    .unwrap_or(0)
            })
            .collect()
    }

    fn read(&mut self) -> GpuInfo {
        // Load: the busiest engine over the last interval (what Task Manager shows).
        let now = self.node_times();
        let elapsed = self.prev_at.elapsed().as_nanos() as f64 / 100.0;
        let mut load = 0.0f64;
        if elapsed > 0.0 && now.len() == self.prev.len() {
            for (a, b) in now.iter().zip(&self.prev) {
                load = load.max((a - b) as f64 / elapsed);
            }
        }
        self.prev = now;
        self.prev_at = std::time::Instant::now();

        let mut used = 0u64;
        for s in 0..self.segments {
            if let Some(q) = unsafe { stats(self.luid, D3DKMT_QUERYSTATISTICS_SEGMENT, Some(s)) } {
                let si = unsafe { q.QueryResult.SegmentInformation };
                if si.Aperture == 0 {
                    used += si.BytesResident;
                }
            }
        }
        let perf: Option<D3DKMT_ADAPTER_PERFDATA> = unsafe { query_info(self.handle, KMTQAITYPE_ADAPTERPERFDATA) };
        let temp = perf.map(|p| p.Temperature / 10).filter(|t| *t > 0);
        // Power is reported in tenths of a percent of the board power limit.
        let power = perf.map(|p| (p.Power + 5) / 10).filter(|p| *p > 0);
        let fan = perf
            .filter(|_| self.max_fan_rpm > 0)
            .map(|p| ((p.FanRPM as u64 * 100 + self.max_fan_rpm as u64 / 2) / self.max_fan_rpm as u64).min(100) as u32);
        GpuInfo {
            name: self.name.clone(),
            load: (load * 100.0).round().clamp(0.0, 100.0) as u32,
            mem_used: used,
            mem_total: self.vram_total,
            temp,
            power,
            fan,
        }
    }
}
