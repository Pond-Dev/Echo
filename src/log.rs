//! A plain text log of one run, written beside the executable.
//!
//! Truncated on every start: the file always holds the current run and
//! nothing else, so reading it never means finding the newest of a hundred
//! session files first.
//!
//! Timestamps are seconds since this run started rather than a wall clock.
//! For a diagnostic that answers "what happened during that session" the
//! elapsed time is the useful axis, and the file's own modified time already
//! records when the run was.
//!
//! A log that cannot be opened degrades to console-only. Losing the trace is
//! worth less than refusing to run.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::Instant;

const FILE_NAME: &str = "echo.log";

pub struct Log {
    /// Buffered, because this is written from the middle of the frame loop
    /// and an unbuffered file is one write into the kernel per line. A report
    /// is fifteen lines; a roster change is one per player.
    file: Option<BufWriter<File>>,
    started: Instant,
    path: Option<PathBuf>,
}

impl Log {
    /// Create (or truncate) the log beside the running executable.
    pub fn create() -> Self {
        let path = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join(FILE_NAME)));
        let file = path
            .as_ref()
            .and_then(|path| File::create(path).ok())
            .map(BufWriter::new);
        Self {
            file,
            started: Instant::now(),
            path,
        }
    }

    /// Where the log ended up, for telling the user at startup.
    pub fn path(&self) -> Option<&std::path::Path> {
        self.file
            .is_some()
            .then_some(self.path.as_deref())
            .flatten()
    }

    /// Show on the console *and* keep in the log.
    pub fn say(&mut self, message: &str) {
        println!("{message}");
        self.record(message);
    }

    /// Keep in the log only — for anything too frequent to print.
    ///
    /// Blank lines and the padding newlines that space out console output are
    /// dropped: they are layout for a screen, and in a file they only break
    /// the one-entry-per-line shape that makes the log readable.
    pub fn record(&mut self, message: &str) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        let elapsed = self.started.elapsed().as_secs_f64();
        // Trailing whitespace goes, leading whitespace stays: the breakdown
        // nests sub-stages under the stage they belong to, and flattening that
        // puts a share of the drawing next to a share of the frame with
        // nothing to tell them apart.
        for line in message
            .lines()
            .map(str::trim_end)
            .filter(|l| !l.trim().is_empty())
        {
            // A failed write means the log is gone, not that the run stops.
            let _ = writeln!(file, "[{elapsed:9.3}] {line}");
        }
        // Once per record rather than once per line, and still before
        // returning: a crash must not take the last thing written with it,
        // which is the whole reason to read this file at all.
        let _ = file.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::Log;

    #[test]
    fn a_log_that_could_not_be_opened_reports_no_path_and_still_accepts_writes() {
        let mut log = Log {
            file: None,
            started: std::time::Instant::now(),
            path: Some(std::path::PathBuf::from("nowhere/echo.log")),
        };

        // A path is only reported when there is really a file behind it,
        // otherwise startup would point the user at something that does not
        // exist.
        assert!(log.path().is_none());

        // And recording must stay a no-op rather than panic.
        log.record("dropped on the floor");
    }

    #[test]
    fn console_padding_never_reaches_the_file_as_blank_or_split_lines() {
        let file = std::env::temp_dir().join("echo-log-padding-test.log");
        let mut log = Log {
            file: Some(std::io::BufWriter::new(
                std::fs::File::create(&file).expect("temp log"),
            )),
            started: std::time::Instant::now(),
            path: Some(file.clone()),
        };

        log.record("\nMove your mouse\nCtrl+C to stop.\n");
        log.record("   ");

        let written = std::fs::read_to_string(&file).expect("read back");
        let lines: Vec<&str> = written.lines().collect();
        assert_eq!(lines.len(), 2, "got {lines:?}");
        assert!(lines[0].ends_with("Move your mouse"));
        assert!(lines[1].ends_with("Ctrl+C to stop."));
        assert!(lines.iter().all(|line| line.starts_with('[')));

        let _ = std::fs::remove_file(&file);
    }
}
