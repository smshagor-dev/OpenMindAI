use crate::app_error::AppError;

const SERVICE_NAME: &str = "OpenMindAI";

pub(crate) fn set_secret(slot: &str, secret: &str) -> Result<(), AppError> {
    if secret.is_empty() {
        return delete_secret(slot);
    }
    platform::set_secret(slot, secret)
}

pub(crate) fn get_secret(slot: &str) -> Result<Option<String>, AppError> {
    platform::get_secret(slot)
}

pub(crate) fn delete_secret(slot: &str) -> Result<(), AppError> {
    platform::delete_secret(slot)
}

fn store_error(message: impl Into<String>) -> AppError {
    AppError::SecretStoreFailed(message.into())
}

#[cfg(target_os = "windows")]
mod platform {
    use std::{ffi::c_void, io, ptr, slice};

    use super::{store_error, AppError, SERVICE_NAME};

    const CRED_TYPE_GENERIC: u32 = 1;
    const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;
    const ERROR_NOT_FOUND: i32 = 1168;
    const MAX_CREDENTIAL_BLOB_SIZE: usize = 5 * 512;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FileTime {
        low_date_time: u32,
        high_date_time: u32,
    }

    #[repr(C)]
    struct CredentialW {
        flags: u32,
        credential_type: u32,
        target_name: *mut u16,
        comment: *mut u16,
        last_written: FileTime,
        credential_blob_size: u32,
        credential_blob: *mut u8,
        persist: u32,
        attribute_count: u32,
        attributes: *mut c_void,
        target_alias: *mut u16,
        user_name: *mut u16,
    }

    #[link(name = "Advapi32")]
    extern "system" {
        fn CredWriteW(credential: *const CredentialW, flags: u32) -> i32;
        fn CredReadW(
            target_name: *const u16,
            credential_type: u32,
            flags: u32,
            credential: *mut *mut CredentialW,
        ) -> i32;
        fn CredDeleteW(target_name: *const u16, credential_type: u32, flags: u32) -> i32;
        fn CredFree(buffer: *mut c_void);
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn target(slot: &str) -> String {
        format!("{SERVICE_NAME}:{slot}")
    }

    fn last_error(action: &str) -> AppError {
        store_error(format!(
            "Windows Credential Manager could not {action}: {}",
            io::Error::last_os_error()
        ))
    }

    pub(super) fn set_secret(slot: &str, secret: &str) -> Result<(), AppError> {
        let mut blob = secret.as_bytes().to_vec();
        if blob.len() > MAX_CREDENTIAL_BLOB_SIZE {
            return Err(store_error(format!(
                "credential is too large for Windows Credential Manager ({} bytes)",
                blob.len()
            )));
        }

        let mut target_name = wide(&target(slot));
        let mut user_name = wide(SERVICE_NAME);
        let credential = CredentialW {
            flags: 0,
            credential_type: CRED_TYPE_GENERIC,
            target_name: target_name.as_mut_ptr(),
            comment: ptr::null_mut(),
            last_written: FileTime {
                low_date_time: 0,
                high_date_time: 0,
            },
            credential_blob_size: blob.len() as u32,
            credential_blob: blob.as_mut_ptr(),
            persist: CRED_PERSIST_LOCAL_MACHINE,
            attribute_count: 0,
            attributes: ptr::null_mut(),
            target_alias: ptr::null_mut(),
            user_name: user_name.as_mut_ptr(),
        };

        let result = unsafe { CredWriteW(&credential, 0) };
        blob.fill(0);
        if result == 0 {
            return Err(last_error("store a credential"));
        }
        Ok(())
    }

    pub(super) fn get_secret(slot: &str) -> Result<Option<String>, AppError> {
        let target_name = wide(&target(slot));
        let mut credential: *mut CredentialW = ptr::null_mut();
        let result =
            unsafe { CredReadW(target_name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };

        if result == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NOT_FOUND) {
                return Ok(None);
            }
            return Err(store_error(format!(
                "Windows Credential Manager could not read a credential: {error}"
            )));
        }

        if credential.is_null() {
            return Err(store_error(
                "Windows Credential Manager returned an empty credential pointer",
            ));
        }

        let bytes = unsafe {
            let credential_ref = &mut *credential;
            if credential_ref.credential_blob.is_null() || credential_ref.credential_blob_size == 0
            {
                Vec::new()
            } else {
                slice::from_raw_parts(
                    credential_ref.credential_blob,
                    credential_ref.credential_blob_size as usize,
                )
                .to_vec()
            }
        };

        unsafe {
            let credential_ref = &mut *credential;
            if !credential_ref.credential_blob.is_null() && credential_ref.credential_blob_size > 0
            {
                ptr::write_bytes(
                    credential_ref.credential_blob,
                    0,
                    credential_ref.credential_blob_size as usize,
                );
            }
            CredFree(credential.cast::<c_void>());
        }

        let secret = String::from_utf8(bytes)
            .map_err(|_| store_error("stored Windows credential is not valid UTF-8"))?;
        Ok(Some(secret))
    }

