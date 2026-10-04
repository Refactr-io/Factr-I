//! Memory guard for the process group of a bash command, foreground or background: a group that
//! is driving the machine out of memory is killed with a message that tells the model how to do
//! the work instead. A large but healthy build (cargo -j N, webpack, jest) is left alone.

use factr_base::platform::SystemMemory;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

const GIB: u64 = 1 << 30;
const POLL: Duration = Duration::from_millis(500);

/// When a group is killed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Limit {
    /// `terminal.max_memory_mb`: the user's explicit cap on the group's resident memory.
    Fixed(u64),
    /// No configured cap: kill only under real memory pressure (see [`over_under_pressure`]).
    Pressure,
}

pub(super) fn limit() -> Limit {
    configured_bytes().map_or(Limit::Pressure, Limit::Fixed)
}

/// `terminal.max_memory_mb` in bytes, when set.
pub(super) fn configured_bytes() -> Option<u64> {
    factr_base::factr_config::current().terminal.max_memory_mb.map(|mb| mb.saturating_mul(1 << 20))
}

/// The pressure rule. The machine is short of memory when what is available falls below one
/// core's share of RAM (`total / cores`): a parallel build runs about one job per core, so less
/// than an average job's share left means the next allocation goes to swap or the OOM killer.
/// The group is to blame when it holds at least half of the memory in use; otherwise something
/// else is the cause and killing the command would not help. Summed RSS counts pages shared
/// between members more than once, which only makes the blame test stricter on the group's side;
/// the availability test is what keeps a healthy build alive.
fn over_under_pressure(group_rss: u64, memory: SystemMemory, cores: u64) -> bool {
    let floor = memory.total / cores.max(1);
    let in_use = memory.total.saturating_sub(memory.available);
    memory.available < floor && group_rss.saturating_mul(2) >= in_use
}

fn cores() -> u64 {
    std::thread::available_parallelism().map_or(1, |n| n.get() as u64)
}

fn gib(bytes: u64) -> f64 {
    (bytes as f64 / GIB as f64 * 100.0).round() / 100.0
}

const ADVICE: &str = "reduce parallelism or stream the data";

fn fixed_notice(cap: u64) -> String {
    format!("killed: process group exceeded the {} GiB terminal.max_memory_mb cap; {ADVICE}", gib(cap))
}

fn pressure_notice(group_rss: u64, memory: SystemMemory) -> String {
    format!(
        "killed: process group held {} GiB while system memory ran low ({} of {} GiB available); {ADVICE}",
        gib(group_rss),
        gib(memory.available),
        gib(memory.total)
    )
}

/// The kill message, set once the watched group was killed.
#[derive(Clone, Default)]
pub(super) struct MemoryWatch {
    notice: Arc<OnceLock<String>>,
}

impl MemoryWatch {
    pub(super) fn notice(&self) -> Option<String> {
        self.notice.get().cloned()
    }
}

/// Watch group `pgid` under the configured [`limit`]. With `notice_file`, the kill message is
/// also appended there (for commands whose output goes straight to a file, which nothing in this
/// process reads back).
#[cfg(unix)]
pub(super) fn watch(pgid: u32, notice_file: Option<PathBuf>) -> MemoryWatch {
    watch_with(pgid, limit(), notice_file)
}

#[cfg(unix)]
fn group_alive(pgid: u32) -> bool {
    // Signal 0 checks existence only; EPERM still means the group exists.
    i32::try_from(pgid).is_ok_and(|pgid| {
        let rc = unsafe { libc::kill(-pgid, 0) };
        rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    })
}

