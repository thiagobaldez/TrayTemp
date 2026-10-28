//! Temperatura da GPU dedicada pela interface WDDM (a mesma do Gerenciador de Tarefas),
//! sem carregar bibliotecas do fabricante e sem acordar a GPU de notebooks quando ela está em repouso.

use std::ffi::c_void;
use std::mem::{size_of, zeroed};

use windows_sys::core::GUID;
use windows_sys::Wdk::Graphics::Direct3D::{
    D3DKMTCloseAdapter, D3DKMTEnumAdapters2, D3DKMTOpenAdapterFromLuid, D3DKMTQueryAdapterInfo,
    D3DKMT_ADAPTERINFO, D3DKMT_ADAPTER_PERFDATA, D3DKMT_CLOSEADAPTER, D3DKMT_ENUMADAPTERS2,
    D3DKMT_OPENADAPTERFROMLUID, D3DKMT_QUERYADAPTERINFO, D3DKMT_QUERY_DEVICE_IDS,
    KMTQAITYPE_ADAPTERPERFDATA, KMTQAITYPE_PHYSICALADAPTERDEVICEIDS, KMTQUERYADAPTERINFOTYPE,
};
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_PropertyW, CM_Get_Device_ID_ListW, CM_Get_Device_ID_List_SizeW,
    CM_Locate_DevNodeW, CM_GETIDLIST_FILTER_CLASS, CM_GETIDLIST_FILTER_PRESENT,
    CM_LOCATE_DEVNODE_NORMAL, CR_SUCCESS,
};
use windows_sys::Win32::Devices::Properties::DEVPROPKEY;
use windows_sys::Win32::Foundation::LUID;

use crate::util::wide;

const DEVPKEY_DEVICE_POWERDATA: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 32,
};
const DEVPKEY_DEVICE_FRIENDLYNAME: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};
const DEVPKEY_DEVICE_DEVICEDESC: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 2,
};
const DISPLAY_CLASS: &str = "{4d36e968-e325-11ce-bfc1-08002be10318}";
const POWER_DEVICE_D0: u32 = 1;
/// Fabricantes de GPUs dedicadas: NVIDIA e AMD.
const DISCRETE_VENDORS: [u32; 2] = [0x10DE, 0x1002];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GpuReading {
    Temp(i32),
    /// GPU em D3 (desligada pelo Optimus/RTD3): não consultamos para não acordá-la.
    Sleeping,
    Unavailable,
}

pub struct Gpu {
    pub name: String,
    luid: LUID,
    /// Nó PnP da placa, usado para checar o estado de energia; 0 se não encontrado.
    devinst: u32,
}

