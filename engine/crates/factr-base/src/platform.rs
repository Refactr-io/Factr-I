use std::path::Path;

#[cfg(target_os = "macos")]
mod macos_power {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_void};

    type CFStringRef = *const c_void;
    type IOPMAssertionID = u32;
    type IOReturn = i32;

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const K_IOPM_ASSERTION_LEVEL_ON: u32 = 255;
    const K_IO_RETURN_SUCCESS: IOReturn = 0;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            c_str: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(cf: *const c_void);
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionCreateWithName(
            assertion_type: CFStringRef,
            assertion_level: u32,
            assertion_name: CFStringRef,
            assertion_id: *mut IOPMAssertionID,
        ) -> IOReturn;
        fn IOPMAssertionRelease(assertion_id: IOPMAssertionID) -> IOReturn;
    }

    fn cf_string(value: &str) -> Option<CFStringRef> {
        let c_string = CString::new(value).ok()?;
        let cf = unsafe {
            CFStringCreateWithCString(
                std::ptr::null(),
                c_string.as_ptr(),
                K_CF_STRING_ENCODING_UTF8,
            )
        };
        (!cf.is_null()).then_some(cf)
    }

    pub struct PowerAssertion {
        id: Option<IOPMAssertionID>,
    }

    impl PowerAssertion {
        pub fn prevent_user_idle_system_sleep(reason: &str) -> Self {
            let Some(assertion_type) = cf_string("PreventUserIdleSystemSleep") else {
                return Self { id: None };
            };
            let Some(assertion_name) = cf_string(reason) else {
                unsafe { CFRelease(assertion_type) };
                return Self { id: None };
            };

            let mut id = 0;
            let result = unsafe {
                IOPMAssertionCreateWithName(
                    assertion_type,
                    K_IOPM_ASSERTION_LEVEL_ON,
                    assertion_name,
                    &mut id,
                )
            };
            unsafe {
                CFRelease(assertion_type);
                CFRelease(assertion_name);
            }

            if result == K_IO_RETURN_SUCCESS {
                crate::logging::info(&format!(
                    "Created macOS sleep-prevention assertion while streaming (id={id})"
                ));
                Self { id: Some(id) }
            } else {
                crate::logging::warn(&format!(
                    "Failed to create macOS sleep-prevention assertion while streaming: IOReturn={result}"
                ));
                Self { id: None }
            }
        }

        #[cfg(test)]
        pub fn is_active(&self) -> bool {
            self.id.is_some()
        }
    }

    impl Drop for PowerAssertion {
        fn drop(&mut self) {
            if let Some(id) = self.id.take() {
                let result = unsafe { IOPMAssertionRelease(id) };
                if result != K_IO_RETURN_SUCCESS {
                    crate::logging::warn(&format!(
                        "Failed to release macOS sleep-prevention assertion id={id}: IOReturn={result}"
                    ));
                }
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod macos_power {
    pub struct PowerAssertion;

    impl PowerAssertion {
        pub fn prevent_user_idle_system_sleep(_reason: &str) -> Self {
            Self
        }

        #[cfg(test)]
        pub fn is_active(&self) -> bool {
            false
        }
    }
}

pub use macos_power::PowerAssertion;

#[cfg(any(unix, test))]
fn desired_nofile_soft_limit(current: u64, hard: u64, minimum: u64) -> Option<u64> {
    let desired = current.max(minimum).min(hard);
    (desired > current).then_some(desired)
}

/// Create a symlink (Unix) or copy the file (Windows).
///
/// On Windows, symlinks require elevated privileges or Developer Mode,
/// so we fall back to copying.
pub fn symlink_or_copy(src: &Path, dst: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(src, dst)
    }
    #[cfg(windows)]
    {
        if src.is_dir() {
            std::os::windows::fs::symlink_dir(src, dst).or_else(|_| copy_dir_recursive(src, dst))
        } else {
            std::os::windows::fs::symlink_file(src, dst)
                .or_else(|_| std::fs::copy(src, dst).map(|_| ()))
        }
    }
}

#[cfg(windows)]
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

pub use factr_core::fs::{set_directory_permissions_owner_only, set_permissions_owner_only};

/// Set file permissions to owner read/write/execute (0o755).
/// No-op on Windows (executability is determined by file extension).
pub fn set_permissions_executable(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(path, perms)
    }
    #[cfg(windows)]
    {
        let _ = path;
        Ok(())
    }
}

/// Best-effort increase of the current process soft `RLIMIT_NOFILE` on Unix.
///
/// This helps factr survive short-lived reload/connect spikes even when it was
/// launched from a shell with a conservative `ulimit -n` like 1024.
pub fn raise_nofile_limit_best_effort(minimum_soft_limit: u64) {
    #[cfg(unix)]
    {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
            crate::logging::warn(&format!(
                "Failed to read RLIMIT_NOFILE: {}",
                std::io::Error::last_os_error()
            ));
            return;
        }

        // `rlim_cur`/`rlim_max` are `u64` on Linux/macOS but `i64` on some
        // platforms (e.g. FreeBSD), so cast explicitly to keep builds portable.
        // The cast is a no-op (and clippy-flagged) where the field is already
        // `u64`, hence the allow.
        #[allow(clippy::unnecessary_cast)]
        let current: u64 = limit.rlim_cur as u64;
        #[allow(clippy::unnecessary_cast)]
        let hard: u64 = limit.rlim_max as u64;
        let Some(desired) = desired_nofile_soft_limit(current, hard, minimum_soft_limit) else {
            return;
        };

        let updated = libc::rlimit {
            rlim_cur: desired as libc::rlim_t,
            rlim_max: limit.rlim_max,
        };
        if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &updated) } == 0 {
            crate::logging::info(&format!(
                "Raised RLIMIT_NOFILE soft limit from {} to {} (hard={})",
                current, desired, hard
            ));
        } else {
            crate::logging::warn(&format!(
                "Failed to raise RLIMIT_NOFILE from {} toward {} (hard={}): {}",
                current,
                desired,
                hard,
                std::io::Error::last_os_error()
            ));
        }
    }

    #[cfg(not(unix))]
    {
        let _ = minimum_soft_limit;
    }
}

