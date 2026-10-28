#![windows_subsystem = "windows"]

mod autostart;
mod config;
mod cpu;
mod gpu;
mod icon;
mod popup;
mod util;

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::fmt::Write as _;
use std::mem::{size_of, transmute, zeroed};
use std::os::windows::process::CommandExt;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, GetDC, GetDeviceCaps, GetMonitorInfoW, InvalidateRect, MonitorFromPoint, ReleaseDC,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, HFONT, LOGPIXELSY,
    MONITORINFO, MONITOR_DEFAULTTONEAREST, OUT_TT_PRECIS,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::System::Threading::{CreateMutexW, OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, ChangeWindowMessageFilterEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DispatchMessageW, FindWindowW, GetMessageW, GetWindowThreadProcessId,
    IsWindowVisible, PostMessageW,
    PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SetForegroundWindow, SetMenuDefaultItem,
    SetTimer, SetWindowPos, ShowWindow, TrackPopupMenu, TranslateMessage, HICON, HMENU, HWND_TOPMOST,
    MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MSG, MSGFLT_ALLOW, SWP_SHOWWINDOW, SW_HIDE,
    TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTALIGN, TPM_RIGHTBUTTON, WA_INACTIVE, WM_ACTIVATE, WM_APP,
    WM_CLOSE, WM_CONTEXTMENU, WM_ERASEBKGND, WM_KEYDOWN, WM_NULL, WM_PAINT, WM_SETTINGCHANGE, WM_TIMER, WM_USER,
    WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_OVERLAPPED, WS_POPUP,
};

use config::{Config, CpuMode};
use cpu::{Cpu, CpuError};
use gpu::{Gpu, GpuReading};
use icon::IconRenderer;
use popup::{Kind, Row};
use util::{copy_wstr, fmt_temp, heat_color, uses_light_theme, wide};

const WM_TRAY: u32 = WM_APP + 1;
const NIN_SELECT: u32 = WM_USER;
const NIN_KEYSELECT: u32 = WM_USER + 1;
const VK_ESCAPE: usize = 0x1B;
const TIMER_ID: usize = 1;

const ID_CPU: u32 = 1;
const ID_GPU: u32 = 2;
const ID_CORE_BASE: u32 = 100;

const CMD_PANEL: usize = 1000;
const CMD_GPU: usize = 1010;
const CMD_RESET: usize = 1030;
const CMD_AUTOSTART: usize = 1040;
const CMD_PAWNIO: usize = 1050;
const CMD_RETRY: usize = 1051;
const CMD_EXIT: usize = 1099;

const MODES: [(usize, CpuMode, &str); 4] = [
    (1001, CpuMode::Max, "Temperatura máxima entre os núcleos"),
    (1002, CpuMode::Average, "Média dos núcleos"),
    (1003, CpuMode::Package, "Sensor do pacote (CPU inteira)"),
    (1004, CpuMode::PerCore, "Um ícone para cada núcleo"),
];
const INTERVALS: [(usize, u32, &str); 3] = [
    (1021, 1000, "1 segundo"),
    (1022, 2000, "2 segundos"),
    (1023, 5000, "5 segundos"),
];

#[derive(Default, Clone, Copy)]
struct Stat {
    cur: Option<i32>,
    min: Option<i32>,
    max: Option<i32>,
}

impl Stat {
    fn push(&mut self, v: Option<i32>) {
        self.cur = v;
        if let Some(t) = v {
            self.min = Some(self.min.map_or(t, |m| m.min(t)));
            self.max = Some(self.max.map_or(t, |m| m.max(t)));
        }
    }

    fn reset(&mut self) {
        self.min = self.cur;
        self.max = self.cur;
    }

    fn row(&self, label: &str) -> Row {
        Row::temp(label, self.cur, self.min, self.max)
    }
}

#[derive(Clone)]
struct TrayIcon {
    id: u32,
    text: String,
    color: u32,
    tip: String,
    added: bool,
}

