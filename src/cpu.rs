//! Temperatura da CPU Intel por núcleo, lida dos MSRs através do driver PawnIO.

use std::arch::x86_64::__cpuid;
use std::ffi::c_void;
use std::mem::{transmute, zeroed};
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{FreeLibrary, HMODULE};
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_WITH_ALTERED_SEARCH_PATH,
};
use windows_sys::Win32::System::Registry::HKEY_LOCAL_MACHINE;
use windows_sys::Win32::System::SystemInformation::{
    GetLogicalProcessorInformationEx, RelationProcessorCore, GROUP_AFFINITY,
    SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
};
use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadGroupAffinity};

use crate::util::{reg_string, wide};

/// Módulo assinado do projeto PawnIO.Modules (release 0.2.11), que libera leitura dos MSRs térmicos.
static INTEL_MSR_MODULE: &[u8] = include_bytes!("../assets/IntelMSR.bin");

const MSR_IA32_THERM_STATUS: u64 = 0x19C;
const MSR_IA32_TEMPERATURE_TARGET: u64 = 0x1A2;
const MSR_IA32_PACKAGE_THERM_STATUS: u64 = 0x1B1;

const E_ACCESSDENIED: i32 = 0x8007_0005u32 as i32;
const E_FILE_NOT_FOUND: i32 = 0x8007_0002u32 as i32;

type PawnOpen = unsafe extern "system" fn(*mut *mut c_void) -> i32;
type PawnLoad = unsafe extern "system" fn(*mut c_void, *const u8, usize) -> i32;
type PawnExecute = unsafe extern "system" fn(
    *mut c_void,
    *const u8,
    *const u64,
    usize,
    *mut u64,
    usize,
    *mut usize,
) -> i32;
type PawnClose = unsafe extern "system" fn(*mut c_void) -> i32;

pub enum CpuError {
    NotIntel,
    NoSensor,
    PawnIoMissing,
    PawnIoFailed(i32),
}

impl CpuError {
    pub fn message(&self) -> String {
        match self {
            CpuError::NotIntel => "Somente CPUs Intel são suportadas".into(),
            CpuError::NoSensor => "CPU sem sensor térmico digital".into(),
            CpuError::PawnIoMissing | CpuError::PawnIoFailed(E_FILE_NOT_FOUND) => {
                "Driver PawnIO não instalado".into()
            }
            CpuError::PawnIoFailed(E_ACCESSDENIED) => {
                "Sem permissão: execute como administrador".into()
            }
            CpuError::PawnIoFailed(hr) => format!("Falha no PawnIO (0x{:08X})", *hr as u32),
        }
    }

    pub fn needs_pawnio(&self) -> bool {
        matches!(self, CpuError::PawnIoMissing | CpuError::PawnIoFailed(E_FILE_NOT_FOUND))
    }
}

pub struct Core {
    /// "P1", "E3", "C2"
    pub short: String,
    /// "P-core 1", "E-core 3", "Núcleo 2"
    pub long: String,
    group: u16,
    mask: usize,
    tjmax: i32,
}

pub struct Cpu {
    pub name: String,
    pub cores: Vec<Core>,
    pub tjmax: i32,
    pub has_package: bool,
    lib: HMODULE,
    handle: *mut c_void,
    execute: PawnExecute,
    close: PawnClose,
}

