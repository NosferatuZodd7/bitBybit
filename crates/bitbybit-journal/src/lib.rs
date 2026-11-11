//! The change journal. Every system change is recorded *before* it is applied, with the previous state, so a
//! crash mid-change still leaves a recoverable record. Records never contain secrets.
//!
//! File format: JSON lines. Each line is a full snapshot of one record (the last snapshot per id wins) plus
//! `seq` and `chain`, where `chain = sha256(previous chain + record JSON)`. Lines written by the .NET versions
//! have no chain and are accepted as legacy. A torn last line (power loss) is ignored.

mod file;
mod record;

pub use file::{Integrity, JsonLinesJournal};
pub use record::{ChangeCategory, ChangeRecord, ChangeStatus};

use bitbybit_core::Result;
use uuid::Uuid;

pub trait Journal: Send + Sync {
    /// Records a new change. Fails with `Conflict` when the id already exists.
    fn append(&self, record: ChangeRecord) -> Result<()>;

    fn update_status(&self, id: Uuid, status: ChangeStatus, detail: Option<String>) -> Result<()>;

    /// Replaces a record (e.g. the applied state once the OS assigned an index).
    fn update(&self, record: ChangeRecord) -> Result<()>;

    /// All records, oldest first.
    fn all(&self) -> Result<Vec<ChangeRecord>>;

    fn get(&self, id: Uuid) -> Result<Option<ChangeRecord>> {
        Ok(self.all()?.into_iter().find(|r| r.id == id))
    }

    fn active(&self) -> Result<Vec<ChangeRecord>> {
        Ok(self.all()?.into_iter().filter(ChangeRecord::is_active).collect())
    }
}