struct App {
    hwnd: HWND,
    popup: HWND,
    cfg: Config,
    cpu: Result<Cpu, CpuError>,
    gpu: Option<Gpu>,
    temps: Vec<Option<i32>>,
    cores: Vec<Stat>,
    package: Stat,
    gpu_stat: Stat,
    gpu_reading: GpuReading,
    icons: Vec<TrayIcon>,
    renderer: IconRenderer,
    taskbar_light: bool,
    apps_light: bool,
    autostart: bool,
    scale: f32,
    font: HFONT,
    bold: HFONT,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static POPUP_HIDDEN_AT: Cell<u32> = const { Cell::new(0) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

/// Acesso ao estado global; devolve None em reentrância (mensagens enviadas durante outra chamada).
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| {
        let mut guard = cell.try_borrow_mut().ok()?;
        guard.as_mut().map(f)
    })
}

fn temp_text(t: Option<i32>) -> String {
    t.map_or_else(|| "--".to_string(), |t| t.to_string())
}

fn temp_c(t: Option<i32>) -> String {
    t.map_or_else(|| "—".to_string(), |t| format!("{t} °C"))
}

impl App {
    fn new(hwnd: HWND, popup: HWND) -> App {
        let cfg = Config::load();
        let cpu = Cpu::open();
        let n = cpu.as_ref().map_or(0, |c| c.cores.len());
        let (scale, font, bold) = popup_fonts();
        App {
            hwnd,
            popup,
            renderer: IconRenderer::new(&cfg.font, cfg.font_size, cfg.font_bold),
            cfg,
            cpu,
            gpu: Gpu::find(),
            temps: Vec::with_capacity(n),
            cores: vec![Stat::default(); n],
            package: Stat::default(),
            gpu_stat: Stat::default(),
            gpu_reading: GpuReading::Unavailable,
            icons: Vec::new(),
            taskbar_light: uses_light_theme("SystemUsesLightTheme"),
            apps_light: uses_light_theme("AppsUseLightTheme"),
            autostart: autostart::is_enabled(),
            scale,
            font,
            bold,
        }
    }

    fn update(&mut self) {
        if let Ok(cpu) = &self.cpu {
            self.package.push(cpu.read(&mut self.temps));
            for (s, t) in self.cores.iter_mut().zip(&self.temps) {
                s.push(*t);
            }
        }
        if self.cfg.show_gpu {
            if let Some(gpu) = &mut self.gpu {
                self.gpu_reading = gpu.read();
                self.gpu_stat.push(match self.gpu_reading {
                    GpuReading::Temp(t) => Some(t),
                    _ => None,
                });
            }
        }
        self.sync_icons();
        unsafe {
            if IsWindowVisible(self.popup) != 0 {
                InvalidateRect(self.popup, null(), 0);
            }
        }
    }

    /// (máxima, média) dos núcleos na última leitura.
    fn cpu_summary(&self) -> (Option<i32>, Option<i32>) {
        let max = self.temps.iter().flatten().max().copied();
        let (sum, n) = self.temps.iter().flatten().fold((0, 0), |(s, n), t| (s + t, n + 1));
        let avg = (n > 0).then(|| (sum as f32 / n as f32).round() as i32);
        (max, avg)
    }

    fn color(&self, t: Option<i32>, gpu: bool) -> u32 {
        let base = match (gpu, self.taskbar_light) {
            (false, false) => 0xFFFFFF,
            (false, true) => 0x000000,
            (true, false) => 0x9BE15D,
            (true, true) => 0x3A7D00,
        };
        heat_color(t, self.cfg.warn, self.cfg.crit, self.taskbar_light, base)
    }

    fn cpu_tooltip(&self, cpu: &Cpu, value: Option<i32>) -> String {
        let (max, avg) = self.cpu_summary();
        let mut tip = format!("CPU {} ({})\n", temp_c(value), self.cfg.cpu_mode.label());
        if cpu.has_package {
            let _ = write!(tip, "Pacote {} · ", temp_c(self.package.cur));
        }
        let _ = write!(tip, "Máx {} · Média {}", temp_c(max), temp_c(avg));
        // Uma linha por tipo de núcleo: "P: 62 60 66 …"
        let mut prefix = None;
        for (core, t) in cpu.cores.iter().zip(&self.temps) {
            let p = core.short.chars().next();
            if p != prefix {
                prefix = p;
                let _ = write!(tip, "\n{}:", p.unwrap_or('C'));
            }
            let _ = write!(tip, " {}", temp_text(*t));
        }
        tip
    }

