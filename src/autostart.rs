//! "Launch at login" without extra crates:
//! macOS LaunchAgent plist, Linux XDG autostart entry, Windows HKCU Run key.

use std::io;
#[cfg(not(windows))]
use std::path::PathBuf;

#[cfg(not(target_os = "macos"))]
const APP_NAME: &str = "light-control";

fn exe() -> io::Result<String> {
    Ok(std::env::current_exe()?.to_string_lossy().into_owned())
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    fn plist() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|d| {
            d.home_dir()
                .join("Library/LaunchAgents/com.tanmaster.light-control.plist")
        })
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    pub fn is_enabled() -> bool {
        plist().is_some_and(|p| p.exists())
    }

    pub fn set_enabled(on: bool) -> io::Result<()> {
        let path = plist().ok_or_else(|| io::Error::other("no home directory"))?;
        if !on {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                r => r,
            };
        }
        let body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>com.tanmaster.light-control</string>
    <key>ProgramArguments</key><array><string>{}</string></array>
    <key>RunAtLoad</key><true/>
</dict>
</plist>
"#,
            xml_escape(&exe()?)
        );
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, body)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use super::*;

    fn entry() -> Option<PathBuf> {
        directories::BaseDirs::new()
            .map(|d| d.config_dir().join("autostart").join(format!("{APP_NAME}.desktop")))
    }

    pub fn is_enabled() -> bool {
        entry().is_some_and(|p| p.exists())
    }

    pub fn set_enabled(on: bool) -> io::Result<()> {
        let path = entry().ok_or_else(|| io::Error::other("no home directory"))?;
        if !on {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                r => r,
            };
        }
        let body = format!(
            "[Desktop Entry]\nType=Application\nName={APP_NAME}\nExec=\"{}\"\nX-GNOME-Autostart-enabled=true\n",
            exe()?
        );
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, body)
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn reg(args: &[&str]) -> io::Result<bool> {
        Ok(Command::new("reg")
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?
            .success())
    }

    pub fn is_enabled() -> bool {
        reg(&["query", RUN_KEY, "/v", APP_NAME]).unwrap_or(false)
    }

    pub fn set_enabled(on: bool) -> io::Result<()> {
        let ok = if on {
            let value = format!("\"{}\"", exe()?);
            reg(&["add", RUN_KEY, "/v", APP_NAME, "/t", "REG_SZ", "/d", &value, "/f"])?
        } else {
            !is_enabled() || reg(&["delete", RUN_KEY, "/v", APP_NAME, "/f"])?
        };
        if ok {
            Ok(())
        } else {
            Err(io::Error::other("reg.exe failed"))
        }
    }
}

pub use imp::{is_enabled, set_enabled};
