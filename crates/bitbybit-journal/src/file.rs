use crate::{ChangeRecord, ChangeStatus, Journal};
use bitbybit_core::{Error, ErrorCode, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
struct Line {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chain: Option<String>,
    #[serde(flatten)]
    record: ChangeRecord,
}

/// What reading the file found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Integrity {
    /// Lines whose chain value matched.
    pub verified_lines: u64,
    /// Lines without a chain (written by the .NET versions).
    pub legacy_lines: u64,
    /// Unparseable lines in the middle of the file (corruption). A torn *last* line is not counted here.
    pub corrupt_lines: u64,
    /// The last line was incomplete (interrupted write) and was ignored.
    pub torn_tail: bool,
    /// 1-based number of the first line whose chain did not match (edited or reordered file).
    pub chain_broken_at: Option<u64>,
}

impl Integrity {
    pub fn is_intact(&self) -> bool {
        self.corrupt_lines == 0 && self.chain_broken_at.is_none()
    }
}

#[derive(Default)]
struct State {
    records: HashMap<Uuid, ChangeRecord>,
    order: Vec<Uuid>,
    last_chain: String,
    next_seq: u64,
    file_len: u64,
    loaded: bool,
    integrity: Integrity,
}

pub struct JsonLinesJournal {
    path: PathBuf,
    state: Mutex<State>,
}

fn chain_of(previous: &str, record_json: &str) -> String {
    let digest = Sha256::new().chain_update(previous.as_bytes()).chain_update(record_json.as_bytes()).finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

impl JsonLinesJournal {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), state: Mutex::new(State::default()) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Integrity of the file as of the last read.
    pub fn integrity(&self) -> Result<Integrity> {
        self.with_state(|s| Ok(s.integrity.clone()))
    }

    fn with_state<T>(&self, f: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        // Another process may have appended (the app reads the broker's journal): reload when the size changed.
        let len = fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        if !state.loaded || len != state.file_len {
            *state = Self::load(&self.path)?;
        }
        f(&mut state)
    }

    fn load(path: &Path) -> Result<State> {
        let mut state = State { loaded: true, ..State::default() };
        let text = match fs::read(path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(state),
            Err(e) => return Err(e.into()),
        };
        state.file_len = text.len() as u64;
        let lines: Vec<&str> = text.lines().collect();
        for (index, raw) in lines.iter().enumerate() {
            if raw.trim().is_empty() {
                continue;
            }
            let Ok(line) = serde_json::from_str::<Line>(raw) else {
                if index + 1 == lines.len() {
                    state.integrity.torn_tail = true;
                } else {
                    state.integrity.corrupt_lines += 1;
                }
                continue;
            };
            match &line.chain {
                Some(chain) => {
                    let expected = chain_of(&state.last_chain, &serde_json::to_string(&line.record)?);
                    if *chain == expected && state.integrity.chain_broken_at.is_none() {
                        state.integrity.verified_lines += 1;
                    } else if state.integrity.chain_broken_at.is_none() {
                        state.integrity.chain_broken_at = Some(index as u64 + 1);
                    }
                    state.last_chain = chain.clone();
                }
                None => state.integrity.legacy_lines += 1,
            }
            state.next_seq = state.next_seq.max(line.seq.unwrap_or(0) + 1);
            if state.records.insert(line.record.id, line.record.clone()).is_none() {
                state.order.push(line.record.id);
            }
        }
        Ok(state)
    }

    fn write(&self, state: &mut State, record: ChangeRecord) -> Result<()> {
        if let Some(directory) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(directory)?;
        }
        let chain = chain_of(&state.last_chain, &serde_json::to_string(&record)?);
        let line = Line { seq: Some(state.next_seq), chain: Some(chain.clone()), record };
        let mut bytes = serde_json::to_vec(&line)?;
        bytes.push(b'\n');
        let mut file = OpenOptions::new().create(true).append(true).open(&self.path)?;
        // A torn previous write leaves no newline: start on a fresh line so this record stays parseable.
        if state.integrity.torn_tail {
            file.write_all(b"\n")?;
            state.file_len += 1;
            state.integrity.torn_tail = false;
        }
        file.write_all(&bytes)?;
        file.sync_all()?;
        state.file_len += bytes.len() as u64;
        state.last_chain = chain;
        state.next_seq += 1;
        state.integrity.verified_lines += 1;
        if state.records.insert(line.record.id, line.record.clone()).is_none() {
            state.order.push(line.record.id);
        }
        Ok(())
    }
}

impl Journal for JsonLinesJournal {
    fn append(&self, record: ChangeRecord) -> Result<()> {
        self.with_state(|state| {
            if state.records.contains_key(&record.id) {
                return Err(Error::new(
                    ErrorCode::Conflict,
                    format!("Change {} already exists in the journal.", record.id),
                ));
            }
            self.write(state, record)
        })
    }

    fn update_status(&self, id: Uuid, status: ChangeStatus, detail: Option<String>) -> Result<()> {
        self.with_state(|state| {
            let mut record = state
                .records
                .get(&id)
                .cloned()
                .ok_or_else(|| Error::not_found(format!("Change {id} not found in the journal.")))?;
            if status == ChangeStatus::RevertFailed {
                record.revert_attempts += 1;
            }
            record.status = status;
            record.status_detail = detail;
            record.status_changed_at = Some(OffsetDateTime::now_utc());
            self.write(state, record)
        })
    }

    fn update(&self, record: ChangeRecord) -> Result<()> {
        self.with_state(|state| {
            if !state.records.contains_key(&record.id) {
                return Err(Error::not_found(format!("Change {} not found in the journal.", record.id)));
            }
            self.write(state, record)
        })
    }