    fn desired_icons(&self) -> Vec<TrayIcon> {
        let icon = |id, text: String, color, tip: String| TrayIcon { id, text, color, tip, added: false };
        let mut out = Vec::new();
        match &self.cpu {
            Err(e) => out.push(icon(
                ID_CPU,
                "!".into(),
                heat_color(Some(self.cfg.warn), self.cfg.warn, self.cfg.crit, self.taskbar_light, 0),
                format!("TrayTemp: {}\nClique com o botão direito para opções", e.message()),
            )),
            Ok(cpu) if self.cfg.cpu_mode == CpuMode::PerCore => {
                for (i, (core, t)) in cpu.cores.iter().zip(&self.temps).enumerate() {
                    out.push(icon(
                        ID_CORE_BASE + i as u32,
                        temp_text(*t),
                        self.color(*t, false),
                        format!("CPU {}: {}", core.long, temp_c(*t)),
                    ));
                }
            }
            Ok(cpu) => {
                let (max, avg) = self.cpu_summary();
                let value = match self.cfg.cpu_mode {
                    CpuMode::Average => avg,
                    CpuMode::Package => self.package.cur.or(max),
                    _ => max,
                };
                out.push(icon(ID_CPU, temp_text(value), self.color(value, false), self.cpu_tooltip(cpu, value)));
            }
        }
        if let (Some(gpu), true) = (&self.gpu, self.cfg.show_gpu) {
            let (t, tip) = match self.gpu_reading {
                GpuReading::Temp(t) => (Some(t), format!("GPU {}\n{}", temp_c(Some(t)), gpu.name)),
                GpuReading::Sleeping => (None, format!("GPU em repouso (não é consultada para não acordá-la)\n{}", gpu.name)),
                GpuReading::Unavailable => (None, format!("GPU: leitura indisponível\n{}", gpu.name)),
            };
            out.push(icon(ID_GPU, temp_text(t), self.color(t, true), tip));
        }
        out
    }

    fn sync_icons(&mut self) {
        let desired = self.desired_icons();
        let hwnd = self.hwnd;
        self.icons.retain(|ic| {
            let keep = desired.iter().any(|d| d.id == ic.id);
            if !keep && ic.added {
                notify(NIM_DELETE, hwnd, ic.id, None, "");
            }
            keep
        });

        for want in desired {
            let pos = self.icons.iter().position(|i| i.id == want.id);
            if let Some(cur) = pos.map(|i| &mut self.icons[i]).filter(|c| c.added) {
                let redraw = cur.text != want.text || cur.color != want.color;
                if !redraw && cur.tip == want.tip {
                    continue;
                }
                let hicon = redraw.then(|| self.renderer.render(&want.text, want.color));
                notify(NIM_MODIFY, hwnd, want.id, hicon, &want.tip);
                if let Some(h) = hicon {
                    unsafe { DestroyIcon(h) };
                }
                *cur = TrayIcon { added: true, ..want };
                continue;
            }

            // Ícone novo (ou a barra de tarefas ainda não estava pronta da última vez).
            let hicon = self.renderer.render(&want.text, want.color);
            let added = notify(NIM_ADD, hwnd, want.id, Some(hicon), &want.tip)
                || notify(NIM_MODIFY, hwnd, want.id, Some(hicon), &want.tip);
            if added {
                notify(NIM_SETVERSION, hwnd, want.id, None, "");
            }
            unsafe { DestroyIcon(hicon) };
            let entry = TrayIcon { added, ..want };
            match pos {
                Some(i) => self.icons[i] = entry,
                None => self.icons.push(entry),
            }
        }
    }

    fn remove_icons(&mut self) {
        for ic in self.icons.drain(..).filter(|i| i.added) {
            notify(NIM_DELETE, self.hwnd, ic.id, None, "");
        }
    }

    fn refresh_theme(&mut self) {
        self.taskbar_light = uses_light_theme("SystemUsesLightTheme");
        self.apps_light = uses_light_theme("AppsUseLightTheme");
        set_popup_frame(self.popup, self.apps_light);
        self.sync_icons();
    }

