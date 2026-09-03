//! Temperature sources.
//!
//! CPU: the PawnIO kernel driver is asked to read the vendor's thermal
//! registers directly - MSRs on Intel, SMN on AMD. This is the same mechanism
//! LibreHardwareMonitor uses, minus the .NET runtime; a reading costs a couple
//! of microseconds and no allocation.
//!
//! GPU: NVML on NVIDIA. Anything else falls back to D3DKMT, the WDDM interface
//! Task Manager reads, which reports a temperature for any vendor's adapter.
//!
//! Every source degrades to `None` rather than failing, so a missing driver
//! shows "N/A" on the display instead of stopping the service.

use std::ffi::{c_void, CString};
use std::path::Path;

// ============================================================
// CPU
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CpuVendor {
    Intel,
    Amd,
    Unknown,
}

pub fn detect_cpu_vendor() -> CpuVendor {
    // CPUID leaf 0 returns the vendor string in EBX, EDX, ECX.
    #[cfg(target_arch = "x86_64")]
    {
        let r = std::arch::x86_64::__cpuid(0);
        let mut s = [0u8; 12];
        s[0..4].copy_from_slice(&r.ebx.to_le_bytes());
        s[4..8].copy_from_slice(&r.edx.to_le_bytes());
        s[8..12].copy_from_slice(&r.ecx.to_le_bytes());
        return match &s {
            b"GenuineIntel" => CpuVendor::Intel,
            b"AuthenticAMD" => CpuVendor::Amd,
            _ => CpuVendor::Unknown,
        };
    }
    #[allow(unreachable_code)]
    CpuVendor::Unknown
}

/// Returns the CPU family and model, combined the way Intel/AMD document them
/// (extended family and model folded in).
#[cfg(target_arch = "x86_64")]
fn cpu_family_model() -> (u32, u32) {
    {
        let r = std::arch::x86_64::__cpuid(1);
        let base_family = (r.eax >> 8) & 0xF;
        let base_model = (r.eax >> 4) & 0xF;
        let family = if base_family == 0xF { base_family + ((r.eax >> 20) & 0xFF) } else { base_family };
        let model = if base_family == 0x6 || base_family == 0xF {
            base_model + (((r.eax >> 16) & 0xF) << 4)
        } else {
            base_model
        };
        (family, model)
    }
}

// --- PawnIO bindings -------------------------------------------------------

type OpenFn = unsafe extern "system" fn(*mut *mut c_void) -> i32;
type LoadFn = unsafe extern "system" fn(*mut c_void, *const u8, usize) -> i32;
type ExecFn = unsafe extern "system" fn(
    *mut c_void,
    *const i8,
    *const u64,
    usize,
    *mut u64,
    usize,
    *mut usize,
) -> i32;
type CloseFn = unsafe extern "system" fn(*mut c_void) -> i32;

struct PawnIo {
    _lib: libloading::Library,
    handle: *mut c_void,
    exec: ExecFn,
    close: CloseFn,
}

impl PawnIo {
    /// Open the driver and load a compiled PawnIO module (a .bin blob).
    fn open(module: &Path) -> Result<Self, String> {
        let blob = std::fs::read(module)
            .map_err(|e| format!("missing PawnIO module {}: {e}", module.display()))?;
        unsafe {
            let lib = libloading::Library::new(r"C:\Program Files\PawnIO\PawnIOLib.dll")
                .map_err(|_| "PawnIO is not installed (PawnIOLib.dll not found)".to_string())?;
            let open: libloading::Symbol<OpenFn> =
                lib.get(b"pawnio_open").map_err(|e| e.to_string())?;
            let load: libloading::Symbol<LoadFn> =
                lib.get(b"pawnio_load").map_err(|e| e.to_string())?;
            let exec: libloading::Symbol<ExecFn> =
                lib.get(b"pawnio_execute").map_err(|e| e.to_string())?;
            let close: libloading::Symbol<CloseFn> =
                lib.get(b"pawnio_close").map_err(|e| e.to_string())?;
            let (exec, close) = (*exec, *close);

            let mut handle: *mut c_void = std::ptr::null_mut();
            let hr = open(&mut handle);
            if hr != 0 {
                // 0x80070005 is access denied: PawnIO only talks to elevated callers.
                return Err(if hr as u32 == 0x8007_0005 {
                    "PawnIO requires administrator rights (run elevated)".into()
                } else {
                    format!("pawnio_open failed (0x{:08X})", hr as u32)
                });
            }
            let hr = load(handle, blob.as_ptr(), blob.len());
            if hr != 0 {
                close(handle);
                return Err(format!("pawnio_load failed (0x{:08X})", hr as u32));
            }
            Ok(PawnIo { _lib: lib, handle, exec, close })
        }
    }

