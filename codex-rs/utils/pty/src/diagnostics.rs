//! Opt-in process diagnostics contain only static stages, PIDs, and OS status.
//! Never print command arguments, environment entries, or file contents here.

use std::io;
use std::process::ExitStatus;

pub(crate) const ENV_VAR: &str = "CODEX_HARMONY_PROCESS_DIAGNOSTICS";

pub(crate) fn enabled() -> bool {
    std::env::var_os(ENV_VAR).is_some_and(|value| value == "1")
}

#[cfg(target_os = "linux")]
pub(crate) fn stage(enabled: bool, stage: &'static str) {
    if enabled {
        eprintln!("[codex-process] pid={} stage={stage}", std::process::id());
    }
}

/// Describe the native wait result before compatibility exit-code normalization.
/// In particular, an explicit exit 159 is different from a signal termination.
pub fn describe_exit_status(status: ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        format!(
            "exit_code={:?} signal={:?} raw_wait_status={} core_dumped={}",
            status.code(),
            status.signal(),
            status.into_raw(),
            status.core_dumped()
        )
    }
    #[cfg(not(unix))]
    {
        format!("exit_code={:?} native_status={status}", status.code())
    }
}

pub(crate) fn child_started(stage: &'static str, child_pid: Option<u32>) {
    if enabled() {
        eprintln!(
            "[codex-process] pid={} child_pid={child_pid:?} stage={stage}",
            std::process::id(),
        );
    }
}