    fn popup_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        match &self.cpu {
            Ok(cpu) => {
                rows.push(Row::new(Kind::Title, cpu.name.as_str()));
                rows.push(Row::new(Kind::Header, ""));
                if cpu.has_package {
                    rows.push(self.package.row("Pacote"));
                }
                for (core, stat) in cpu.cores.iter().zip(&self.cores) {
                    rows.push(stat.row(&core.long));
                }
                let (max, avg) = self.cpu_summary();
                rows.push(Row::new(
                    Kind::Note,
                    format!("Máxima {} · Média {} · TjMax {}°", fmt_temp(max), fmt_temp(avg), cpu.tjmax),
                ));
            }
            Err(e) => {
                rows.push(Row::new(Kind::Title, "CPU"));
                rows.push(Row::new(Kind::Note, e.message()));
            }
        }
        if let Some(gpu) = &self.gpu {
            rows.push(Row::new(Kind::Gap, ""));
            rows.push(Row::new(Kind::Title, gpu.name.as_str()));
            if self.cpu.is_err() {
                rows.push(Row::new(Kind::Header, ""));
            }
            rows.push(match (self.cfg.show_gpu, self.gpu_reading) {
                (false, _) => Row::new(Kind::Note, "Monitoramento da GPU desativado"),
                (_, GpuReading::Sleeping) => Row::new(Kind::Note, "Em repouso: leitura pausada para não acordá-la"),
                (_, GpuReading::Unavailable) => Row::new(Kind::Note, "Leitura indisponível"),
                (_, GpuReading::Temp(_)) => self.gpu_stat.row("GPU"),
            });
        }
        rows
    }

    fn popup_style(&self) -> popup::Style {
        popup::Style {
            light: self.apps_light,
            font: self.font,
            bold: self.bold,
            scale: self.scale,
            warn: self.cfg.warn,
            crit: self.cfg.crit,
        }
    }

    /// Posição do painel junto ao ponto clicado, dentro da área de trabalho do monitor.
    fn popup_rect(&self, x: i32, y: i32) -> RECT {
        let (w, h) = popup::size(&self.popup_rows(), self.scale);
        let margin = (12.0 * self.scale) as i32;
        let work = unsafe {
            let mut mi: MONITORINFO = zeroed();
            mi.cbSize = size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST), &mut mi);
            mi.rcWork
        };
        let left = (x - w / 2).clamp(work.left + margin, (work.right - w - margin).max(work.left));
        let top = (y - h - margin).clamp(work.top + margin, (work.bottom - h - margin).max(work.top));
        RECT { left, top, right: left + w, bottom: top + h }
    }

    fn build_menu(&self) -> HMENU {
        unsafe {
            let item = |menu: HMENU, id: usize, text: &str, checked: bool, enabled: bool| {
                let flags = MF_STRING | if checked { MF_CHECKED } else { 0 } | if enabled { 0 } else { MF_GRAYED };
                AppendMenuW(menu, flags, id, wide(text).as_ptr());
            };
            let submenu = |menu: HMENU, sub: HMENU, text: &str| {
                AppendMenuW(menu, MF_POPUP, sub as usize, wide(text).as_ptr());
            };
            let separator = |menu: HMENU| {
                AppendMenuW(menu, MF_SEPARATOR, 0, null());
            };

            let menu = CreatePopupMenu();
            item(menu, CMD_PANEL, "Abrir painel de temperaturas", false, true);
            SetMenuDefaultItem(menu, CMD_PANEL as u32, 0);
            separator(menu);

            match &self.cpu {
                Ok(cpu) => {
                    let sub = CreatePopupMenu();
                    for (cmd, mode, label) in MODES {
                        let available = mode != CpuMode::Package || cpu.has_package;
                        item(sub, cmd, label, self.cfg.cpu_mode == mode, available);
                    }
                    submenu(menu, sub, "CPU na bandeja");
                }
                Err(e) => {
                    item(menu, 0, &e.message(), false, false);
                    if e.needs_pawnio() {
                        item(menu, CMD_PAWNIO, "Baixar o driver PawnIO (pawnio.eu)…", false, true);
                    }
                    item(menu, CMD_RETRY, "Tentar novamente", false, true);
                }
            }
            if self.gpu.is_some() {
                item(menu, CMD_GPU, "Mostrar temperatura da GPU", self.cfg.show_gpu, true);
            }
            let sub = CreatePopupMenu();
            for (cmd, ms, label) in INTERVALS {
                item(sub, cmd, label, self.cfg.interval_ms == ms, true);
            }
            submenu(menu, sub, "Atualizar a cada");
            item(menu, CMD_RESET, "Zerar mínimas e máximas", false, self.cpu.is_ok() || self.gpu.is_some());
            separator(menu);
            item(menu, CMD_AUTOSTART, "Iniciar com o Windows", self.autostart, true);
            separator(menu);
            item(menu, CMD_EXIT, "Sair", false, true);
            menu
        }
    }

    /// Executa um comando do menu; devolve true para encerrar o app.
    fn command(&mut self, cmd: usize) -> bool {
        match cmd {
            CMD_EXIT => return true,
            CMD_RESET => {
                self.cores.iter_mut().for_each(Stat::reset);
                self.package.reset();
                self.gpu_stat.reset();
            }
            CMD_GPU => {
                self.cfg.show_gpu = !self.cfg.show_gpu;
                self.cfg.save();
            }
            CMD_AUTOSTART => {
                if self.autostart {
                    autostart::disable();
                } else {
                    autostart::enable();
                }
                self.autostart = autostart::is_enabled();
            }
            CMD_PAWNIO => {
                // Via explorer.exe para o navegador abrir sem herdar a elevação deste processo.
                let explorer = format!(r"{}\explorer.exe", std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()));
                let _ = std::process::Command::new(explorer).arg("https://pawnio.eu/").creation_flags(0x0800_0000).spawn();
            }
            CMD_RETRY => {
                self.cpu = Cpu::open();
                let n = self.cpu.as_ref().map_or(0, |c| c.cores.len());
                self.cores = vec![Stat::default(); n];
                self.package = Stat::default();
            }
            _ => {
                if let Some(&(_, mode, _)) = MODES.iter().find(|m| m.0 == cmd) {
                    self.cfg.cpu_mode = mode;
                    self.cfg.save();
                } else if let Some(&(_, ms, _)) = INTERVALS.iter().find(|i| i.0 == cmd) {
                    self.cfg.interval_ms = ms;
                    self.cfg.save();
                    unsafe { SetTimer(self.hwnd, TIMER_ID, ms, None) };
                }
            }
        }
        self.update();
        false
    }
}

