//! Opt-in Explorer context-menu entry ("Analyze Space with diskmap") under HKCU, so no admin rights.
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

/// Folder, folder background (right-click inside a folder) and drive.
const ROOTS: [&str; 3] = [r"Software\Classes\Directory\shell\diskmap", r"Software\Classes\Directory\Background\shell\diskmap", r"Software\Classes\Drive\shell\diskmap"];

pub fn is_installed() -> bool {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey(ROOTS[0]).is_ok()
}

pub fn install() -> std::io::Result<()> {
    let exe = std::env::current_exe()?.display().to_string();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for root in ROOTS {
        let (key, _) = hkcu.create_subkey(root)?;
        key.set_value("", &i18n::t!("app.service.analyze"))?;
        key.set_value("Icon", &exe)?;
        let (cmd, _) = key.create_subkey("command")?;
        cmd.set_value("", &format!("\"{exe}\" \"%V\""))?;
    }
    Ok(())
}

pub fn uninstall() -> std::io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for root in ROOTS {
        match hkcu.delete_subkey_all(root) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}