/// Watch group `pgid` until it is gone; the first time it breaks `limit`, SIGKILL the group.
/// Under [`Limit::Pressure`] the cheap system-wide read comes first, and the group's members are
/// only summed when memory is actually short.
#[cfg(unix)]
fn watch_with(pgid: u32, limit: Limit, notice_file: Option<PathBuf>) -> MemoryWatch {
    let watch = MemoryWatch::default();
    let flag = watch.clone();
    let cores = cores();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(POLL).await;
            let notice = match limit {
                Limit::Fixed(cap) => match factr_base::platform::process_group_rss_bytes(pgid) {
                    None => return,
                    Some(rss) => (rss > cap).then(|| fixed_notice(cap)),
                },
                Limit::Pressure => match factr_base::platform::system_memory() {
                    Some(memory) if memory.available < memory.total / cores => {
                        match factr_base::platform::process_group_rss_bytes(pgid) {
                            None => return,
                            Some(rss) => over_under_pressure(rss, memory, cores).then(|| pressure_notice(rss, memory)),
                        }
                    }
                    _ if !group_alive(pgid) => return,
                    _ => None,
                },
            };
            if let Some(notice) = notice {
                // Set before the kill, so whoever sees the group die also sees why.
                let _ = flag.notice.set(notice.clone());
                let _ = factr_base::platform::signal_detached_process_group(pgid, libc::SIGKILL);
                if let Some(path) = &notice_file {
                    use std::io::Write;
                    if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(path) {
                        let _ = writeln!(file, "\n--- {notice} ---");
                    }
                }
                return;
            }
        }
    });
    watch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_needs_both_low_memory_and_a_group_holding_half_of_what_is_in_use() {
        let machine = |available| SystemMemory { total: 16 * GIB, available };
        // 8 cores: the floor is 2 GiB.
        // Plenty available: a 12 GiB build is left alone.
        assert!(!over_under_pressure(12 * GIB, machine(3 * GIB), 8));
        // Short of memory and the group holds most of what is in use: killed.
        assert!(over_under_pressure(8 * GIB, machine(GIB), 8));
        // Short of memory but something else holds it: the command is not the cause.
        assert!(!over_under_pressure(2 * GIB, machine(GIB), 8));
        // A cgroup-narrowed 4 GiB container on 2 cores: floor 2 GiB.
        let container = SystemMemory { total: 4 * GIB, available: GIB };
        assert!(over_under_pressure(2 * GIB, container, 2));
    }

    #[test]
    fn notices_name_the_numbers_and_give_advice_that_fits_builds() {
        assert_eq!(fixed_notice(GIB / 2), "killed: process group exceeded the 0.5 GiB terminal.max_memory_mb cap; reduce parallelism or stream the data");
        assert_eq!(
            pressure_notice(6 * GIB, SystemMemory { total: 8 * GIB, available: GIB / 4 }),
            "killed: process group held 6 GiB while system memory ran low (0.25 of 8 GiB available); reduce parallelism or stream the data"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_group_over_a_fixed_cap_is_killed_and_a_small_one_is_left_alone() {
        let spawn = |script: &str| {
            let mut cmd = std::process::Command::new("perl");
            cmd.args(["-e", script]);
            std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
            cmd.spawn().unwrap()
        };
        // Holds 300 MB (the string is written, so the pages are resident) under a 100 MB cap.
        let mut big = spawn("my $x = 'x' x 300_000_000; sleep 30");
        let big_watch = watch_with(big.id(), Limit::Fixed(100 << 20), None);
        let status = tokio::time::timeout(Duration::from_secs(20), tokio::task::spawn_blocking(move || big.wait()))
            .await
            .expect("the group is killed well before its sleep ends")
            .unwrap()
            .unwrap();
        assert!(!status.success());
        assert_eq!(big_watch.notice(), Some(fixed_notice(100 << 20)));

        let mut small = spawn("sleep 2");
        let small_watch = watch_with(small.id(), Limit::Fixed(100 << 20), None);
        small.wait().unwrap();
        assert_eq!(small_watch.notice(), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn without_pressure_a_large_group_is_left_alone() {
        let mut cmd = std::process::Command::new("perl");
        cmd.args(["-e", "my $x = 'x' x 300_000_000; sleep 2"]);
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = cmd.spawn().unwrap();
        let watch = watch_with(child.id(), Limit::Pressure, None);
        assert!(tokio::task::spawn_blocking(move || child.wait()).await.unwrap().unwrap().success());
        assert_eq!(watch.notice(), None, "a test machine is not under memory pressure from 300 MB");
    }
}