    fn call(&self, name: &CString, input: &[u64], out: &mut [u64]) -> Option<usize> {
        unsafe {
            let mut n = 0usize;
            let hr = (self.exec)(
                self.handle,
                name.as_ptr(),
                input.as_ptr(),
                input.len(),
                out.as_mut_ptr(),
                out.len(),
                &mut n,
            );
            if hr == 0 { Some(n) } else { None }
        }
    }
}

impl Drop for PawnIo {
    fn drop(&mut self) {
        unsafe { (self.close)(self.handle) };
    }
}

/// CPU temperature reader. Opens the driver once and reuses it for every tick.
pub struct CpuSensor {
    io: PawnIo,
    vendor: CpuVendor,
    read_msr: CString,
    read_smn: CString,
    /// Intel: TjMax in C, read once at startup.
    tjmax: f32,
    /// AMD: subtracted from the raw Tctl reading.
    tctl_offset: f32,
}

impl CpuSensor {
    pub fn new(modules_dir: &Path) -> Result<Self, String> {
        let vendor = detect_cpu_vendor();
        let (family, _model) = cpu_family_model();

        let module = match vendor {
            CpuVendor::Intel => "IntelMSR.bin",
            CpuVendor::Amd => match family {
                0x17 | 0x18 | 0x19 | 0x1A => "AMDFamily17.bin",
                0x10..=0x16 => "AMDFamily10.bin",
                0x0F => "AMDFamily0F.bin",
                _ => "AMDFamily17.bin",
            },
            CpuVendor::Unknown => return Err("unrecognised CPU vendor".into()),
        };

        let io = PawnIo::open(&modules_dir.join(module))?;
        let read_msr = CString::new("ioctl_read_msr").unwrap();
        let read_smn = CString::new("ioctl_read_smn").unwrap();

        let mut s = CpuSensor {
            io,
            vendor,
            read_msr,
            read_smn,
            tjmax: 100.0,
            tctl_offset: 0.0,
        };

        if vendor == CpuVendor::Intel {
            // MSR_TEMPERATURE_TARGET bits 23:16 hold TjMax.
            if let Some(v) = s.msr(0x1A2) {
                let tj = ((v >> 16) & 0xFF) as f32;
                if (50.0..=130.0).contains(&tj) {
                    s.tjmax = tj;
                }
            }
        } else if vendor == CpuVendor::Amd {
            // Ryzen models with a 27C reporting offset. Threadripper/older SKUs
            // differ, but the common desktop parts report Tctl == Tdie.
            s.tctl_offset = 0.0;
        }
        Ok(s)
    }

    // PawnIO modules validate in_size and out_size exactly and reject anything
    // else with STATUS_INVALID_PARAMETER, so these buffers must be 1 entry.
    fn msr(&self, index: u64) -> Option<u64> {
        let mut out = [0u64; 1];
        self.io.call(&self.read_msr, &[index], &mut out).map(|_| out[0])
    }

    fn smn(&self, address: u64) -> Option<u64> {
        let mut out = [0u64; 1];
        self.io.call(&self.read_smn, &[address], &mut out).map(|_| out[0])
    }

    /// Current package temperature in degrees Celsius.
    pub fn read(&self) -> Option<f32> {
        match self.vendor {
            CpuVendor::Intel => {
                // IA32_PACKAGE_THERM_STATUS holds the distance below TjMax.
                // Bit 31 marks the reading valid; fall back to core 0's
                // IA32_THERM_STATUS on parts without a package sensor.
                for msr in [0x1B1u64, 0x19C] {
                    if let Some(v) = self.msr(msr) {
                        if v >> 31 & 1 == 1 {
                            let delta = ((v >> 16) & 0x7F) as f32;
                            let t = self.tjmax - delta;
                            if (0.0..=130.0).contains(&t) {
                                return Some(t);
                            }
                        }
                    }
                }
                None
            }
            CpuVendor::Amd => {
                // SMN THM_TCON_CUR_TMP: bits 31:21 are the reading in 1/8 C,
                // bit 19 selects the -49 C range.
                let v = self.smn(0x0005_9800)?;
                let mut t = ((v >> 21) & 0x7FF) as f32 * 0.125;
                if v >> 19 & 1 == 1 {
                    t -= 49.0;
                }
                t -= self.tctl_offset;
                if (0.0..=130.0).contains(&t) { Some(t) } else { None }
            }
            CpuVendor::Unknown => None,
        }
    }
}