impl Cpu {
    pub fn open() -> Result<Cpu, CpuError> {
        let (vendor, max_leaf) = {
            let r = __cpuid(0);
            let mut v = [0u8; 12];
            v[0..4].copy_from_slice(&r.ebx.to_le_bytes());
            v[4..8].copy_from_slice(&r.edx.to_le_bytes());
            v[8..12].copy_from_slice(&r.ecx.to_le_bytes());
            (v, r.eax)
        };
        if &vendor != b"GenuineIntel" {
            return Err(CpuError::NotIntel);
        }
        if max_leaf < 6 {
            return Err(CpuError::NoSensor);
        }
        // CPUID.06H:EAX bit 0 = sensor digital por núcleo, bit 6 = sensor do pacote
        let thermal = __cpuid(6).eax;
        if thermal & 1 == 0 {
            return Err(CpuError::NoSensor);
        }

        unsafe {
            let lib = load_pawnio_lib().ok_or(CpuError::PawnIoMissing)?;
            let proc = |name: &[u8]| GetProcAddress(lib, name.as_ptr());
            let (Some(open), Some(load), Some(execute), Some(close)) = (
                proc(b"pawnio_open\0"),
                proc(b"pawnio_load\0"),
                proc(b"pawnio_execute\0"),
                proc(b"pawnio_close\0"),
            ) else {
                FreeLibrary(lib);
                return Err(CpuError::PawnIoMissing);
            };
            let open: PawnOpen = transmute(open);
            let load: PawnLoad = transmute(load);
            let execute: PawnExecute = transmute(execute);
            let close: PawnClose = transmute(close);

            let mut handle = null_mut();
            let hr = open(&mut handle);
            if hr < 0 {
                FreeLibrary(lib);
                return Err(CpuError::PawnIoFailed(hr));
            }
            let hr = load(handle, INTEL_MSR_MODULE.as_ptr(), INTEL_MSR_MODULE.len());
            if hr < 0 {
                close(handle);
                FreeLibrary(lib);
                return Err(CpuError::PawnIoFailed(hr));
            }

            let mut cpu = Cpu {
                name: brand_string(),
                cores: Vec::new(),
                tjmax: 100,
                has_package: thermal & (1 << 6) != 0,
                lib,
                handle,
                execute,
                close,
            };
            cpu.cores = cpu.discover_cores();
            cpu.tjmax = cpu.cores.first().map_or(100, |c| c.tjmax);
            Ok(cpu)
        }
    }

    fn read_msr(&self, msr: u64) -> Option<u64> {
        let input = [msr];
        let mut out = [0u64; 1];
        let mut written = 0usize;
        let hr = unsafe {
            (self.execute)(
                self.handle,
                b"ioctl_read_msr\0".as_ptr(),
                input.as_ptr(),
                1,
                out.as_mut_ptr(),
                1,
                &mut written,
            )
        };
        (hr >= 0 && written >= 1).then_some(out[0])
    }

    fn discover_cores(&self) -> Vec<Core> {
        let mut topo = core_topology();
        // P-cores primeiro (maior classe de eficiência = mais desempenho); sort estável mantém a ordem física.
        topo.sort_by(|a, b| b.2.cmp(&a.2));
        let hybrid = topo.iter().any(|t| t.2 != topo[0].2);
        let perf_class = topo.first().map_or(0, |t| t.2);

        let mut pin = Pin::default();
        let (mut p, mut e, mut n) = (0, 0, 0);
        topo.into_iter()
            .map(|(group, mask, class)| {
                let (short, long) = if !hybrid {
                    n += 1;
                    (format!("C{n}"), format!("Núcleo {n}"))
                } else if class == perf_class {
                    p += 1;
                    (format!("P{p}"), format!("P-core {p}"))
                } else {
                    e += 1;
                    (format!("E{e}"), format!("E-core {e}"))
                };
                let tjmax = pin
                    .run_on(group, mask)
                    .then(|| self.read_msr(MSR_IA32_TEMPERATURE_TARGET))
                    .flatten()
                    .map(|v| ((v >> 16) & 0xFF) as i32)
                    .filter(|&t| t > 0)
                    .unwrap_or(100);
                Core { short, long, group, mask, tjmax }
            })
            .collect()
    }

