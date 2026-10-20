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
    use std::{
        ffi::{c_char, c_void},
        ptr, slice,
    };

    use super::{store_error, AppError, SERVICE_NAME};

    // Preserve the service/account naming scheme from the old security CLI.
    // This reads previously saved items but never puts a password in argv.
    // The legacy generic-password API is available on macOS.
    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    #[link(name = "Security", kind = "framework")]
    extern "C" {
        fn SecKeychainAddGenericPassword(
            keychain: *const c_void,
            service_length: u32,
            service: *const c_char,
            account_length: u32,
            account: *const c_char,
            password_length: u32,
            password: *const c_void,
            item: *mut *mut c_void,
        ) -> i32;
        fn SecKeychainFindGenericPassword(
            keychain_or_array: *const c_void,
            service_length: u32,
            service: *const c_char,
            account_length: u32,
            account: *const c_char,
            password_length: *mut u32,
            password: *mut *mut c_void,
            item: *mut *mut c_void,
        ) -> i32;
        fn SecKeychainItemModifyAttributesAndData(
            item: *mut c_void,
            attributes: *const c_void,
            password_length: u32,
            password: *const c_void,
        ) -> i32;
        fn SecKeychainItemDelete(item: *mut c_void) -> i32;
        fn SecKeychainItemFreeContent(attributes: *mut c_void, data: *mut c_void) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(value: *const c_void);
    }

    struct ItemRef(*mut c_void);

    impl Drop for ItemRef {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0.cast_const()) };
            }
        }
    }

    struct PasswordContent(*mut c_void);

    impl Drop for PasswordContent {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // Memory came from Security.framework, not Rust's allocator.
                unsafe { SecKeychainItemFreeContent(ptr::null_mut(), self.0) };
            }
        }
    }

    fn status_error(operation: &str, status: i32) -> AppError {
        // Never include credentials or account identifiers in diagnostics.
        store_error(format!("macOS Keychain could not {operation} (OSStatus {status})"))
    }

    fn sizes(slot: &str, secret: Option<&str>) -> Result<(u32, u32, u32), AppError> {
        let service_len = u32::try_from(SERVICE_NAME.len())
            .map_err(|_| store_error("macOS Keychain service name is too large"))?;
        let slot_len = u32::try_from(slot.len())
            .map_err(|_| store_error("macOS Keychain slot name is too large"))?;
        let secret_len = u32::try_from(secret.unwrap_or("").len())
            .map_err(|_| store_error("macOS Keychain credential is too large"))?;
        Ok((service_len, slot_len, secret_len))
    }

    fn find_item(slot: &str) -> Result<Option<ItemRef>, AppError> {
        let (service_len, slot_len, _) = sizes(slot, None)?;
        let mut item = ptr::null_mut();
        let status = unsafe {
            SecKeychainFindGenericPassword(
                ptr::null(),
                service_len,
                SERVICE_NAME.as_ptr().cast(),
                slot_len,
                slot.as_ptr().cast(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut item,
            )
        };
        if status == ERR_SEC_ITEM_NOT_FOUND {
            return Ok(None);
        }
        if status != 0 {
            return Err(status_error("find a credential", status));
        }
        if item.is_null() {
            return Err(store_error("macOS Keychain returned an empty item reference"));
        }
        Ok(Some(ItemRef(item)))
    }

    pub(super) fn set_secret(slot: &str, secret: &str) -> Result<(), AppError> {
        let (service_len, slot_len, secret_len) = sizes(slot, Some(secret))?;
        if let Some(item) = find_item(slot)? {
            let status = unsafe {
                SecKeychainItemModifyAttributesAndData(
                    item.0,
                    ptr::null(),
                    secret_len,
                    secret.as_ptr().cast(),
                )
            };
            return if status == 0 {
                Ok(())
            } else {
                Err(status_error("update a credential", status))
            };
        }

        let status = unsafe {
            SecKeychainAddGenericPassword(
                ptr::null(),
                service_len,
                SERVICE_NAME.as_ptr().cast(),
                slot_len,
                slot.as_ptr().cast(),
                secret_len,
                secret.as_ptr().cast(),
                ptr::null_mut(),
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(status_error("store a credential", status))
        }
    }

    pub(super) fn get_secret(slot: &str) -> Result<Option<String>, AppError> {
        let (service_len, slot_len, _) = sizes(slot, None)?;
        let mut password_len = 0_u32;
        let mut password_ptr = ptr::null_mut();
        let status = unsafe {
            SecKeychainFindGenericPassword(
                ptr::null(),
                service_len,
                SERVICE_NAME.as_ptr().cast(),
                slot_len,
                slot.as_ptr().cast(),
                &mut password_len,
                &mut password_ptr,
                ptr::null_mut(),
            )
        };
        let content = PasswordContent(password_ptr);
        if status == ERR_SEC_ITEM_NOT_FOUND {
            return Ok(None);
        }
        if status != 0 {
            return Err(status_error("read a credential", status));
        }
        if password_len > 0 && content.0.is_null() {
            return Err(store_error("macOS Keychain returned an empty password pointer"));
        }
        let bytes = if password_len == 0 {
            &[][..]
        } else {
            unsafe { slice::from_raw_parts(content.0.cast::<u8>(), password_len as usize) }
        };
        let password = std::str::from_utf8(bytes)
            .map_err(|_| store_error("macOS Keychain credential is not valid UTF-8"))?
            .to_string();
        Ok(Some(password))
    }

    pub(super) fn delete_secret(slot: &str) -> Result<(), AppError> {
        let Some(item) = find_item(slot)? else {
            return Ok(());
        };
        let status = unsafe { SecKeychainItemDelete(item.0) };
        if status == 0 || status == ERR_SEC_ITEM_NOT_FOUND {
            Ok(())
        } else {
            Err(status_error("delete a credential", status))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn storing_secrets_never_invokes_a_subprocess() {
            let source = include_str!("secret_store.rs");
            let implementation = source.split("#[cfg(target_os = \"macos\")]").nth(1).unwrap()
                .split("    #[cfg(test)]").next().unwrap();
            assert!(!implementation.contains("Command::new("));
            assert!(implementation.contains("SecKeychainAddGenericPassword("));
        }

        #[test]
        #[ignore = "requires an unlocked user macOS login Keychain"]
        fn synthetic_keychain_round_trip_and_update() {
            let slot = format!(
                "keychain-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let secret = "synthetic-keychain-test-only-credential";
            assert!(get_secret(&slot).unwrap().is_none());
            set_secret(&slot, secret).unwrap();
            assert_eq!(get_secret(&slot).unwrap().as_deref(), Some(secret));
            set_secret(&slot, "synthetic-keychain-updated").unwrap();
            assert_eq!(get_secret(&slot).unwrap().as_deref(), Some("synthetic-keychain-updated"));
            delete_secret(&slot).unwrap();
            assert!(get_secret(&slot).unwrap().is_none());
            delete_secret(&slot).unwrap();
        }
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
