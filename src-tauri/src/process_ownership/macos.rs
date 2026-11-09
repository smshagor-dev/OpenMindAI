//! Darwin process identity and restart recovery. ABI references:
//! https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc_info_private.h
//! https://github.com/apple-oss-distributions/xnu/blob/main/libsyscall/wrappers/libproc/libproc.h

use std::{
    io,
    mem::{size_of, MaybeUninit},
    os::unix::process::CommandExt,
    process::{Child, Command},
    sync::OnceLock,
};

use super::ProcessIdentity;

const PROC_PIDT_BSDINFOWITHUNIQID: i32 = 18;

// The 56-byte Darwin ABI is fixed; keep reserved fields instead of guessing offsets.
#[repr(C)]
struct UniqueInfo {
    uuid: [u8; 16],
    unique_id: u64,
    parent_unique_id: u64,
    pid_version: i32,
    original_parent_version: i32,
    reserved: [u64; 2],
}

#[repr(C)]
struct ProcessInfo {
    bsd: libc::proc_bsdinfo,
    unique: UniqueInfo,
}
const _: () = assert!(size_of::<UniqueInfo>() == 56);
const _: () = assert!(size_of::<ProcessInfo>() == 192);

#[repr(C)]
struct AuditToken {
    values: [u32; 8],
}
type SignalWithToken = unsafe extern "C" fn(*mut AuditToken, libc::c_int) -> libc::c_int;

fn signal_function() -> Option<SignalWithToken> {
    static SIGNAL: OnceLock<Option<SignalWithToken>> = OnceLock::new();
    *SIGNAL.get_or_init(|| {
        // SAFETY: libproc is loaded by the libc proc_pidinfo binding. The symbol's
        // ABI is declared in Apple's libproc.h; do not fall back to kill(pid).
        let symbol =
            unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"proc_signal_with_audittoken".as_ptr()) };
        if symbol.is_null() {
            return None;
        }
        Some(unsafe { std::mem::transmute::<*mut libc::c_void, SignalWithToken>(symbol) })
    })
}

pub fn spawn(mut command: Command) -> io::Result<Child> {
    // Isolate runtimes from the app's process group. Recovery deliberately signals
    // only the registered process, never an unverified process-group membership.
    command.process_group(0).spawn()
}

pub fn adopt(_child: &Child) {}

fn process_info(pid: u32) -> Option<ProcessInfo> {
    let native_pid = libc::pid_t::try_from(pid).ok().filter(|pid| *pid > 1)?;
    let mut info = MaybeUninit::<ProcessInfo>::uninit();
    // SAFETY: proc_pidinfo writes exactly this ABI-sized buffer on success.
    let count = unsafe {
        libc::proc_pidinfo(
            native_pid,
            PROC_PIDT_BSDINFOWITHUNIQID,
            0,
            info.as_mut_ptr().cast(),
            size_of::<ProcessInfo>() as i32,
        )
    };
    if count != size_of::<ProcessInfo>() as i32 {
        return None;
    }
    let info = unsafe { info.assume_init() };
    if info.bsd.pbi_pid != pid || info.bsd.pbi_status == libc::SZOMB {
        return None;
    }
    Some(info)
}

fn generation(info: &ProcessInfo) -> (u64, u64, u64, i32) {
    (
        info.bsd.pbi_start_tvsec,
        info.bsd.pbi_start_tvusec,
        info.unique.unique_id,
        info.unique.pid_version,
    )
}

pub fn identity(pid: u32) -> Option<ProcessIdentity> {
    let before = process_info(pid)?;
    let mut path = [0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: this is a writable path buffer of the size required by libproc.
    let count =
        unsafe { libc::proc_pidpath(pid as i32, path.as_mut_ptr().cast(), path.len() as u32) };
    if count <= 0 {
        return None;
    }
    let end = path.iter().position(|byte| *byte == 0)?;
    if end == 0 {
        return None;
    }
    // Reject unrepresentable paths rather than merging distinct non-UTF8 identities.
    let executable = std::str::from_utf8(&path[..end]).ok()?.to_owned();
    if !executable.starts_with('/') || before.bsd.pbi_start_tvusec >= 1_000_000 {
        return None;
    }
    let after = process_info(pid)?;
    if generation(&before) != generation(&after) {
        return None;
    }
    let started = before
        .bsd
        .pbi_start_tvsec
        .checked_mul(1_000_000)?
        .checked_add(before.bsd.pbi_start_tvusec)?;
    if started == 0 {
        return None;
    }
    Some(ProcessIdentity {
        pid,
        started,
        executable,
        macos_pid_version: Some(before.unique.pid_version),
    })
}

fn signal_generation(pid: u32, version: i32) -> bool {
    if pid <= 1 || pid > i32::MAX as u32 {
        return false;
    }
    let Some(signal) = signal_function() else {
        return false;
    };
    // The kernel uses PID + pidversion to address this incarnation atomically.
    // Authorization still uses the caller's credentials, not these token fields.
    let mut token = AuditToken { values: [0; 8] };
    token.values[5] = pid;
    token.values[7] = version as u32;
    // SAFETY: the token is a valid 32-byte audit_token_t, passed to the checked ABI.
    unsafe { signal(&mut token, libc::SIGKILL) == 0 }
}

pub fn terminate_if_identity(expected: &ProcessIdentity) -> bool {
    let Some(version) = expected.macos_pid_version else {
        return false;
    };
    if identity(expected.pid).as_ref() != Some(expected) {
        return false;
    }
    // A PID reused after the lookup is rejected by the kernel's generation check.
    signal_generation(expected.pid, version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_rejects_wrong_process_generation() {
        let mut child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let actual = identity(child.id()).expect("sleep identity");
        let mut stale = actual.clone();
        stale.started += 1;
        assert!(!terminate_if_identity(&stale));
        stale = actual.clone();
        stale.executable.push_str("-different");
        assert!(!terminate_if_identity(&stale));
        stale = actual.clone();
        stale.macos_pid_version = None;
        assert!(!terminate_if_identity(&stale));
        assert!(
            signal_function().is_some(),
            "Darwin token signalling unavailable"
        );
        assert!(!signal_generation(
            child.id(),
            actual.macos_pid_version.unwrap().wrapping_add(1)
        ));
        assert!(
            child.try_wait().unwrap().is_none(),
            "wrong-generation signal killed sleep"
        );
        assert!(terminate_if_identity(&actual));
        let _ = child.wait();
    }
}