    /// Lê todos os núcleos em `temps` (mesma ordem de `cores`) e devolve a temperatura do pacote.
    pub fn read(&self, temps: &mut Vec<Option<i32>>) -> Option<i32> {
        temps.clear();
        let mut pin = Pin::default();
        for core in &self.cores {
            let t = if pin.run_on(core.group, core.mask) {
                self.read_msr(MSR_IA32_THERM_STATUS).and_then(|v| decode(v, core.tjmax))
            } else {
                None
            };
            temps.push(t);
        }
        if self.has_package {
            self.read_msr(MSR_IA32_PACKAGE_THERM_STATUS).and_then(|v| decode(v, self.tjmax))
        } else {
            None
        }
    }
}

impl Drop for Cpu {
    fn drop(&mut self) {
        unsafe {
            (self.close)(self.handle);
            FreeLibrary(self.lib);
        }
    }
}

/// Bits 22:16 = graus abaixo do TjMax; bit 31 = leitura válida.
fn decode(v: u64, tjmax: i32) -> Option<i32> {
    (v & (1 << 31) != 0).then(|| tjmax - ((v >> 16) & 0x7F) as i32)
}

/// Fixa a thread atual num núcleo e restaura a afinidade original ao sair de escopo.
#[derive(Default)]
struct Pin {
    original: Option<GROUP_AFFINITY>,
}

impl Pin {
    fn run_on(&mut self, group: u16, mask: usize) -> bool {
        unsafe {
            let ga = GROUP_AFFINITY { Mask: mask, Group: group, Reserved: [0; 3] };
            let mut prev: GROUP_AFFINITY = zeroed();
            let ok = SetThreadGroupAffinity(GetCurrentThread(), &ga, &mut prev) != 0;
            if ok && self.original.is_none() {
                self.original = Some(prev);
            }
            ok
        }
    }
}

impl Drop for Pin {
    fn drop(&mut self) {
        if let Some(o) = self.original {
            unsafe { SetThreadGroupAffinity(GetCurrentThread(), &o, null_mut()) };
        }
    }
}

/// (grupo, máscara do primeiro processador lógico, classe de eficiência) de cada núcleo físico.
fn core_topology() -> Vec<(u16, usize, u8)> {
    let mut out = Vec::new();
    unsafe {
        let mut len = 0u32;
        GetLogicalProcessorInformationEx(RelationProcessorCore, null_mut(), &mut len);
        // Vec<u64> garante alinhamento de 8 bytes para as estruturas.
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let base = buf.as_mut_ptr() as *mut u8;
        if GetLogicalProcessorInformationEx(
            RelationProcessorCore,
            base as *mut SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
            &mut len,
        ) == 0
        {
            return out;
        }
        let mut offset = 0usize;
        while offset < len as usize {
            let info = &*(base.add(offset) as *const SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX);
            let core = &info.Anonymous.Processor;
            let ga = core.GroupMask[0];
            out.push((ga.Group, ga.Mask & ga.Mask.wrapping_neg(), core.EfficiencyClass));
            offset += info.Size as usize;
        }
    }
    out
}

fn brand_string() -> String {
    if __cpuid(0x8000_0000).eax < 0x8000_0004 {
        return "CPU Intel".into();
    }
    let mut bytes = Vec::with_capacity(48);
    for leaf in 0x8000_0002u32..=0x8000_0004 {
        let r = __cpuid(leaf);
        for reg in [r.eax, r.ebx, r.ecx, r.edx] {
            bytes.extend_from_slice(&reg.to_le_bytes());
        }
    }
    let s = String::from_utf8_lossy(&bytes).replace(' ', "");
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn load_pawnio_lib() -> Option<HMODULE> {
    let mut dirs = Vec::new();
    if let Some(d) = reg_string(
        HKEY_LOCAL_MACHINE,
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO",
        "InstallLocation",
    ) {
        dirs.push(d);
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        dirs.push(format!(r"{pf}\PawnIO"));
    }
    dirs.iter().find_map(|dir| {
        let path = wide(&format!(r"{}\PawnIOLib.dll", dir.trim_end_matches('\\')));
        let lib = unsafe { LoadLibraryExW(path.as_ptr(), null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH) };
        (!lib.is_null()).then_some(lib)
    })
}
