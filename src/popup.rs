//! Painel que abre com o clique esquerdo: temperatura atual, mínima e máxima de cada núcleo.

use std::mem::zeroed;

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC,
    DeleteObject, DrawTextW, EndPaint, FillRect, SelectObject, SetBkMode, SetTextColor,
    DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, HDC, HFONT,
    HGDIOBJ, PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

use crate::util::{colorref, fmt_temp, heat_color};

pub enum Kind {
    Title,
    Header,
    Temp,
    Note,
    Gap,
}

pub struct Row {
    pub kind: Kind,
    pub label: String,
    pub cur: Option<i32>,
    pub min: Option<i32>,
    pub max: Option<i32>,
}

impl Row {
    pub fn new(kind: Kind, label: impl Into<String>) -> Row {
        Row { kind, label: label.into(), cur: None, min: None, max: None }
    }

    pub fn temp(label: impl Into<String>, cur: Option<i32>, min: Option<i32>, max: Option<i32>) -> Row {
        Row { kind: Kind::Temp, label: label.into(), cur, min, max }
    }
}

pub struct Style {
    pub light: bool,
    pub font: HFONT,
    pub bold: HFONT,
    pub scale: f32,
    pub warn: i32,
    pub crit: i32,
}

fn px(v: f32, scale: f32) -> i32 {
    (v * scale).round() as i32
}

fn row_height(kind: &Kind, scale: f32) -> i32 {
    match kind {
        Kind::Gap => px(8.0, scale),
        Kind::Title => px(26.0, scale),
        _ => px(21.0, scale),
    }
}

pub fn size(rows: &[Row], scale: f32) -> (i32, i32) {
    let h: i32 = rows.iter().map(|r| row_height(&r.kind, scale)).sum();
    (px(300.0, scale), h + px(20.0, scale))
}

pub fn paint(hwnd: HWND, rows: &[Row], style: &Style) {
    unsafe {
        let mut ps: PAINTSTRUCT = zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let mut rc: RECT = zeroed();
        GetClientRect(hwnd, &mut rc);

        // Desenha num bitmap e copia de uma vez, para não piscar a cada atualização.
        let mem = CreateCompatibleDC(dc);
        let bmp = CreateCompatibleBitmap(dc, rc.right, rc.bottom);
        let old = SelectObject(mem, bmp as HGDIOBJ);

        let (bg, fg, dim) = if style.light {
            (0xF9F9F9, 0x1A1A1A, 0x6B6B6B)
        } else {
            (0x202020, 0xE8E8E8, 0x9A9A9A)
        };
        let brush = CreateSolidBrush(colorref(bg));
        FillRect(mem, &rc, brush);
        DeleteObject(brush as HGDIOBJ);
        SetBkMode(mem, TRANSPARENT as _);

        let s = style.scale;
        let pad = px(14.0, s);
        let col = px(44.0, s);
        let right = rc.right - pad;
        let cols = [right - 2 * col, right - col, right];
        let mut y = px(10.0, s);

        for row in rows {
            let h = row_height(&row.kind, s);
            let line = |font, color, text: &str, l, r, flags| {
                draw(mem, font, color, text, RECT { left: l, top: y, right: r, bottom: y + h }, flags)
            };
            match row.kind {
                Kind::Title => line(style.bold, fg, &row.label, pad, right, DT_LEFT | DT_END_ELLIPSIS),
                Kind::Header => {
                    line(style.font, dim, &row.label, pad, cols[0] - col, DT_LEFT);
                    for (i, t) in ["Atual", "Mín", "Máx"].iter().enumerate() {
                        line(style.font, dim, t, cols[i] - col, cols[i], DT_RIGHT);
                    }
                }
                Kind::Temp => {
                    line(style.font, fg, &row.label, pad, cols[0] - col, DT_LEFT | DT_END_ELLIPSIS);
                    for (i, v) in [row.cur, row.min, row.max].into_iter().enumerate() {
                        let color = heat_color(v, style.warn, style.crit, style.light, fg);
                        let font = if i == 0 { style.bold } else { style.font };
                        line(font, if v.is_none() { dim } else { color }, &fmt_temp(v), cols[i] - col, cols[i], DT_RIGHT);
                    }
                }
                Kind::Note => line(style.font, dim, &row.label, pad, right, DT_LEFT | DT_END_ELLIPSIS),
                Kind::Gap => {}
            }
            y += h;
        }

        BitBlt(dc, 0, 0, rc.right, rc.bottom, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old);
        DeleteObject(bmp as HGDIOBJ);
        DeleteDC(mem);
        EndPaint(hwnd, &ps);
    }
}

fn draw(dc: HDC, font: HFONT, rgb: u32, text: &str, mut rc: RECT, flags: u32) {
    // DrawTextW lê o ponteiro mesmo com tamanho 0, e um Vec vazio aponta para um endereço inválido.
    if text.is_empty() {
        return;
    }
    let t: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        SelectObject(dc, font as HGDIOBJ);
        SetTextColor(dc, colorref(rgb));
        DrawTextW(dc, t.as_ptr(), t.len() as i32, &mut rc, flags | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
    }
}