pub(crate) fn wait_status(stage: &'static str, child_pid: Option<u32>, status: ExitStatus) {
    if enabled() {
        eprintln!(
            "[codex-process] pid={} child_pid={child_pid:?} stage={stage} {}",
            std::process::id(),
            describe_exit_status(status)
        );
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn forward_helper_stages(child_pid: Option<u32>, output: &[u8]) {
    for line in output.split(|byte| *byte == b'\n') {
        let Ok(line) = std::str::from_utf8(line) else {
            continue;
        };
        if let Some(stage) = helper_stage(child_pid, line) {
            child_started(stage, child_pid);
        }
    }
}

#[cfg(target_os = "linux")]
fn helper_stage(child_pid: Option<u32>, line: &str) -> Option<&'static str> {
    let mut fields = line.strip_prefix("[codex-process] ")?.split_whitespace();
    let pid = fields.next()?.strip_prefix("pid=")?.parse::<u32>().ok()?;
    if Some(pid) != child_pid {
        return None;
    }
    // Reconstruct only fixed labels. Never forward arbitrary helper stderr,
    // loader messages, environment values, or extra fields in the original line.
    match fields.next()?.strip_prefix("stage=")? {
        "helper-arguments" => Some("helper-arguments"),
        "helper-control-cloexec" => Some("helper-control-cloexec"),
        "helper-detach-terminal" => Some("helper-detach-terminal"),
        "helper-parent-death-signal" => Some("helper-parent-death-signal"),
        "helper-configure-pty" => Some("helper-configure-pty"),
        "helper-descriptor-cleanup" => Some("helper-descriptor-cleanup"),
        "helper-environment-transfer" => Some("helper-environment-transfer"),
        "helper-target-exec" => Some("helper-target-exec"),
        _ => None,
    }
}

/// Called after fork: use only stack storage and async-signal-safe libc calls.
/// The opt-in is captured in the parent; do not inspect the environment here.
#[cfg(unix)]
pub(crate) fn pre_exec_stage(enabled: bool, stage: &'static [u8]) {
    if !enabled {
        return;
    }
    let prefix = b"[codex-process] child_pid=";
    let separator = b" stage=";
    let mut message = [0_u8; 256];
    if stage.len() > message.len() - prefix.len() - 10 - separator.len() - 1 {
        return;
    }
    // SAFETY: getpid is async-signal-safe and has no preconditions.
    let mut pid = unsafe { libc::getpid() } as u32;
    let mut digits = [0_u8; 10];
    let mut first = digits.len();
    loop {
        first -= 1;
        digits[first] = b'0' + (pid % 10) as u8;
        pid /= 10;
        if pid == 0 {
            break;
        }
    }
    let mut end = prefix.len();
    message[..end].copy_from_slice(prefix);
    for part in [&digits[first..], separator.as_slice(), stage] {
        message[end..end + part.len()].copy_from_slice(part);
        end += part.len();
    }
    message[end] = b'\n';
    // SAFETY: write is async-signal-safe; all bytes are in this stack frame.
    // Diagnostics are best-effort and never change the setup result.
    unsafe { libc::write(libc::STDERR_FILENO, message.as_ptr().cast(), end + 1) };
}

pub(crate) fn failure(enabled: bool, stage: &'static str, error: &io::Error) {
    if enabled {
        eprintln!(
            "[codex-process] pid={} stage={stage} errno={:?} kind={:?}",
            std::process::id(),
            error.raw_os_error(),
            error.kind()
        );
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn explicit_159_does_not_claim_a_signal() {
        let detail = describe_exit_status(ExitStatus::from_raw(159 << 8));
        assert!(detail.contains("exit_code=Some(159) signal=None"));
        assert!(detail.contains("raw_wait_status=40704"));
    }

    #[test]
    fn signal_is_kept_separate_from_an_explicit_exit() {
        let detail = describe_exit_status(ExitStatus::from_raw(31));
        assert!(detail.contains("exit_code=None signal=Some(31)"));
        assert!(detail.contains("raw_wait_status=31"));
        assert!(!detail.contains("SIGSYS"));
    }

    #[test]
    fn pre_exec_diagnostics_report_the_real_child_pid() -> io::Result<()> {
        use std::os::unix::process::CommandExt;

        let mut command = std::process::Command::new("/bin/sh");
        command.args(["-c", "printf '%s' \"$$\""]);
        // SAFETY: The logger uses only stack storage, getpid, and write after fork.
        unsafe {
            command.pre_exec(|| {
                pre_exec_stage(true, b"diagnostic-fixture");
                Ok(())
            });
        }
        let output = command.output()?;
        assert!(output.status.success());
        let pid = String::from_utf8(output.stdout).expect("shell PID is ASCII");
        assert_ne!(pid, std::process::id().to_string());
        assert_eq!(
            String::from_utf8(output.stderr).expect("diagnostic is ASCII"),
            format!("[codex-process] child_pid={pid} stage=diagnostic-fixture\n"),
        );
        Ok(())
    }

    #[test]
    fn disabled_pre_exec_diagnostics_leave_child_output_unchanged() -> io::Result<()> {
        use std::os::unix::process::CommandExt;

        let mut command = std::process::Command::new("/bin/sh");
        command.args(["-c", "printf output; printf error >&2"]);
        // SAFETY: Disabled diagnostics return without performing a syscall.
        unsafe {
            command.pre_exec(|| {
                pre_exec_stage(false, b"diagnostic-fixture");
                Ok(())
            });
        }
        let output = command.output()?;
        assert!(output.status.success());
        assert_eq!(output.stdout, b"output");
        assert_eq!(output.stderr, b"error");
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn helper_forwarding_accepts_only_known_stages_from_the_expected_child() {
        assert_eq!(
            helper_stage(
                Some(42),
                "[codex-process] pid=42 stage=helper-descriptor-cleanup",
            ),
            Some("helper-descriptor-cleanup"),
        );
        for line in [
            "loader: private-path",
            "[codex-process] pid=7 stage=helper-arguments",
            "[codex-process] pid=42 stage=unknown-label",
            "[codex-process] pid=invalid stage=helper-arguments",
        ] {
            assert_eq!(helper_stage(Some(42), line), None);
        }
        assert_eq!(
            helper_stage(
                Some(42),
                "[codex-process] pid=42 stage=helper-arguments secret=ignored",
            ),
            Some("helper-arguments"),
        );
        assert_eq!(
            helper_stage(None, "[codex-process] pid=42 stage=helper-arguments"),
            None,
        );
    }
}
