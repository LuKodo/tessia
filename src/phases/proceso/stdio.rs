//! Keeps stdout clean for the event stream (Go redirected the phases'
//! stdout/stderr to stderr behind a mutex).
//!
//! The phases still `println!` their own progress (`thefactory` prints one
//! line per invoice). While the stream is open, the process's stdout (fd 1)
//! points at stderr, and the events are written to a private duplicate of the
//! real stdout, so a `--json` consumer only ever sees events. Dropping the
//! [`Stream`] restores fd 1. On non-Unix targets nothing is redirected.

use std::io::{IsTerminal, Write};

use super::events::{Events, Mode};

/// The event sink plus the stdout redirection that protects it.
pub struct Stream {
    pub events: Events,
    #[cfg(unix)]
    saved: Option<libc::c_int>,
}

/// Opens the event stream on the real stdout and redirects everything else
/// the process prints on stdout to stderr until the stream is dropped.
pub fn open(mode: Mode) -> Stream {
    #[cfg(unix)]
    if let Some((file, saved)) = redirect() {
        let tty = file.is_terminal();
        return Stream {
            events: Events::new(mode, Box::new(file), tty),
            saved: Some(saved),
        };
    }
    Stream {
        events: Events::stdout(mode),
        #[cfg(unix)]
        saved: None,
    }
}

/// Duplicates fd 1 for the events and points fd 1 at fd 2.
#[cfg(unix)]
fn redirect() -> Option<(std::fs::File, libc::c_int)> {
    use std::os::fd::FromRawFd;

    let _ = std::io::stdout().flush();
    // SAFETY: plain file-descriptor duplication; every fd returned by `dup`
    // is either owned by the returned `File`, kept in `saved` and closed in
    // `Drop`, or closed right here on failure.
    unsafe {
        let events_fd = libc::dup(1);
        if events_fd < 0 {
            return None;
        }
        let saved = libc::dup(1);
        if saved < 0 {
            libc::close(events_fd);
            return None;
        }
        if libc::dup2(2, 1) < 0 {
            libc::close(events_fd);
            libc::close(saved);
            return None;
        }
        Some((std::fs::File::from_raw_fd(events_fd), saved))
    }
}

#[cfg(unix)]
impl Drop for Stream {
    fn drop(&mut self) {
        let Some(saved) = self.saved.take() else {
            return;
        };
        let _ = std::io::stdout().flush();
        // SAFETY: `saved` is the duplicate of the original stdout made in `redirect`.
        unsafe {
            libc::dup2(saved, 1);
            libc::close(saved);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::process::Command;

    use super::*;
    use crate::phases::proceso::events::Event;

    const CHILD: &str = "PROCESO_STDIO_TEST_CHILD";

    /// Runs in a child process (so the redirection cannot disturb the other
    /// tests): prints chatter and one event while the stream is open.
    #[test]
    fn child_prints_chatter_and_an_event() {
        if std::env::var_os(CHILD).is_none() {
            return;
        }
        let stream = open(Mode::Json);
        println!("phase chatter");
        stream.events.emit(Event::new("run_end").status("OK"));
        drop(stream);
        println!("after the stream");
    }

    #[test]
    fn phase_chatter_goes_to_stderr_and_only_events_reach_stdout() {
        let exe = std::env::current_exe().expect("test binary");
        let output = Command::new(exe)
            .args([
                "--exact",
                "phases::proceso::stdio::tests::child_prints_chatter_and_an_event",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .output()
            .expect("child");
        let (stdout, stderr) = (
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(output.status.success(), "{stdout}\n{stderr}");
        assert!(stdout.contains("\"type\":\"run_end\""), "{stdout}");
        assert!(!stdout.contains("phase chatter"), "{stdout}");
        assert!(stderr.contains("phase chatter"), "{stderr}");
        assert!(
            stdout.contains("after the stream"),
            "fd 1 is restored: {stdout}"
        );
    }
}
