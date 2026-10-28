use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=assets/IntelMSR.bin");
    println!("cargo:rerun-if-changed=assets/TrayTemp.ico");

    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    // Embute o manifesto (pede elevação e declara DPI awareness) direto pelo linker do MSVC.
    let manifest = root.join("app.manifest");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", manifest.display());
    println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:NO");

    // Ícone e informações de versão (aparecem no Explorer, no Gerenciador de Tarefas e no instalador).
    match find_rc() {
        Some(rc) => compile_resources(&rc, &root),
        None => println!("cargo:warning=rc.exe (Windows SDK) não encontrado: o .exe ficará sem ícone e versão"),
    }
}

fn find_rc() -> Option<PathBuf> {
    if let Ok(rc) = env::var("RC") {
        return Some(PathBuf::from(rc));
    }
    let program_files = env::var("ProgramFiles(x86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into());
    let mut found: Vec<PathBuf> = fs::read_dir(Path::new(&program_files).join(r"Windows Kits\10\bin"))
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path().join(r"x64\rc.exe"))
        .filter(|p| p.exists())
        .collect();
    found.sort();
    found.pop()
}

fn compile_resources(rc: &Path, root: &Path) {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let mut parts: Vec<u32> = version.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    parts.resize(4, 0);
    let numeric = parts.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let icon = root.join("assets").join("TrayTemp.ico").display().to_string().replace('\\', "/");

    // Sem #include para não depender dos caminhos de include do SDK: as constantes vão como números.
    let script = format!(
        r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "041604b0"
    BEGIN
      VALUE "FileDescription", "TrayTemp"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "TrayTemp"
      VALUE "OriginalFilename", "TrayTemp.exe"
      VALUE "ProductName", "TrayTemp"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x416, 1200
  END
END
"#
    );
    let rc_file = out.join("traytemp.rc");
    let res_file = out.join("traytemp.res");
    fs::write(&rc_file, script).unwrap();
    let status = Command::new(rc)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res_file)
        .arg(&rc_file)
        .status()
        .expect("falha ao executar rc.exe");
    assert!(status.success(), "rc.exe falhou");
    println!("cargo:rustc-link-arg-bins={}", res_file.display());
}
