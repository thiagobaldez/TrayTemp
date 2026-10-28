use std::fs;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CpuMode {
    Max,
    Average,
    Package,
    PerCore,
}

impl CpuMode {
    fn key(self) -> &'static str {
        match self {
            CpuMode::Max => "max",
            CpuMode::Average => "avg",
            CpuMode::Package => "package",
            CpuMode::PerCore => "cores",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CpuMode::Max => "máxima",
            CpuMode::Average => "média",
            CpuMode::Package => "pacote",
            CpuMode::PerCore => "por núcleo",
        }
    }
}

pub struct Config {
    pub cpu_mode: CpuMode,
    pub show_gpu: bool,
    pub interval_ms: u32,
    pub warn: i32,
    pub crit: i32,
    /// Fonte dos números na bandeja.
    pub font: String,
    /// Tamanho em pontos; 0 = números preenchem o ícone inteiro.
    pub font_size: u32,
    pub font_bold: bool,
}

const DEFAULT_FONT: &str = "Trebuchet MS";

impl Default for Config {
    fn default() -> Self {
        Config {
            cpu_mode: CpuMode::Max,
            show_gpu: true,
            interval_ms: 2000,
            warn: 85,
            crit: 95,
            font: DEFAULT_FONT.to_string(),
            font_size: 8,
            font_bold: false,
        }
    }
}

fn path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("TrayTemp").join("config.ini"))
}

impl Config {
    pub fn load() -> Config {
        let mut c = Config::default();
        let Some(text) = path().and_then(|p| fs::read_to_string(p).ok()) else {
            return c;
        };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "cpu_mode" => {
                    c.cpu_mode = match v {
                        "avg" => CpuMode::Average,
                        "package" => CpuMode::Package,
                        "cores" => CpuMode::PerCore,
                        _ => CpuMode::Max,
                    }
                }
                "show_gpu" => c.show_gpu = v != "0",
                "interval_ms" => {
                    if let Ok(n) = v.parse::<u32>() {
                        c.interval_ms = n.clamp(500, 60_000);
                    }
                }
                "warn" => c.warn = v.parse().unwrap_or(c.warn),
                "crit" => c.crit = v.parse().unwrap_or(c.crit),
                "font" if !v.is_empty() => c.font = v.to_string(),
                "font_size" => c.font_size = v.parse::<u32>().map_or(c.font_size, |n| n.min(72)),
                "font_bold" => c.font_bold = v != "0",
                _ => {}
            }
        }
        c
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let text = format!(
            "; TrayTemp\n\
             ; cpu_mode: max | avg | package | cores\n\
             cpu_mode={}\n\
             show_gpu={}\n\
             interval_ms={}\n\
             ; limites (°C) para laranja e vermelho\n\
             warn={}\n\
             crit={}\n\
             ; fonte dos números na bandeja; font_size em pontos (0 = preencher o ícone)\n\
             font={}\n\
             font_size={}\n\
             font_bold={}\n",
            self.cpu_mode.key(),
            self.show_gpu as u8,
            self.interval_ms,
            self.warn,
            self.crit,
            self.font,
            self.font_size,
            self.font_bold as u8,
        );
        let _ = fs::write(p, text);
    }
}
