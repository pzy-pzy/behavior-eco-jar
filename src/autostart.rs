//! 开机自启：写 HKCU\...\Run，不碰系统目录、不需要管理员权限。
//!
//! 注册的是当前可执行文件的路径；命令行带 `--autostart`，
//! 方便以后区分“用户手动打开”和“开机拉起”。

#[cfg(windows)]
mod imp {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
        RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE,
        REG_SZ,
    };

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    /// reg.exe 需要带根键的完整路径。
    const RUN_KEY_FULL: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "BehaviorEcoJar";

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// 启动命令行。路径无空格时不加引号，避免 reg.exe 把内层引号吃掉。
    fn command_line() -> std::io::Result<String> {
        let exe = std::env::current_exe()?;
        let path = exe.to_string_lossy();
        if path.contains(' ') {
            Ok(format!("\"{path}\" --autostart"))
        } else {
            Ok(format!("{path} --autostart"))
        }
    }

    /// 当前是否已经登记在启动项里。
    pub fn is_enabled() -> bool {
        let sub = wide(RUN_KEY);
        let name = wide(VALUE_NAME);
        unsafe {
            let mut key: HKEY = std::ptr::null_mut();
            if RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_READ, &mut key) != ERROR_SUCCESS
            {
                return false;
            }
            let mut size = 0u32;
            let rc = RegQueryValueExW(
                key,
                name.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            );
            RegCloseKey(key);
            rc == ERROR_SUCCESS
        }
    }

    /// 打开或关闭开机自启。
    ///
    /// 先直接写注册表；某些终端安全软件会拦截未签名程序的注册表写入，
    /// 这时退回到调用系统自带的 `reg.exe`（带 CREATE_NO_WINDOW，不闪窗）。
    pub fn set_enabled(enabled: bool) -> std::io::Result<()> {
        let direct = set_direct(enabled);
        if direct.is_ok() {
            return Ok(());
        }

        match set_via_reg_exe(enabled) {
            Ok(()) => Ok(()),
            Err(e) => {
                let first = direct.err().map(|e| e.to_string()).unwrap_or_default();
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("direct: {first}; reg.exe: {e}"),
                ))
            }
        }
    }

    fn set_via_reg_exe(enabled: bool) -> std::io::Result<()> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        let mut command = std::process::Command::new("reg.exe");
        if enabled {
            let cmd = command_line()?;
            command.args([
                "add", RUN_KEY_FULL, "/v", VALUE_NAME, "/t", "REG_SZ", "/d", &cmd, "/f",
            ]);
        } else {
            command.args(["delete", RUN_KEY_FULL, "/v", VALUE_NAME, "/f"]);
        }
        let command = command.creation_flags(CREATE_NO_WINDOW);
        let output = command.output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }

    fn set_direct(enabled: bool) -> std::io::Result<()> {
        unsafe {
            if !enabled {
                let sub = wide(RUN_KEY);
                let mut key: HKEY = std::ptr::null_mut();
                if RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_WRITE, &mut key)
                    == ERROR_SUCCESS
                {
                    let name = wide(VALUE_NAME);
                    RegDeleteValueW(key, name.as_ptr());
                    RegCloseKey(key);
                }
                return Ok(());
            }

            let cmd = command_line()?;
            let sub = wide(RUN_KEY);
            let name = wide(VALUE_NAME);
            let data = wide(&cmd);

            let mut key: HKEY = std::ptr::null_mut();
            let mut disposition = 0u32;
            let rc = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                sub.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                std::ptr::null(),
                &mut key,
                &mut disposition,
            );
            if rc != ERROR_SUCCESS {
                return Err(std::io::Error::from_raw_os_error(rc as i32));
            }

            let bytes = (data.len() * std::mem::size_of::<u16>()) as u32;
            let rc = RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr() as *const u8,
                bytes,
            );
            RegCloseKey(key);
            if rc != ERROR_SUCCESS {
                return Err(std::io::Error::from_raw_os_error(rc as i32));
            }
        }
        Ok(())
    }

    /// 当前登记的启动命令（诊断用）。
    pub fn registered_command() -> Option<String> {
        let sub = wide(RUN_KEY);
        let name = wide(VALUE_NAME);
        unsafe {
            let mut key: HKEY = std::ptr::null_mut();
            if RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_READ, &mut key) != ERROR_SUCCESS
            {
                return None;
            }
            let mut size = 0u32;
            let rc = RegQueryValueExW(
                key,
                name.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            );
            if rc != ERROR_SUCCESS || size == 0 {
                RegCloseKey(key);
                return None;
            }
            let mut buf = vec![0u8; size as usize];
            let mut ty = 0u32;
            let rc = RegQueryValueExW(
                key,
                name.as_ptr(),
                std::ptr::null(),
                &mut ty,
                buf.as_mut_ptr(),
                &mut size,
            );
            RegCloseKey(key);
            if rc != ERROR_SUCCESS {
                return None;
            }
            let wide: Vec<u16> = buf[..size as usize]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            Some(String::from_utf16_lossy(&wide))
        }
    }
}

#[cfg(windows)]
pub use imp::{is_enabled, registered_command, set_enabled};

#[cfg(not(windows))]
pub fn is_enabled() -> bool {
    false
}

#[cfg(not(windows))]
pub fn registered_command() -> Option<String> {
    None
}

#[cfg(not(windows))]
pub fn set_enabled(_enabled: bool) -> std::io::Result<()> {
    Ok(())
}
