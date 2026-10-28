//! Desenha números como ícones da bandeja (32 bpp com alpha, antialiasing em escala de cinza).

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;

use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject,
    GdiFlush, GetDC, GetDeviceCaps, GetTextFaceW, ReleaseDC, SelectObject, LOGPIXELSY, SetBkMode, SetTextColor, TextOutW, ANTIALIASED_QUALITY,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH,
    DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, OUT_TT_PRECIS, TRANSPARENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, GetSystemMetrics, HICON, ICONINFO, SM_CXSMICON,
};

use crate::util::wide;

/// Usadas se a fonte configurada não existir; as condensadas deixam os dígitos maiores no ícone.
const FONT_CANDIDATES: &[(&str, i32)] = &[
    ("Bahnschrift Condensed", 700),
    ("Bahnschrift SemiBold Condensed", 400),
    ("Segoe UI", 700),
];

#[derive(Clone, Copy)]
struct Fit {
    height: i32,
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
}

pub struct IconRenderer {
    pub size: i32,
    face: Vec<u16>,
    weight: i32,
    /// Altura inicial da fonte em pixels; reduzida só se o texto não couber no ícone.
    start_height: i32,
    /// Ajuste calculado por quantidade de dígitos, para que todos os números tenham o mesmo tamanho.
    fitted: [Option<Fit>; 5],
}

impl IconRenderer {
    /// `size_pt` = 0 faz os números preencherem o ícone inteiro.
    pub fn new(font: &str, size_pt: u32, bold: bool) -> Self {
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
        let (face, weight) = std::iter::once((font, if bold { 700 } else { 400 }))
            .chain(FONT_CANDIDATES.iter().copied())
            .find(|(f, _)| face_exists(f))
            .unwrap_or(FONT_CANDIDATES[FONT_CANDIDATES.len() - 1]);
        let start_height = if size_pt > 0 {
            let dpi = unsafe {
                let dc = GetDC(null_mut());
                let dpi = GetDeviceCaps(dc, LOGPIXELSY as _);
                ReleaseDC(null_mut(), dc);
                dpi
            };
            (size_pt as f32 * dpi as f32 / 72.0).round() as i32
        } else {
            size * 3 / 2
        };
        Self::with_font(size, face, weight, start_height)
    }

    pub fn with_font(size: i32, face: &str, weight: i32, start_height: i32) -> Self {
        IconRenderer { size, face: wide(face), weight, start_height, fitted: [None; 5] }
    }

    pub fn render(&mut self, text: &str, rgb: u32) -> HICON {
        let s = self.size;
        let pixels = self.render_pixels(text, rgb);
        unsafe {
            let color = Canvas::new(s, s);
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), color.bits, pixels.len());
            color.deselect();
            // A máscara é ignorada quando o bitmap colorido tem alpha, mas precisa existir.
            let mask_bits = vec![0u8; ((s as usize + 15) / 16 * 2) * s as usize];
            let mask = CreateBitmap(s, s, 1, 1, mask_bits.as_ptr() as *const c_void);
            let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color.bmp };
            let icon = CreateIconIndirect(&info);
            DeleteObject(mask as HGDIOBJ);
            icon
        }
    }

    /// Pixels BGRA pré-multiplicados (size × size, de cima para baixo).
    pub fn render_pixels(&mut self, text: &str, rgb: u32) -> Vec<u32> {
        let s = self.size;
        let mut out = vec![0u32; (s * s) as usize];
        let digits = !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
        let len = text.chars().count().min(4);
        let canvas = Canvas::new(s * 4, s * 3);

        let fit = match self.fitted[len] {
            Some(f) if digits => f,
            _ => {
                // Dígitos: calibra com "8"s, que ocupam a caixa inteira, e centraliza todos pela mesma caixa.
                let probe = if digits { "8".repeat(len) } else { text.to_string() };
                let f = self.fit(&canvas, &probe);
                if digits {
                    self.fitted[len] = Some(f);
                }
                f
            }
        };
        canvas.clear();
        canvas.draw(&self.face, self.weight, fit.height, text, s / 4);

        let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
        let (dx, dy) = ((s - fit.w) / 2, (s - fit.h) / 2);
        for y in 0..fit.h {
            for x in 0..fit.w {
                let a = canvas.coverage(fit.x0 + x, fit.y0 + y);
                if a == 0 {
                    continue;
                }
                let pm = |c: u32| c * a / 255;
                out[((dy + y) * s + dx + x) as usize] = (a << 24) | (pm(r) << 16) | (pm(g) << 8) | pm(b);
            }
        }
        out
    }

    fn fit(&self, canvas: &Canvas, probe: &str) -> Fit {
        let s = self.size;
        let mut height = self.start_height;
        loop {
            canvas.clear();
            canvas.draw(&self.face, self.weight, height, probe, s / 4);
            let (x0, y0, w, h) = canvas.ink_bounds();
            if (w <= s && h <= s) || height <= 6 {
                return Fit { height, x0, y0, w: w.min(s), h: h.min(s) };
            }
            height -= 1;
        }
    }
}

/// DIB de 32 bpp selecionado num DC de memória.
struct Canvas {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    w: i32,
    h: i32,
}