/// The user's home directory: `$HOME` on Unix, the profile folder on Windows (where `HOME` is normally
/// unset). Callers read this instead of `std::env::var("HOME")`.
pub fn user_home_dir() -> Option<std::path::PathBuf> {
    dirs::home_dir()
}

/// Check if a process is running by PID.
///
/// On Unix, uses `kill(pid, 0)` to check without sending a signal.
/// On Windows, uses OpenProcess to query the process.
pub fn is_process_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let result = unsafe { libc::kill(pid as i32, 0) };
        if result == 0 {
            return true;
        }
        let err = std::io::Error::last_os_error();
        !matches!(err.raw_os_error(), Some(code) if code == libc::ESRCH)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            let mut exit_code = 0u32;
            let ok = GetExitCodeProcess(handle, &mut exit_code);
            CloseHandle(handle);
            ok != 0 && exit_code == STILL_ACTIVE as u32
        }
    }
}

/// Memory-accounting reads for the process-group cap. macOS reads libproc, Linux reads `/proc`;
/// elsewhere every function answers `None` (no cap is enforced).
#[cfg(target_os = "macos")]
mod proc_memory {
    #[repr(C)]
    struct ProcTaskInfo {
        virtual_size: u64,
        resident_size: u64,
        total_user: u64,
        total_system: u64,
        threads_user: u64,
        threads_system: u64,
        policy: i32,
        faults: i32,
        pageins: i32,
        cow_faults: i32,
        messages_sent: i32,
        messages_received: i32,
        syscalls_mach: i32,
        syscalls_unix: i32,
        context_switches: i32,
        thread_count: i32,
        running_count: i32,
        priority: i32,
    }