// ============================================================
// GPU
// ============================================================

pub enum GpuSensor {
    Nvidia(Box<nvml_wrapper::Nvml>),
    /// Vendor-neutral WDDM path, used for AMD and Intel adapters.
    Wddm(d3dkmt::Adapter),
}

impl GpuSensor {
    /// Try NVML first; fall back to the WDDM adapter with the highest
    /// dedicated memory, which is reliably the discrete GPU. `wddm` forces the
    /// vendor-neutral path even on an NVIDIA card.
    pub fn new(preference: &str) -> Result<Self, String> {
        let want_nvidia = matches!(preference, "auto" | "nvidia");
        if want_nvidia {
            match nvml_wrapper::Nvml::init() {
                Ok(nvml) => {
                    if nvml.device_count().unwrap_or(0) > 0 {
                        return Ok(GpuSensor::Nvidia(Box::new(nvml)));
                    }
                }
                Err(e) if preference == "nvidia" => {
                    return Err(format!("NVML unavailable: {e}"));
                }
                Err(_) => {}
            }
        }
        d3dkmt::Adapter::best().map(GpuSensor::Wddm).ok_or_else(|| {
            "no GPU temperature source found (no NVIDIA driver, and WDDM reported none)".into()
        })
    }

    pub fn name(&self) -> &'static str {
        match self {
            GpuSensor::Nvidia(_) => "NVIDIA (NVML)",
            GpuSensor::Wddm(_) => "WDDM (D3DKMT)",
        }
    }

    pub fn read(&self) -> Option<f32> {
        match self {
            GpuSensor::Nvidia(nvml) => {
                use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
                let dev = nvml.device_by_index(0).ok()?;
                dev.temperature(TemperatureSensor::Gpu).ok().map(|t| t as f32)
            }
            GpuSensor::Wddm(a) => a.temperature(),
        }
    }
}