    pub(super) fn delete_secret(slot: &str) -> Result<(), AppError> {
        let target_name = wide(&target(slot));
        let result = unsafe { CredDeleteW(target_name.as_ptr(), CRED_TYPE_GENERIC, 0) };
        if result == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NOT_FOUND) {
                return Ok(());
            }
            return Err(store_error(format!(
                "Windows Credential Manager could not delete a credential: {error}"
            )));
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::process::Command;

    use super::{store_error, AppError, SERVICE_NAME};

    fn command_error(action: &str, stderr: &[u8]) -> AppError {
        let detail = String::from_utf8_lossy(stderr).trim().to_string();
        if detail.is_empty() {
            store_error(format!("macOS Keychain could not {action}"))
        } else {
            store_error(format!("macOS Keychain could not {action}: {detail}"))
        }
    }

    pub(super) fn set_secret(slot: &str, secret: &str) -> Result<(), AppError> {
        let output = Command::new("/usr/bin/security")
            .args([
                "add-generic-password",
                "-U",
                "-a",
                slot,
                "-s",
                SERVICE_NAME,
                "-w",
                secret,
            ])
            .output()
            .map_err(|error| store_error(format!("could not open macOS Keychain: {error}")))?;
        if !output.status.success() {
            return Err(command_error("store a credential", &output.stderr));
        }
        Ok(())
    }

    pub(super) fn get_secret(slot: &str) -> Result<Option<String>, AppError> {
        let output = Command::new("/usr/bin/security")
            .args([
                "find-generic-password",
                "-a",
                slot,
                "-s",
                SERVICE_NAME,
                "-w",
            ])
            .output()
            .map_err(|error| store_error(format!("could not open macOS Keychain: {error}")))?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr).to_lowercase();
            if output.status.code() == Some(44)
                || detail.contains("could not be found")
                || detail.contains("item not found")
            {
                return Ok(None);
            }
            return Err(command_error("read a credential", &output.stderr));
        }
        Ok(Some(
            String::from_utf8_lossy(&output.stdout)
                .trim_end_matches(['\r', '\n'])
                .to_string(),
        ))
    }

    pub(super) fn delete_secret(slot: &str) -> Result<(), AppError> {
        if get_secret(slot)?.is_none() {
            return Ok(());
        }
        let output = Command::new("/usr/bin/security")
            .args(["delete-generic-password", "-a", slot, "-s", SERVICE_NAME])
            .output()
            .map_err(|error| store_error(format!("could not open macOS Keychain: {error}")))?;
        if !output.status.success() {
            return Err(command_error("delete a credential", &output.stderr));
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };

    use super::{store_error, AppError, SERVICE_NAME};

    fn secret_tool_error(action: &str, stderr: &[u8]) -> AppError {
        let detail = String::from_utf8_lossy(stderr).trim().to_string();
        let suffix = if detail.is_empty() {
            "Install the libsecret command-line utility (`secret-tool`) and ensure a desktop keyring is available."
                .to_string()
        } else {
            detail
        };
        store_error(format!("Linux Secret Service could not {action}: {suffix}"))
    }

    pub(super) fn set_secret(slot: &str, secret: &str) -> Result<(), AppError> {
        let mut child = Command::new("secret-tool")
            .args([
                "store",
                "--label=OpenMindAI credential",
                "service",
                SERVICE_NAME,
                "slot",
                slot,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                store_error(format!(
                    "could not open Linux Secret Service (`secret-tool`): {error}"
                ))
            })?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(secret.as_bytes())?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(secret_tool_error("store a credential", &output.stderr));
        }
        Ok(())
    }

    pub(super) fn get_secret(slot: &str) -> Result<Option<String>, AppError> {
        let output = Command::new("secret-tool")
            .args(["lookup", "service", SERVICE_NAME, "slot", slot])
            .output()
            .map_err(|error| {
                store_error(format!(
                    "could not open Linux Secret Service (`secret-tool`): {error}"
                ))
            })?;
        if !output.status.success() {
            if output.status.code() == Some(1) {
                return Ok(None);
            }
            return Err(secret_tool_error("read a credential", &output.stderr));
        }
        let value = String::from_utf8_lossy(&output.stdout)
            .trim_end_matches(['\r', '\n'])
            .to_string();
        if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }

    pub(super) fn delete_secret(slot: &str) -> Result<(), AppError> {
        let output = Command::new("secret-tool")
            .args(["clear", "service", SERVICE_NAME, "slot", slot])
            .output()
            .map_err(|error| {
                store_error(format!(
                    "could not open Linux Secret Service (`secret-tool`): {error}"
                ))
            })?;
        if output.status.success() || output.status.code() == Some(1) {
            return Ok(());
        }
        Err(secret_tool_error("delete a credential", &output.stderr))
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod platform {
    use super::{store_error, AppError};

    pub(super) fn set_secret(_slot: &str, _secret: &str) -> Result<(), AppError> {
        Err(store_error(
            "secure credential storage is not implemented for this platform",
        ))
    }

    pub(super) fn get_secret(_slot: &str) -> Result<Option<String>, AppError> {
        Err(store_error(
            "secure credential storage is not implemented for this platform",
        ))
    }

    pub(super) fn delete_secret(_slot: &str) -> Result<(), AppError> {
        Err(store_error(
            "secure credential storage is not implemented for this platform",
        ))
    }
}