    #[link(name = "proc")]
    unsafe extern "C" {
        fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut ProcTaskInfo, size: i32) -> i32;
        fn proc_listpids(kind: u32, info: u32, buffer: *mut i32, size: i32) -> i32;
    }

    pub fn process_rss_bytes(pid: u32) -> Option<u64> {
        let mut info = std::mem::MaybeUninit::<ProcTaskInfo>::uninit();
        let size = std::mem::size_of::<ProcTaskInfo>() as i32;
        // PROC_PIDTASKINFO = 4 in Apple's libproc API.
        let read = unsafe { proc_pidinfo(pid as i32, 4, 0, info.as_mut_ptr(), size) };
        (read == size).then(|| unsafe { info.assume_init() }.resident_size)
    }

    pub fn process_group_rss_bytes(pgid: u32) -> Option<u64> {
        // PROC_PGRP_ONLY = 2. A null buffer asks for the byte size of the list.
        let wanted = unsafe { proc_listpids(2, pgid, std::ptr::null_mut(), 0) };
        if wanted <= 0 {
            return None;
        }
        // Room for members that start between the two calls.
        let mut pids = vec![0i32; wanted as usize / size_of::<i32>() + 16];
        let bytes = unsafe { proc_listpids(2, pgid, pids.as_mut_ptr(), (pids.len() * size_of::<i32>()) as i32) };
        if bytes <= 0 {
            return None;
        }
        let members = &pids[..bytes as usize / size_of::<i32>()];
        Some(members.iter().filter(|p| **p > 0).filter_map(|p| process_rss_bytes(*p as u32)).sum())
    }

    fn sysctl<T: Default>(name: &std::ffi::CStr) -> Option<T> {
        let mut value = T::default();
        let mut len = size_of::<T>();
        let rc = unsafe { libc::sysctlbyname(name.as_ptr(), (&raw mut value).cast(), &mut len, std::ptr::null_mut(), 0) };
        (rc == 0 && len == size_of::<T>()).then_some(value)
    }

    pub fn system_memory() -> Option<super::SystemMemory> {
        let total: u64 = sysctl(c"hw.memsize")?;
        // The kernel's own available-memory figure (the one memory pressure and jetsam act on), as a
        // percentage of physical memory: free, purgeable and reclaimable file pages, not wired or
        // compressed ones.
        let available_percent: u32 = sysctl(c"kern.memorystatus_level")?;
        Some(super::SystemMemory { total, available: total / 100 * u64::from(available_percent.min(100)) })
    }
}

#[cfg(target_os = "linux")]
mod proc_memory {
    fn page_size() -> u64 {
        unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as u64
    }

    /// `(pgrp, resident pages)` from `/proc/<pid>/stat`. The command name sits in parentheses and may
    /// hold spaces, so the fields are counted from the last `)`.
    fn pgrp_and_rss_pages(stat: &str) -> Option<(u32, u64)> {
        let fields: Vec<&str> = stat[stat.rfind(')')? + 1..].split_whitespace().collect();
        // After the name: state ppid pgrp ... (rss is field 24 overall, index 21 here).
        Some((fields.get(2)?.parse().ok()?, fields.get(21)?.parse().ok()?))
    }