impl Gpu {
    /// Localiza a primeira GPU dedicada (NVIDIA ou AMD).
    pub fn find() -> Option<Gpu> {
        unsafe {
            let mut adapters: [D3DKMT_ADAPTERINFO; 16] = zeroed();
            let mut e = D3DKMT_ENUMADAPTERS2 { NumAdapters: adapters.len() as u32, pAdapters: adapters.as_mut_ptr() };
            if D3DKMTEnumAdapters2(&mut e) < 0 {
                return None;
            }
            let mut found = None;
            for a in &adapters[..e.NumAdapters as usize] {
                let mut ids = D3DKMT_QUERY_DEVICE_IDS { PhysicalAdapterIndex: 0, ..zeroed() };
                if found.is_none()
                    && query(a.hAdapter, KMTQAITYPE_PHYSICALADAPTERDEVICEIDS, &mut ids)
                    && DISCRETE_VENDORS.contains(&ids.DeviceIds.VendorID)
                {
                    found = Some((a.AdapterLuid, ids.DeviceIds.VendorID, ids.DeviceIds.DeviceID));
                }
                D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER { hAdapter: a.hAdapter });
            }
            let (luid, vendor, device) = found?;
            let devinst = find_devnode(vendor, device).unwrap_or(0);
            let name = (devinst != 0)
                .then(|| {
                    devnode_string(devinst, &DEVPKEY_DEVICE_FRIENDLYNAME)
                        .or_else(|| devnode_string(devinst, &DEVPKEY_DEVICE_DEVICEDESC))
                })
                .flatten()
                // Descrições vindas do INF podem ter o formato "@oemXX.inf,%chave%;Nome".
                .map(|n| n.rsplit(';').next().unwrap_or(&n).to_string())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| "GPU".into());
            Some(Gpu { name, luid, devinst })
        }
    }

    /// Consulta o estado de energia mantido pelo PnP; isso não toca no hardware.
    pub fn is_awake(&self) -> bool {
        if self.devinst == 0 {
            return true;
        }
        // CM_POWER_DATA: PD_Size, PD_MostRecentPowerState, ...
        let mut data = [0u32; 14];
        let mut size = size_of::<[u32; 14]>() as u32;
        let mut ty = 0u32;
        let cr = unsafe {
            CM_Get_DevNode_PropertyW(
                self.devinst,
                &DEVPKEY_DEVICE_POWERDATA,
                &mut ty,
                data.as_mut_ptr() as *mut u8,
                &mut size,
                0,
            )
        };
        // Sem a informação, assume ligada para não deixar de mostrar a temperatura.
        cr != CR_SUCCESS || data[1] == POWER_DEVICE_D0
    }

    pub fn read(&mut self) -> GpuReading {
        if !self.is_awake() {
            return GpuReading::Sleeping;
        }
        unsafe {
            let mut open = D3DKMT_OPENADAPTERFROMLUID { AdapterLuid: self.luid, hAdapter: 0 };
            if D3DKMTOpenAdapterFromLuid(&mut open) < 0 {
                return GpuReading::Unavailable;
            }
            let mut perf = D3DKMT_ADAPTER_PERFDATA { PhysicalAdapterIndex: 0, ..zeroed() };
            let ok = query(open.hAdapter, KMTQAITYPE_ADAPTERPERFDATA, &mut perf);
            D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER { hAdapter: open.hAdapter });
            // Temperature vem em décimos de grau; 0 = driver não informa.
            if ok && perf.Temperature > 0 {
                GpuReading::Temp(((perf.Temperature + 5) / 10) as i32)
            } else {
                GpuReading::Unavailable
            }
        }
    }
}

fn query<T>(adapter: u32, ty: KMTQUERYADAPTERINFOTYPE, data: &mut T) -> bool {
    let mut q = D3DKMT_QUERYADAPTERINFO {
        hAdapter: adapter,
        Type: ty,
        pPrivateDriverData: data as *mut T as *mut c_void,
        PrivateDriverDataSize: size_of::<T>() as u32,
    };
    unsafe { D3DKMTQueryAdapterInfo(&mut q) >= 0 }
}

/// Nó PnP da placa de vídeo com este fabricante/dispositivo.
fn find_devnode(vendor: u32, device: u32) -> Option<u32> {
    let filter = wide(DISPLAY_CLASS);
    let flags = CM_GETIDLIST_FILTER_CLASS | CM_GETIDLIST_FILTER_PRESENT;
    let pattern = format!("VEN_{vendor:04X}&DEV_{device:04X}");
    unsafe {
        let mut len = 0u32;
        if CM_Get_Device_ID_List_SizeW(&mut len, filter.as_ptr(), flags) != CR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u16; len as usize];
        if CM_Get_Device_ID_ListW(filter.as_ptr(), buf.as_mut_ptr(), len, flags) != CR_SUCCESS {
            return None;
        }
        let id = buf
            .split(|&c| c == 0)
            .map(String::from_utf16_lossy)
            .find(|id| id.to_ascii_uppercase().contains(&pattern))?;
        let id = wide(&id);
        let mut devinst = 0u32;
        (CM_Locate_DevNodeW(&mut devinst, id.as_ptr(), CM_LOCATE_DEVNODE_NORMAL) == CR_SUCCESS).then_some(devinst)
    }
}

fn devnode_string(devinst: u32, key: &DEVPROPKEY) -> Option<String> {
    let mut buf = [0u16; 256];
    let mut size = (buf.len() * 2) as u32;
    let mut ty = 0u32;
    let cr = unsafe {
        CM_Get_DevNode_PropertyW(devinst, key, &mut ty, buf.as_mut_ptr() as *mut u8, &mut size, 0)
    };
    if cr != CR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}
