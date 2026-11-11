//! Rolling file log: one file per component per day, size-capped, old files pruned. Every line passes through
//! [`redact`](crate::redact::redact). Lines are English with UTC timestamps. Logging never fails the process:
//! I/O errors drop the line.

use crate::redact::redact;
use log::{Level, LevelFilter, Log, Metadata, Record};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use time::OffsetDateTime;
use time::macros::format_description;

pub struct FileLogger {
    directory: PathBuf,
    component: String,
    level: LevelFilter,
    max_bytes: u64,
    retained_files: usize,
    state: Mutex<Option<(PathBuf, File)>>,
}

impl FileLogger {
    pub fn new(directory: impl Into<PathBuf>, component: &str, level: LevelFilter) -> Self {
        Self {
            directory: directory.into(),
            component: component.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect(),
            level,
            max_bytes: 10 * 1024 * 1024,
            retained_files: 14,
            state: Mutex::new(None),
        }
    }

    #[must_use]
    pub fn with_limits(mut self, max_bytes: u64, retained_files: usize) -> Self {
        self.max_bytes = max_bytes;
        self.retained_files = retained_files.max(1);
        self
    }

    /// Installs this logger as the process logger. Fails only when one is already installed.
    pub fn install(self) -> Result<(), log::SetLoggerError> {
        log::set_max_level(self.level);
        log::set_boxed_logger(Box::new(self))
    }

    fn write_line(&self, now: OffsetDateTime, line: &str) -> std::io::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let day = now.format(format_description!("[year][month][day]")).unwrap_or_default();
        let base = self.directory.join(format!("{}-{day}", self.component));
        let reuse = matches!(&*state, Some((path, file))
            if path.to_string_lossy().starts_with(&*base.to_string_lossy())
                && file.metadata().map(|m| m.len() < self.max_bytes).unwrap_or(false));
        if !reuse {
            let same_day_full =
                matches!(&*state, Some((path, _)) if path.to_string_lossy().starts_with(&*base.to_string_lossy()));
            let mut path = PathBuf::from(format!("{}.log", base.display()));
            if same_day_full {
                let mut n = 1;
                while Path::new(&format!("{}.{n}.log", base.display())).exists() {
                    n += 1;
                }
                path = PathBuf::from(format!("{}.{n}.log", base.display()));
            }
            fs::create_dir_all(&self.directory)?;
            let file = OpenOptions::new().create(true).append(true).open(&path)?;
            *state = Some((path, file));
            self.prune();
        }
        if let Some((_, file)) = state.as_mut() {
            writeln!(file, "{line}")?;
        }
        Ok(())
    }

    fn prune(&self) {
        let prefix = format!("{}-", self.component);
        let Ok(entries) = fs::read_dir(&self.directory) else { return };
        let mut files: Vec<_> = entries
            .filter_map(Result::ok)
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                name.starts_with(&prefix) && name.ends_with(".log")
            })
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        files.sort_by_key(|f| std::cmp::Reverse(f.0));
        for (_, path) in files.into_iter().skip(self.retained_files) {
            let _ = fs::remove_file(path);
        }
    }
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let now = OffsetDateTime::now_utc();
        let stamp = now
            .format(format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"))
            .unwrap_or_default();
        let tag = match record.level() {
            Level::Trace => "TRC",
            Level::Debug => "DBG",
            Level::Info => "INF",
            Level::Warn => "WRN",
            Level::Error => "ERR",
        };
        let line = format!("{stamp} {tag} {}: {}", record.target(), redact(&record.args().to_string()));
        let _ = self.write_line(now, &line);
    }

    fn flush(&self) {
        if let Some((_, file)) = self.state.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
            let _ = file.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(logger: &FileLogger, message: &str) {
        logger.log(&Record::builder().args(format_args!("{message}")).level(Level::Info).target("test").build());
    }

    #[test]
    fn writes_redacted_lines_and_rolls_by_size() {
        let dir = tempfile::tempdir().unwrap();
        let logger = FileLogger::new(dir.path(), "app", LevelFilter::Info).with_limits(200, 2);
        record(&logger, "connecting with password=hunter2");
        for i in 0..20 {
            record(&logger, &format!("line {i} with enough text to fill the small file quickly"));
        }
        let mut names: Vec<String> =
            fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert!(names.len() <= 2, "old files are pruned: {names:?}");
        assert!(names.iter().all(|n| n.starts_with("app-") && n.ends_with(".log")));
        let all: String = names.iter().map(|n| fs::read_to_string(dir.path().join(n)).unwrap()).collect();
        assert!(!all.contains("hunter2"));
        assert!(all.contains(" INF test: line 19"));
    }

    #[test]
    fn levels_below_the_minimum_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let logger = FileLogger::new(dir.path(), "svc", LevelFilter::Warn);
        record(&logger, "informational");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