    pub fn process_rss_bytes(pid: u32) -> Option<u64> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        Some(pgrp_and_rss_pages(&stat)?.1 * page_size())
    }

    pub fn process_group_rss_bytes(pgid: u32) -> Option<u64> {
        let mut pages = 0;
        let mut members = 0;
        for entry in std::fs::read_dir("/proc").ok()?.flatten() {
            if !entry.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else { continue };
            if let Some((group, rss)) = pgrp_and_rss_pages(&stat)
                && group == pgid
            {
                members += 1;
                pages += rss;
            }
        }
        (members > 0).then(|| pages * page_size())
    }

    fn meminfo_bytes(meminfo: &str, key: &str) -> Option<u64> {
        let kib: u64 = meminfo.lines().find_map(|l| l.strip_prefix(key))?.split_whitespace().next()?.parse().ok()?;
        Some(kib * 1024)
    }

    /// `(limit, usage, inactive file cache)` of this process's memory cgroup, when it has a limit.
    /// v2 reads the unified hierarchy at the path in `/proc/self/cgroup`; v1 reads the memory
    /// controller's own mount. Inactive file pages are reclaimable, so they count as available
    /// (the working-set definition the kubelet and `docker stats` use).
    fn cgroup_memory() -> Option<(u64, u64, u64)> {
        let read = |path: String| std::fs::read_to_string(path).ok();
        let stat_value = |stat: &str, key: &str| {
            stat.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(' ')?.trim().parse::<u64>().ok())
        };
        let membership = read("/proc/self/cgroup".into())?;
        if let Some(path) = membership.lines().find_map(|l| l.strip_prefix("0::")) {
            let dir = format!("/sys/fs/cgroup{}", path.trim_end_matches('/'));
            let limit = read(format!("{dir}/memory.max"))?.trim().parse().ok()?; // "max" = no limit
            let usage = read(format!("{dir}/memory.current"))?.trim().parse().ok()?;
            let inactive = read(format!("{dir}/memory.stat")).and_then(|s| stat_value(&s, "inactive_file"));
            return Some((limit, usage, inactive.unwrap_or(0)));
        }
        let dir = "/sys/fs/cgroup/memory";
        let limit = read(format!("{dir}/memory.limit_in_bytes"))?.trim().parse().ok()?;
        let usage = read(format!("{dir}/memory.usage_in_bytes"))?.trim().parse().ok()?;
        let inactive = read(format!("{dir}/memory.stat")).and_then(|s| stat_value(&s, "total_inactive_file"));
        Some((limit, usage, inactive.unwrap_or(0)))
    }

    /// Host `MemTotal`/`MemAvailable`, narrowed to the memory cgroup's limit when there is one
    /// (a container sees its own budget, not the host's). An unlimited v1 cgroup reports a huge
    /// limit, which the `min` against the host ignores.
    pub fn system_memory() -> Option<super::SystemMemory> {
        let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
        let host = super::SystemMemory {
            total: meminfo_bytes(&meminfo, "MemTotal:")?,
            available: meminfo_bytes(&meminfo, "MemAvailable:")?,
        };
        Some(narrow_to_cgroup(host, cgroup_memory()))
    }

    pub(super) fn narrow_to_cgroup(host: super::SystemMemory, cgroup: Option<(u64, u64, u64)>) -> super::SystemMemory {
        let Some((limit, usage, inactive)) = cgroup else { return host };
        let cgroup_available = limit.saturating_sub(usage.saturating_sub(inactive));
        super::SystemMemory { total: host.total.min(limit), available: host.available.min(cgroup_available) }
    }

    #[cfg(test)]
    #[test]
    fn stat_fields_survive_spaces_in_the_command_name() {
        let stat = "42 (my (odd) cmd) S 1 77 77 0 -1 4194560 100 0 0 0 1 1 0 0 20 0 1 0 5 1000 321 18446744073709551615";
        assert_eq!(pgrp_and_rss_pages(stat), Some((77, 321)));
    }

    #[cfg(test)]
    #[test]
    fn a_cgroup_limit_narrows_total_and_available() {
        use super::SystemMemory;
        const GIB: u64 = 1 << 30;
        let host = SystemMemory { total: 64 * GIB, available: 40 * GIB };
        assert_eq!(narrow_to_cgroup(host, None), host);
        // 8 GiB container using 7 GiB, 2 GiB of it reclaimable cache: 3 GiB left.
        assert_eq!(narrow_to_cgroup(host, Some((8 * GIB, 7 * GIB, 2 * GIB))), SystemMemory { total: 8 * GIB, available: 3 * GIB });
        // An unlimited v1 cgroup (huge limit) leaves the host figures.
        assert_eq!(narrow_to_cgroup(host, Some((u64::MAX / 2, 30 * GIB, 0))), host);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod proc_memory {
    pub fn process_rss_bytes(_pid: u32) -> Option<u64> {
        None
    }
    pub fn process_group_rss_bytes(_pgid: u32) -> Option<u64> {
        None
    }
    pub fn system_memory() -> Option<super::SystemMemory> {
        None
    }
}

/// Resident memory of one process, in bytes.
pub use proc_memory::process_rss_bytes;
/// Resident memory summed over every process in group `pgid`; `None` when the group is gone.
pub use proc_memory::process_group_rss_bytes;
/// Total and available memory, in bytes, as the kernel reports them for this process (on Linux,
/// narrowed to its memory cgroup).
pub use proc_memory::system_memory;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemMemory {
    pub total: u64,
    /// Memory that can be handed out without swapping: free plus reclaimable cache.
    pub available: u64,
}

/// Send a signal to an entire detached process group/session led by `pid`.
///
/// On Unix, detached tasks are spawned with `setsid()`, so the leader PID is
/// also the process-group/session ID. Signaling `-pid` reaches the full tree.
pub fn signal_detached_process_group(pid: u32, signal: i32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let rc = unsafe { libc::kill(-(pid as i32), signal) };
        if rc == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    #[cfg(windows)]
    {
        let _ = signal;
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            CREATE_NO_WINDOW, OpenProcess, PROCESS_TERMINATE, TerminateProcess,
        };

        // Detached commands commonly run through cmd.exe or PowerShell. Killing
        // only that shell leaves compilers, test runners, and other descendants
        // alive. taskkill's /T flag walks the Windows process tree. Keep the
        // direct Win32 termination below as a fallback if taskkill is missing or
        // the tree operation fails.
        let tree_status = std::process::Command::new("taskkill.exe")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
        if tree_status.is_ok_and(|status| status.success()) {
            return Ok(());
        }
        // taskkill can report failure when a descendant exits while it walks the
        // tree, even though it successfully terminated the leader and remaining
        // descendants. Avoid turning that benign race into a misleading access
        // denied error from the direct-handle fallback.
        for _ in 0..20 {
            if !is_process_running(pid) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if handle.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let ok = TerminateProcess(handle, 1);
            CloseHandle(handle);
            if ok == 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }
}

/// Best-effort non-blocking reap for a child process owned by the current process.
///
/// Returns:
/// - `Ok(Some(exit_code))` if the child exited and was reaped now
/// - `Ok(None)` if it is still running or is not our child
pub fn try_reap_child_process(pid: u32) -> std::io::Result<Option<i32>> {
    #[cfg(unix)]
    {
        let mut status = 0;
        let rc = unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) };
        if rc == 0 {
            return Ok(None);
        }
        if rc == -1 {
            let err = std::io::Error::last_os_error();
            if matches!(err.raw_os_error(), Some(code) if code == libc::ECHILD) {
                return Ok(None);
            }
            return Err(err);
        }

        if libc::WIFEXITED(status) {
            Ok(Some(libc::WEXITSTATUS(status)))
        } else if libc::WIFSIGNALED(status) {
            Ok(Some(128 + libc::WTERMSIG(status)))
        } else {
            Ok(Some(-1))
        }
    }
    #[cfg(windows)]
    {
        let _ = pid;
        Ok(None)
    }
}

