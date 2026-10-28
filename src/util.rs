use std::ffi::c_void;
use std::ptr::null_mut;

use windows_sys::Win32::System::Registry::{
    RegGetValueW, HKEY, HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Copia `s` para um buffer UTF-16 de tamanho fixo, truncando e terminando com NUL.
pub fn copy_wstr(dst: &mut [u16], s: &str) {
    let mut i = 0;
    for c in s.encode_utf16() {
        if i + 1 >= dst.len() {
            break;
        }
        dst[i] = c;
        i += 1;
    }
    dst[i] = 0;
}

pub fn reg_dword(root: HKEY, key: &str, value: &str) -> Option<u32> {
    let (key, value) = (wide(key), wide(value));
    let mut data = 0u32;
    let mut size = 4u32;
    let status = unsafe {
        RegGetValueW(
            root,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            &mut data as *mut u32 as *mut c_void,
            &mut size,
        )
    };
    (status == 0).then_some(data)
}

pub fn reg_string(root: HKEY, key: &str, value: &str) -> Option<String> {
    let (key, value) = (wide(key), wide(value));
    let mut buf = [0u16; 512];
    let mut size = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            root,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// `SystemUsesLightTheme` vale para a barra de tarefas; `AppsUseLightTheme` para janelas.
pub fn uses_light_theme(value: &str) -> bool {
    reg_dword(
        HKEY_CURRENT_USER,
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        value,
    )
    .is_some_and(|v| v != 0)
}

/// Cor de uma temperatura (0xRRGGBB): base abaixo do alerta, laranja no alerta, vermelho no crítico.
pub fn heat_color(temp: Option<i32>, warn: i32, crit: i32, light: bool, base: u32) -> u32 {
    match temp {
        None => 0x8A8A8A,
        Some(t) if t >= crit => if light { 0xC42B1C } else { 0xFF6B6B },
        Some(t) if t >= warn => if light { 0xB45F00 } else { 0xFFB340 },
        Some(_) => base,
    }
}

/// 0xRRGGBB -> COLORREF (0x00BBGGRR)
pub fn colorref(rgb: u32) -> u32 {
    ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF)
}

pub fn fmt_temp(t: Option<i32>) -> String {
    t.map_or_else(|| "—".to_string(), |t| format!("{t}°"))
}