fn notify(op: u32, hwnd: HWND, id: u32, icon: Option<HICON>, tip: &str) -> bool {
    unsafe {
        let mut nid: NOTIFYICONDATAW = zeroed();
        nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = id;
        match op {
            NIM_SETVERSION => nid.Anonymous.uVersion = NOTIFYICON_VERSION_4,
            NIM_DELETE => {}
            _ => {
                nid.uFlags = NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
                nid.uCallbackMessage = WM_TRAY;
                copy_wstr(&mut nid.szTip, tip);
                if let Some(h) = icon {
                    nid.uFlags |= NIF_ICON;
                    nid.hIcon = h;
                }
            }
        }
        Shell_NotifyIconW(op, &nid) != 0
    }
}

fn show_menu(hwnd: HWND, x: i32, y: i32) {
    let Some(menu) = with_app(|a| a.build_menu()) else { return };
    let cmd = unsafe {
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_RIGHTALIGN | TPM_BOTTOMALIGN, x, y, 0, hwnd, null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        cmd as usize
    };
    match cmd {
        0 => {}
        CMD_PANEL => show_popup(x, y),
        _ => {
            if with_app(|a| a.command(cmd)) == Some(true) {
                unsafe { PostQuitMessage(0) };
            }
        }
    }
}

fn show_popup(x: i32, y: i32) {
    // Clicar no ícone com o painel aberto fecha o painel (ele perde o foco antes do clique chegar).
    if unsafe { GetTickCount() }.wrapping_sub(POPUP_HIDDEN_AT.get()) < 300 {
        return;
    }
    let Some((popup, rc)) = with_app(|a| (a.popup, a.popup_rect(x, y))) else { return };
    unsafe {
        SetWindowPos(popup, HWND_TOPMOST, rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top, SWP_SHOWWINDOW);
        SetForegroundWindow(popup);
        InvalidateRect(popup, null(), 0);
    }
}