impl Canvas {
    fn new(w: i32, h: i32) -> Canvas {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let mut bmi: BITMAPINFO = zeroed();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // de cima para baixo
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..zeroed()
            };
            let mut bits = null_mut();
            let bmp = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            let old = SelectObject(dc, bmp as HGDIOBJ);
            Canvas { dc, bmp, old, bits: bits as *mut u32, w, h }
        }
    }

    fn clear(&self) {
        unsafe {
            GdiFlush();
            std::ptr::write_bytes(self.bits, 0, (self.w * self.h) as usize);
        }
    }

    fn draw(&self, face: &[u16], weight: i32, height: i32, text: &str, x: i32) {
        let t: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            let font = CreateFontW(
                -height,
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_TT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                ANTIALIASED_QUALITY as u32,
                DEFAULT_PITCH as u32,
                face.as_ptr(),
            );
            let old = SelectObject(self.dc, font as HGDIOBJ);
            SetBkMode(self.dc, TRANSPARENT as _);
            SetTextColor(self.dc, 0x00FF_FFFF);
            TextOutW(self.dc, x, 0, t.as_ptr(), t.len() as i32);
            SelectObject(self.dc, old);
            DeleteObject(font as HGDIOBJ);
            GdiFlush();
        }
    }

    fn coverage(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return 0;
        }
        let p = unsafe { *self.bits.add((y * self.w + x) as usize) };
        (p & 0xFF).max((p >> 8) & 0xFF).max((p >> 16) & 0xFF)
    }

    /// (x, y, largura, altura) da área com tinta.
    fn ink_bounds(&self) -> (i32, i32, i32, i32) {
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, -1, -1);
        for y in 0..self.h {
            for x in 0..self.w {
                if self.coverage(x, y) > 0 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        if x1 < 0 {
            return (0, 0, 0, 0);
        }
        (x0, y0, x1 - x0 + 1, y1 - y0 + 1)
    }

    fn deselect(&self) {
        unsafe { SelectObject(self.dc, self.old) };
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bmp as HGDIOBJ);
            DeleteDC(self.dc);
        }
    }
}

fn face_exists(face: &str) -> bool {
    let w = wide(face);
    unsafe {
        let dc = CreateCompatibleDC(null_mut());
        let font = CreateFontW(
            -12, 0, 0, 0, 400, 0, 0, 0,
            DEFAULT_CHARSET as u32, OUT_TT_PRECIS as u32, CLIP_DEFAULT_PRECIS as u32,
            ANTIALIASED_QUALITY as u32, DEFAULT_PITCH as u32, w.as_ptr(),
        );
        let old = SelectObject(dc, font as HGDIOBJ);
        let mut buf = [0u16; 64];
        GetTextFaceW(dc, buf.len() as i32, buf.as_mut_ptr());
        SelectObject(dc, old);
        DeleteObject(font as HGDIOBJ);
        DeleteDC(dc);
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len]).eq_ignore_ascii_case(face)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gera uma prévia ampliada dos ícones sobre o fundo da barra de tarefas:
    /// `cargo test preview -- --ignored` com TRAYTEMP_PREVIEW=<arquivo.bmp>
    #[test]
    #[ignore]
    fn preview() {
        let out = std::env::var("TRAYTEMP_PREVIEW").expect("TRAYTEMP_PREVIEW");
        let size: i32 = std::env::var("TRAYTEMP_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(20);
        let samples = [("45", 0xFFFFFF), ("72", 0xFFFFFF), ("88", 0xFFB340), ("97", 0xFF6B6B), ("100", 0xFF6B6B), ("61", 0x9BE15D), ("--", 0x8A8A8A), ("!", 0xFFB340)];
        let fonts: Vec<(&str, i32, i32)> = vec![
            ("Trebuchet MS", 400, 11),
            ("Trebuchet MS", 700, 11),
            ("Bahnschrift Condensed", 700, size * 3 / 2),
        ];
        let (zoom, gap) = (8, 4);
        let cell = size + gap;
        let (w, h) = (cell * samples.len() as i32 * zoom, cell * fonts.len() as i32 * zoom);
        let bg = 0x1C1C1Cu32;
        let mut img = vec![bg; (w * h) as usize];
        for (row, (face, weight, start)) in fonts.iter().enumerate() {
            let mut r = IconRenderer::with_font(size, face, *weight, *start);
            for (col, (text, rgb)) in samples.iter().enumerate() {
                let px = r.render_pixels(text, *rgb);
                for y in 0..size {
                    for x in 0..size {
                        let p = px[(y * size + x) as usize];
                        let a = p >> 24;
                        let blend = |sh: u32| ((p >> sh) & 0xFF) + ((bg >> sh) & 0xFF) * (255 - a) / 255;
                        let c = (blend(16) << 16) | (blend(8) << 8) | blend(0);
                        for zy in 0..zoom {
                            for zx in 0..zoom {
                                let ix = (col as i32 * cell + x) * zoom + zx;
                                let iy = (row as i32 * cell + y) * zoom + zy;
                                img[(iy * w + ix) as usize] = c;
                            }
                        }
                    }
                }
            }
            println!("linha {row}: {face} peso {weight} altura {start}px");
        }
        let mut bmp = Vec::new();
        let data = (w * h * 4) as u32;
        bmp.extend_from_slice(b"BM");
        for v in [54 + data, 0, 54, 40] { bmp.extend_from_slice(&v.to_le_bytes()); }
        bmp.extend_from_slice(&w.to_le_bytes());
        bmp.extend_from_slice(&(-h).to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&32u16.to_le_bytes());
        for v in [0u32, data, 2835, 2835, 0, 0] { bmp.extend_from_slice(&v.to_le_bytes()); }
        for p in img { bmp.extend_from_slice(&p.to_le_bytes()); }
        std::fs::write(out, bmp).unwrap();
    }
}