    fn all(&self) -> Result<Vec<ChangeRecord>> {
        self.with_state(|state| {
            let mut records: Vec<ChangeRecord> =
                state.order.iter().filter_map(|id| state.records.get(id).cloned()).collect();
            records.sort_by_key(|r| r.created_at);
            Ok(records)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChangeCategory;
    use time::macros::datetime;

    fn record(kind: &str, minute: u8) -> ChangeRecord {
        ChangeRecord::new(
            ChangeCategory::Network,
            kind,
            "bitBybit-nat",
            "create NAT",
            datetime!(2026-11-11 10:00 UTC) + time::Duration::minutes(minute.into()),
        )
        .with_states(None, Some(r#"{"prefix":"192.168.173.0/24"}"#.into()))
    }

    fn journal() -> (tempfile::TempDir, JsonLinesJournal) {
        let dir = tempfile::tempdir().unwrap();
        let journal = JsonLinesJournal::new(dir.path().join("journal").join("changes.jsonl"));
        (dir, journal)
    }

    #[test]
    fn appends_updates_and_survives_reopen() {
        let (_dir, j) = journal();
        let (a, b) = (record("net.nat", 1), record("net.address", 0));
        j.append(a.clone()).unwrap();
        j.append(b.clone()).unwrap();
        j.update_status(a.id, ChangeStatus::Reverted, Some("done".into())).unwrap();

        let reopened = JsonLinesJournal::new(j.path());
        let all = reopened.all().unwrap();
        assert_eq!(all.iter().map(|r| r.id).collect::<Vec<_>>(), [b.id, a.id], "oldest first");
        assert_eq!(all[1].status, ChangeStatus::Reverted);
        assert_eq!(reopened.active().unwrap().len(), 1);
        let integrity = reopened.integrity().unwrap();
        assert_eq!((integrity.verified_lines, integrity.is_intact()), (3, true));
    }

    #[test]
    fn duplicate_ids_and_unknown_updates_are_rejected() {
        let (_dir, j) = journal();
        let a = record("net.nat", 0);
        j.append(a.clone()).unwrap();
        assert_eq!(j.append(a).unwrap_err().code, ErrorCode::Conflict);
        assert_eq!(
            j.update_status(Uuid::new_v4(), ChangeStatus::Reverted, None).unwrap_err().code,
            ErrorCode::NotFound
        );
        assert_eq!(j.update(record("x", 0)).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn a_torn_last_line_is_ignored_and_the_next_write_stays_readable() {
        let (_dir, j) = journal();
        let a = record("net.nat", 0);
        j.append(a.clone()).unwrap();
        let mut file = OpenOptions::new().append(true).open(j.path()).unwrap();
        file.write_all(br#"{"seq":1,"chain":"ab","id":"0000"#).unwrap();
        drop(file);

        let reopened = JsonLinesJournal::new(j.path());
        assert_eq!(reopened.all().unwrap().len(), 1);
        assert!(reopened.integrity().unwrap().torn_tail);
        let b = record("net.address", 1);
        reopened.append(b.clone()).unwrap();
        let again = JsonLinesJournal::new(j.path());
        assert_eq!(again.all().unwrap().len(), 2);
        assert_eq!(again.integrity().unwrap().corrupt_lines, 1, "the torn line is now in the middle");
    }

    #[test]
    fn an_edited_line_breaks_the_chain() {
        let (_dir, j) = journal();
        j.append(record("net.nat", 0)).unwrap();
        j.append(record("net.address", 1)).unwrap();
        let text = fs::read_to_string(j.path()).unwrap().replacen("create NAT", "create XXX", 1);
        fs::write(j.path(), text).unwrap();
        let integrity = JsonLinesJournal::new(j.path()).integrity().unwrap();
        assert_eq!(integrity.chain_broken_at, Some(1));
        assert!(!integrity.is_intact());
    }

    #[test]
    fn reads_lines_written_by_the_dotnet_journal() {
        let (_dir, j) = journal();
        fs::create_dir_all(j.path().parent().unwrap()).unwrap();
        fs::write(
            j.path(),
            concat!(
                r#"{"id":"6f9d3c1e-2b7a-4c55-9e0f-1a2b3c4d5e6f","createdAt":"2026-10-20T21:57:01.1234567+00:00","category":"Driver","targetKind":"device.driverSelected","targetId":"HDAUDIO\\FUNC_01","description":"Install driver hdaudio.inf","previousState":"{\"infName\":\"oem14.inf\"}","appliedState":null,"correlationId":null,"revertOnRecovery":false,"revertOnUninstall":false,"status":"Applied","statusChangedAt":null,"statusDetail":null}"#,
                "\n"
            ),
        )
        .unwrap();
        let all = j.all().unwrap();
        assert_eq!(all[0].category, ChangeCategory::Driver);
        assert!(all[0].is_active() && all[0].reversible);
        assert_eq!(j.integrity().unwrap().legacy_lines, 1);
        j.update_status(all[0].id, ChangeStatus::Reverted, None).unwrap();
        assert!(JsonLinesJournal::new(j.path()).active().unwrap().is_empty());
    }

    #[test]
    fn failed_reverts_are_counted_and_other_writers_are_seen() {
        let (_dir, j) = journal();
        let a = record("net.nat", 0);
        j.append(a.clone()).unwrap();
        j.update_status(a.id, ChangeStatus::RevertFailed, Some("busy".into())).unwrap();
        j.update_status(a.id, ChangeStatus::RevertFailed, Some("busy".into())).unwrap();
        assert_eq!(j.get(a.id).unwrap().unwrap().revert_attempts, 2);

        let other = JsonLinesJournal::new(j.path());
        other.append(record("net.address", 1)).unwrap();
        assert_eq!(j.all().unwrap().len(), 2, "reloads when another writer appended");
    }
}