fn hide_popup(hwnd: HWND) {
    unsafe { ShowWindow(hwnd, SW_HIDE) };
    POPUP_HIDDEN_AT.set(unsafe { GetTickCount() });
}

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_TRAY {
        // NOTIFYICON_VERSION_4: evento em LOWORD(lParam), coordenadas em wParam.
        let x = (wp & 0xFFFF) as u16 as i16 as i32;
        let y = ((wp >> 16) & 0xFFFF) as u16 as i16 as i32;
        match (lp as u32) & 0xFFFF {
            WM_CONTEXTMENU => show_menu(hwnd, x, y),
            NIN_SELECT | NIN_KEYSELECT => show_popup(x, y),
            _ => {}
        }
        return 0;
    }
    if msg == TASKBAR_CREATED.get() && msg != 0 {
        // O Explorer reiniciou: todos os ícones precisam ser adicionados de novo.
        with_app(|a| {
            a.icons.iter_mut().for_each(|i| i.added = false);
            a.sync_icons();
        });
        return 0;
    }
    match msg {
        WM_TIMER => {
            with_app(|a| a.update());
            0
        }
        // Enviado por `TrayTemp.exe --exit` (instalador/desinstalador).
        WM_CLOSE => {
            PostQuitMessage(0);
            0
        }
        WM_SETTINGCHANGE => {
            if lp != 0 && wstr_eq(lp as *const u16, "ImmersiveColorSet") {
                with_app(|a| a.refresh_theme());
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe extern "system" fn popup_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let painted = with_app(|a| popup::paint(hwnd, &a.popup_rows(), &a.popup_style()));
            if painted.is_none() {
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_ACTIVATE if (wp & 0xFFFF) as u32 == WA_INACTIVE => {
            hide_popup(hwnd);
            0
        }
        WM_KEYDOWN if wp == VK_ESCAPE => {
            hide_popup(hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn wstr_eq(p: *const u16, s: &str) -> bool {
    let mut i = 0;
    for c in s.encode_utf16() {
        if unsafe { *p.add(i) } != c {
            return false;
        }
        i += 1;
    }
    unsafe { *p.add(i) == 0 }
}

fn popup_fonts() -> (f32, HFONT, HFONT) {
    unsafe {
        let dc = GetDC(null_mut());
        let dpi = GetDeviceCaps(dc, LOGPIXELSY as _);
        ReleaseDC(null_mut(), dc);
        let height = -((9.0 * dpi as f32 / 72.0).round() as i32);
        let face = wide("Segoe UI");
        let font = |weight| {
            CreateFontW(
                height, 0, 0, 0, weight, 0, 0, 0,
                DEFAULT_CHARSET as u32, OUT_TT_PRECIS as u32, CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32, DEFAULT_PITCH as u32, face.as_ptr(),
            )
        };
        (dpi as f32 / 96.0, font(400), font(600))
    }
}

/// Cantos arredondados do Windows 11 e borda escura quando o tema é escuro.
fn set_popup_frame(popup: HWND, light: bool) {
    const DWMWA_USE_IMMERSIVE_DARK_MODE: i32 = 20;
    const DWMWA_WINDOW_CORNER_PREFERENCE: i32 = 33;
    const DWMWCP_ROUND: u32 = 2;
    let dark = u32::from(!light);
    unsafe {
        DwmSetWindowAttribute(popup, DWMWA_WINDOW_CORNER_PREFERENCE as _, &DWMWCP_ROUND as *const u32 as *const c_void, 4);
        DwmSetWindowAttribute(popup, DWMWA_USE_IMMERSIVE_DARK_MODE as _, &dark as *const u32 as *const c_void, 4);
    }
}

/// Menus de contexto escuros quando o Windows está no tema escuro.
/// Usa os ordinais não documentados do uxtheme (SetPreferredAppMode e FlushMenuThemes, Windows 10 1903+).
fn allow_dark_menus() {
    unsafe {
        let name = wide("uxtheme.dll");
        let ux = LoadLibraryExW(name.as_ptr(), null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32);
        if ux.is_null() {
            return;
        }
        if let Some(f) = GetProcAddress(ux, 135 as *const u8) {
            let set_preferred_app_mode: unsafe extern "system" fn(i32) -> i32 = transmute(f);
            set_preferred_app_mode(1); // AllowDark: segue o tema do sistema
        }
        if let Some(f) = GetProcAddress(ux, 136 as *const u8) {
            let flush_menu_themes: unsafe extern "system" fn() = transmute(f);
            flush_menu_themes();
        }
    }
}

fn register_class(name: &[u16], proc_: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT) {
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(proc_),
            hInstance: GetModuleHandleW(null()),
            lpszClassName: name.as_ptr(),
            ..zeroed()
        };
        RegisterClassExW(&wc);
    }
}

/// `TrayTemp.exe --dump [arquivo]`: grava as leituras num arquivo de texto (diagnóstico).
fn dump(path: Option<String>) {
    let mut out = String::new();
    match Cpu::open() {
        Ok(cpu) => {
            let _ = writeln!(out, "CPU: {} | TjMax {} | sensor do pacote: {}", cpu.name, cpu.tjmax, cpu.has_package);
            let mut temps = Vec::new();
            for i in 1..=3 {
                let package = cpu.read(&mut temps);
                let _ = writeln!(out, "leitura {i}: pacote {:?}", package);
                for (core, t) in cpu.cores.iter().zip(&temps) {
                    let _ = writeln!(out, "  {:<10} {:?}", core.long, t);
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
        Err(e) => {
            let _ = writeln!(out, "CPU: erro: {}", e.message());
        }
    }
    match Gpu::find() {
        Some(mut gpu) => {
            let awake = gpu.is_awake();
            let reading = gpu.read();
            let _ = writeln!(out, "GPU: {} | ligada: {awake} | leitura: {reading:?}", gpu.name);
        }
        None => {
            let _ = writeln!(out, "GPU: nenhuma placa NVIDIA encontrada");
        }
    }
    let _ = writeln!(out, "Iniciar com o Windows: {}", autostart::is_enabled());
    let path = path.unwrap_or_else(|| std::env::temp_dir().join("TrayTemp-dump.txt").display().to_string());
    let _ = std::fs::write(path, out);
}

/// Fecha a instância em execução (se houver) e espera ela terminar.
fn exit_running_instance() {
    let class = wide("TrayTempWindow");
    unsafe {
        let hwnd = FindWindowW(class.as_ptr(), null());
        if hwnd.is_null() {
            return;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        PostMessageW(hwnd, WM_CLOSE, 0, 0);
        if !process.is_null() {
            WaitForSingleObject(process, 5000);
            CloseHandle(process);
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--dump") => {
            dump(args.next());
            return;
        }
        // Usados pelo instalador; o código de saída indica sucesso (0) ou falha (1).
        Some("--autostart") => {
            let ok = match args.next().as_deref() {
                Some("on") => autostart::enable(),
                Some("off") => !autostart::is_enabled() || autostart::disable(),
                _ => false,
            };
            std::process::exit(if ok { 0 } else { 1 });
        }
        Some("--exit") => {
            exit_running_instance();
            return;
        }
        _ => {}
    }

    unsafe {
        let mutex_name = wide(r"Local\TrayTemp.SingleInstance");
        let _mutex = CreateMutexW(null(), 1, mutex_name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        allow_dark_menus();

        let main_class = wide("TrayTempWindow");
        let popup_class = wide("TrayTempPopup");
        register_class(&main_class, main_proc);
        register_class(&popup_class, popup_proc);
        let title = wide("TrayTemp");
        let hinst = GetModuleHandleW(null());
        let hwnd = CreateWindowExW(0, main_class.as_ptr(), title.as_ptr(), WS_OVERLAPPED, 0, 0, 0, 0, null_mut(), null_mut(), hinst, null());
        let popup = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            popup_class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0, 0, 0, 0,
            null_mut(), null_mut(), hinst, null(),
        );

        let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        TASKBAR_CREATED.set(taskbar_created);
        // O Explorer roda sem elevação; sem isto o UIPI bloquearia as mensagens dele para esta janela elevada.
        for m in [taskbar_created, WM_TRAY] {
            ChangeWindowMessageFilterEx(hwnd, m, MSGFLT_ALLOW, null_mut());
        }

        let app = App::new(hwnd, popup);
        set_popup_frame(popup, app.apps_light);
        let interval = app.cfg.interval_ms;
        APP.with(|c| *c.borrow_mut() = Some(app));
        with_app(|a| a.update());
        SetTimer(hwnd, TIMER_ID, interval, None);

        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        with_app(|a| a.remove_icons());
    }
}