/// Minimal D3DKMT bindings: enumerate WDDM adapters and read the performance
/// data block that carries GPU temperature. This is what Task Manager uses, so
/// it works for NVIDIA, AMD and Intel alike without a vendor SDK.
pub mod d3dkmt {
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Luid {
        low: u32,
        high: i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct AdapterInfo {
        handle: u32,
        luid: Luid,
        source_count: u32,
        present_move_regions_preferred: u32,
    }

    #[repr(C)]
    struct EnumAdapters2 {
        count: u32,
        adapters: *mut AdapterInfo,
    }

    #[repr(C)]
    struct QueryAdapterInfo {
        handle: u32,
        info_type: u32,
        private_data: *mut c_void,
        private_data_size: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct AdapterPerfData {
        physical_adapter_index: u32,
        memory_frequency: u64,
        max_memory_frequency: u64,
        max_memory_frequency_oc: u64,
        memory_bandwidth: u64,
        pcie_bandwidth: u64,
        fan_rpm: u32,
        power: u32,
        /// Tenths of a degree Celsius.
        temperature: u32,
        power_state_override: u8,
    }

    #[repr(C)]
    #[derive(Default)]
    struct SegmentGroupSizeInfo {
        segment_group: u32,
        _pad: u32,
        dedicated: u64,
        shared: u64,
    }

    const KMTQAITYPE_ADAPTERPERFDATA: u32 = 62;
    const KMTQAITYPE_GETSEGMENTGROUPSIZE: u32 = 34;

    #[link(name = "gdi32")]
    extern "system" {
        fn D3DKMTEnumAdapters2(arg: *mut EnumAdapters2) -> i32;
        fn D3DKMTQueryAdapterInfo(arg: *mut QueryAdapterInfo) -> i32;
        fn D3DKMTCloseAdapter(arg: *const u32) -> i32;
    }

    /// A single WDDM adapter, kept open for the lifetime of the service.
    pub struct Adapter {
        handle: u32,
    }

    impl Adapter {
        /// Pick the adapter with the most dedicated video memory, which
        /// distinguishes a discrete GPU from integrated graphics and from
        /// virtual displays such as headset mirrors.
        pub fn best() -> Option<Adapter> {
            unsafe {
                // First call with a null pointer reports how many adapters exist.
                let mut e = EnumAdapters2 { count: 0, adapters: std::ptr::null_mut() };
                if D3DKMTEnumAdapters2(&mut e) != 0 {
                    return None;
                }
                let mut list = vec![
                    AdapterInfo {
                        handle: 0,
                        luid: Luid::default(),
                        source_count: 0,
                        present_move_regions_preferred: 0
                    };
                    e.count as usize
                ];
                e.adapters = list.as_mut_ptr();
                if D3DKMTEnumAdapters2(&mut e) != 0 {
                    return None;
                }

                let mut best: Option<(u64, u32)> = None;
                for a in list.iter().take(e.count as usize) {
                    let mut seg = SegmentGroupSizeInfo::default();
                    let mut q = QueryAdapterInfo {
                        handle: a.handle,
                        info_type: KMTQAITYPE_GETSEGMENTGROUPSIZE,
                        private_data: &mut seg as *mut _ as *mut c_void,
                        private_data_size: std::mem::size_of::<SegmentGroupSizeInfo>() as u32,
                    };
                    let dedicated =
                        if D3DKMTQueryAdapterInfo(&mut q) == 0 { seg.dedicated } else { 0 };

                    // Only keep adapters that actually report a temperature.
                    // Probe by handle: building an Adapter here would close it
                    // on drop and leave the winner dangling.
                    if raw_temperature(a.handle).is_none() {
                        let _ = D3DKMTCloseAdapter(&a.handle);
                        continue;
                    }
                    match best {
                        Some((mem, h)) if mem >= dedicated => {
                            let _ = D3DKMTCloseAdapter(&a.handle);
                            best = Some((mem, h));
                        }
                        Some((_, h)) => {
                            let _ = D3DKMTCloseAdapter(&h);
                            best = Some((dedicated, a.handle));
                        }
                        None => best = Some((dedicated, a.handle)),
                    }
                }
                best.map(|(_, handle)| Adapter { handle })
            }
        }

        pub fn temperature(&self) -> Option<f32> {
            // Reported in tenths of a degree.
            let t = raw_temperature(self.handle)? as f32 / 10.0;
            if (0.0..=130.0).contains(&t) { Some(t) } else { None }
        }
    }

    /// Read the perf-data block for an adapter handle. Kept free-standing so
    /// candidates can be probed during enumeration without taking ownership.
    fn raw_temperature(handle: u32) -> Option<u32> {
        unsafe {
            let mut perf = AdapterPerfData::default();
            let mut q = QueryAdapterInfo {
                handle,
                info_type: KMTQAITYPE_ADAPTERPERFDATA,
                private_data: &mut perf as *mut _ as *mut c_void,
                private_data_size: std::mem::size_of::<AdapterPerfData>() as u32,
            };
            if D3DKMTQueryAdapterInfo(&mut q) != 0 || perf.temperature == 0 {
                return None;
            }
            Some(perf.temperature)
        }
    }

    impl Drop for Adapter {
        fn drop(&mut self) {
            unsafe { D3DKMTCloseAdapter(&self.handle) };
        }
    }
}

/// Both temperature sources, opened once and polled every tick.
pub struct Readings {
    pub cpu: Option<CpuSensor>,
    pub gpu: Option<GpuSensor>,
}

impl Readings {
    pub fn open(cfg: &crate::config::Sensors) -> (Self, Vec<String>) {
        let mut notes = Vec::new();
        let modules = crate::config::resolve(&cfg.pawnio_modules);

        let cpu = if cfg.cpu == "none" {
            None
        } else {
            match CpuSensor::new(&modules) {
                Ok(s) => {
                    notes.push(format!("CPU: {:?} via PawnIO", s.vendor));
                    Some(s)
                }
                Err(e) => {
                    notes.push(format!("CPU temperature unavailable - {e}"));
                    None
                }
            }
        };

        let gpu = if cfg.gpu == "none" {
            None
        } else {
            match GpuSensor::new(&cfg.gpu) {
                Ok(s) => {
                    notes.push(format!("GPU: {}", s.name()));
                    Some(s)
                }
                Err(e) => {
                    notes.push(format!("GPU temperature unavailable - {e}"));
                    None
                }
            }
        };

        (Readings { cpu, gpu }, notes)
    }

    pub fn sample(&self) -> (Option<f32>, Option<f32>) {
        (
            self.cpu.as_ref().and_then(CpuSensor::read),
            self.gpu.as_ref().and_then(GpuSensor::read),
        )
    }
}