/// Atomically swap a symlink by creating a temp symlink and renaming.
///
/// On Unix: creates temp symlink, then renames over target (atomic).
/// On Windows: removes target, copies source (not atomic, but best effort).
pub fn atomic_symlink_swap(src: &Path, dst: &Path, temp: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(temp);
        std::os::unix::fs::symlink(src, temp)?;
        std::fs::rename(temp, dst)?;
    }
    #[cfg(windows)]
    {
        let _ = std::fs::remove_file(temp);
        let _ = std::fs::remove_file(dst);
        std::fs::copy(src, dst).map(|_| ())?;
    }
    Ok(())
}

/// Spawn a process detached from the current client session.
///
/// This is used for launching new terminal windows (for `/resume`, `/split`,
/// crash restore, etc.) so the new client survives if the invoking factr
/// process exits or its terminal closes.
pub fn spawn_detached(cmd: &mut std::process::Command) -> std::io::Result<std::process::Child> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};

        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
    }

    cmd.spawn()
}

/// Reap a detached child without blocking the caller.
pub fn reap_detached(child: std::process::Child) {
    #[cfg(unix)]
    {
        let mut child = child;
        let _ = std::thread::Builder::new()
            .name("factr-detached-child".to_string())
            .spawn(move || {
                let _ = child.wait();
            });
    }

    #[cfg(windows)]
    {
        // Closing the process handle is sufficient on Windows. Unlike Unix,
        // the child does not need to be waited on to avoid a zombie process.
        drop(child);
    }
}

#[cfg(windows)]
fn spawn_replacement_process(
    cmd: &mut std::process::Command,
) -> std::io::Result<std::process::Child> {
    cmd.spawn()
}

/// Replace the current process with a new command (exec on Unix).
///
/// On Unix, this calls exec() which never returns on success.
/// On Windows, this spawns the process and exits.
///
/// Returns an error only if the operation fails. On success (Unix exec),
/// this function never returns.
pub fn replace_process(cmd: &mut std::process::Command) -> std::io::Error {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = cmd.exec();
        crate::logging::error(&format!(
            "replace_process failed: {} ({})",
            err,
            crate::util::process_fd_diagnostic_snapshot()
        ));
        err
    }
    #[cfg(windows)]
    {
        match spawn_replacement_process(cmd) {
            Ok(_child) => std::process::exit(0),
            Err(e) => e,
        }
    }
}

#[cfg(test)]
#[path = "platform_tests.rs"]
mod platform_tests;
