//! A session descriptor is a bearer credential. Never write its contents before
//! protecting the containing directory and the new file; fail closed on error.
use std::{
    fs,
    io::{self, Write},
    path::Path,
};

fn refused(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

fn regular_owned_path(path: &Path, directory: bool) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink()
        || (directory && !meta.is_dir())
        || (!directory && !meta.is_file())
    {
        return Err(refused(
            "descriptor path must not contain a link or special file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != unsafe { libc::geteuid() } || (!directory && meta.nlink() != 1) {
            return Err(refused(
                "descriptor path is not exclusively owned by the current user",
            ));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(refused(
                "descriptor path must not be a Windows reparse point",
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn protect(path: &Path, directory: bool) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if directory { 0o700 } else { 0o600 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    regular_owned_path(path, directory)?;
    if fs::metadata(path)?.permissions().mode() & 0o777 != mode {
        return Err(refused("could not enforce private descriptor permissions"));
    }
    Ok(())
}

#[cfg(windows)]
fn protect(path: &Path, directory: bool) -> io::Result<()> {
    use std::{os::windows::process::CommandExt, process::Command};
    // Windows PowerShell's ACL APIs work on supported desktop Windows without
    // an extra runtime. No token is passed in process arguments or output.
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$phase = 10
try {
$p = $env:OPENMINDAI_PRIVATE_PATH
$isDir = $env:OPENMINDAI_PRIVATE_DIRECTORY -eq '1'
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$item = Get-Item -LiteralPath $p -Force
if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Linked descriptor path' }
$phase = 20
$old = Get-Acl -LiteralPath $p
$owner = $old.GetOwner([System.Security.Principal.SecurityIdentifier]).Value
$principal = [System.Security.Principal.WindowsPrincipal]::new([System.Security.Principal.WindowsIdentity]::GetCurrent())
$adminOwned = $owner -eq 'S-1-5-32-544' -and $principal.IsInRole([System.Security.Principal.WindowsBuiltInRole]::Administrator)
if ($owner -ne $sid.Value -and -not $adminOwned) { throw 'Foreign descriptor owner' }
$phase = 30
if ($isDir) {
  $acl = [System.Security.AccessControl.DirectorySecurity]::new()
  $rule = [System.Security.AccessControl.FileSystemAccessRule]::new($sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')
} else {
  $acl = [System.Security.AccessControl.FileSecurity]::new()
  $rule = [System.Security.AccessControl.FileSystemAccessRule]::new($sid, 'FullControl', 'Allow')
}
$acl.SetOwner($sid)
$acl.SetAccessRuleProtection($true, $false)
$acl.AddAccessRule($rule)
$phase = 40
Set-Acl -LiteralPath $p -AclObject $acl
$phase = 50
$verified = Get-Acl -LiteralPath $p
$rules = @($verified.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier]))
if (-not $verified.AreAccessRulesProtected -or $rules.Count -ne 1 -or $rules[0].IdentityReference.Value -ne $sid.Value -or $rules[0].AccessControlType -ne 'Allow' -or $rules[0].FileSystemRights -ne 'FullControl') { throw 'Private ACL verification failed' }
} catch { exit $phase }
"#;
    let windows =
        std::env::var_os("SystemRoot").ok_or_else(|| refused("SystemRoot unavailable"))?;
    let powershell = Path::new(&windows).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let status = Command::new(powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            SCRIPT,
        ])
        .env("OPENMINDAI_PRIVATE_PATH", path)
        .env(
            "OPENMINDAI_PRIVATE_DIRECTORY",
            if directory { "1" } else { "0" },
        )
        .creation_flags(0x08000000)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if !status.success() {
        return Err(refused(&format!(
            "could not enforce owner-only descriptor ACL (phase {})",
            status.code().unwrap_or(-1)
        )));
    }
    regular_owned_path(path, directory)
}

#[cfg(not(any(unix, windows)))]
fn protect(_: &Path, _: bool) -> io::Result<()> {
    Err(refused(
        "private session descriptors are unsupported on this platform",
    ))
}

pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| p.file_name().is_some())
        .ok_or_else(|| refused("descriptor requires a dedicated private directory"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(parent)?;
    regular_owned_path(parent, true)?;
    protect(parent, true)?;
    match fs::symlink_metadata(path) {
        Ok(_) => {
            regular_owned_path(path, false)?;
            #[cfg(windows)]
            protect(path, false)?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let temp = parent.join(format!(".coding-agent-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        protect(&temp, false)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        regular_owned_path(path, false)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn creates_and_replaces_private_descriptor_without_temp_leaks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private/endpoint.json");
        write_atomically(&path, b"first token").unwrap();
        write_atomically(&path, b"replacement token").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"replacement token");
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }
    #[cfg(unix)]
    #[test]
    fn repairs_permissive_directory_and_rejects_links_without_touching_target() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("private");
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o777)).unwrap();
        let outside = dir.path().join("keep");
        fs::write(&outside, b"keep").unwrap();
        let path = parent.join("endpoint.json");
        symlink(&outside, &path).unwrap();
        assert!(write_atomically(&path, b"secret").is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"keep");
        fs::remove_file(&path).unwrap();
        fs::hard_link(&outside, &path).unwrap();
        assert!(write_atomically(&path, b"secret").is_err());
        fs::remove_file(&path).unwrap();
        write_atomically(&path, b"secret").unwrap();
        assert_eq!(
            fs::metadata(parent).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    #[test]
    fn non_file_target_is_preserved_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private/endpoint.json");
        fs::create_dir_all(&path).unwrap();
        assert!(write_atomically(&path, b"secret").is_err());
        assert!(path.is_dir());
    }
    #[cfg(unix)]
    #[test]
    fn another_uid_cannot_read_descriptor_in_an_accessible_parent() {
        use std::os::unix::{fs::PermissionsExt, process::CommandExt};
        // Privileged local harnesses can exercise a real separate UID. Native
        // runners without this privilege still enforce exact mode/owner tests.
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let control = dir.path().join("public");
        fs::write(&control, b"public control").unwrap();
        fs::set_permissions(&control, fs::Permissions::from_mode(0o644)).unwrap();
        let path = dir.path().join("private/endpoint.json");
        write_atomically(&path, b"secret token").unwrap();
        let read_as_other = |file: &Path| {
            let mut command = std::process::Command::new("/bin/cat");
            command.arg(file);
            unsafe {
                command.pre_exec(|| {
                    if libc::setgroups(0, std::ptr::null()) != 0
                        || libc::setgid(65534) != 0
                        || libc::setuid(65534) != 0
                    {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            command.output().unwrap()
        };
        assert!(read_as_other(&control).status.success());
        let denied = read_as_other(&path);
        assert!(!denied.status.success());
        assert!(denied.stdout.is_empty());
    }
}
