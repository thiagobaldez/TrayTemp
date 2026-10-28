//! Inicialização com o Windows via Agendador de Tarefas.
//!
//! A chave Run do registro não serve: o Windows não inicia programas que pedem elevação por ela.
//! Uma tarefa "ao fazer logon" com privilégios mais altos abre o app elevado e sem prompt do UAC.

use std::fs;
use std::os::windows::process::CommandExt;
use std::process::Command;

use windows_sys::Win32::System::Registry::HKEY_LOCAL_MACHINE;

use crate::util::reg_string;

const TASK_NAME: &str = "TrayTemp";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const TASK_XML: &str = r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>TrayTemp: temperatura da CPU e da GPU na bandeja</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{USER}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{USER}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>6</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>"{EXE}"</Command>
    </Exec>
  </Actions>
</Task>
"#;

pub fn is_enabled() -> bool {
    reg_string(
        HKEY_LOCAL_MACHINE,
        &format!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Schedule\TaskCache\Tree\{TASK_NAME}"),
        "Id",
    )
    .is_some()
}

fn schtasks(args: &[&std::ffi::OsStr]) -> bool {
    let exe = format!(r"{}\System32\schtasks.exe", std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()));
    Command::new(exe)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .is_ok_and(|s| s.success())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn enable() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let user = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
        (Ok(d), Ok(u)) => format!(r"{d}\{u}"),
        (_, Ok(u)) => u,
        _ => return false,
    };
    let xml = TASK_XML
        .replace("{USER}", &xml_escape(&user))
        .replace("{EXE}", &xml_escape(&exe.display().to_string()));

    // O schtasks lê o XML em UTF-16 LE com BOM.
    let mut bytes = vec![0xFF, 0xFE];
    for u in xml.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    let path = std::env::temp_dir().join("TrayTemp-task.xml");
    if fs::write(&path, bytes).is_err() {
        return false;
    }
    let ok = schtasks(&["/Create".as_ref(), "/TN".as_ref(), TASK_NAME.as_ref(), "/XML".as_ref(), path.as_os_str(), "/F".as_ref()]);
    let _ = fs::remove_file(&path);
    ok
}

pub fn disable() -> bool {
    schtasks(&["/Delete".as_ref(), "/TN".as_ref(), TASK_NAME.as_ref(), "/F".as_ref()])
}
