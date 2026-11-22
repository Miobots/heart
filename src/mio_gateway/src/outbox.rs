//! The durable outbox on disk (H3.2). An event bound for the Brain is here before it is sent, and
//! leaves only when the Brain names it in a `sys.receipt` (ENVELOPE §8). The session decides what
//! goes in and comes out; this only keeps it across a restart or a power cut.
//!
//! One file per event, named by a counter, so the directory listed in name order is the order the
//! events happened in. Not named by `msg_id`: its time half follows the wall clock, and a Pi's
//! clock can jump backwards. Each file is written beside its final name, flushed, renamed into
//! place, and the directory flushed — a power cut leaves the whole event or none of it.

use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;

pub struct Outbox {
    dir: PathBuf,
    next: u64,
    files: HashMap<String, PathBuf>, // msg_id → the file it is in
}

/// What was on disk when the outbox was opened.
pub struct Opened {
    pub outbox: Outbox,
    /// Events still waiting for the Brain, oldest first.
    pub waiting: Vec<Value>,
    /// Files that were not an event, moved aside as `*.bad` — kept for a person to look at, never
    /// silently dropped.
    pub unreadable: Vec<PathBuf>,
}

impl Outbox {
    /// Opens the directory, creating it, and reads back what a previous run left waiting.
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Opened> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        let mut names = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            match path.extension().and_then(|e| e.to_str()) {
                // Never renamed into place, so never in the outbox: a write cut short.
                Some("tmp") => fs::remove_file(&path)?,
                Some("json") => names.push(path),
                _ => {}
            }
        }
        names.sort(); // zero-padded counters: name order is the order they were written
        let (mut waiting, mut unreadable, mut files, mut next) =
            (Vec::new(), Vec::new(), HashMap::new(), 0);
        for path in names {
            let number = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u64>().ok());
            let event = fs::read(&path)
                .ok()
                .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok());
            let msg_id = event
                .as_ref()
                .and_then(|e| e["msg_id"].as_str())
                .map(str::to_owned);
            match (number, event, msg_id) {
                (Some(n), Some(event), Some(id)) => {
                    next = next.max(n + 1);
                    files.insert(id, path);
                    waiting.push(event);
                }
                _ => {
                    let bad = path.with_extension("bad");
                    fs::rename(&path, &bad)?;
                    unreadable.push(bad);
                }
            }
        }
        let outbox = Outbox { dir, next, files };
        Ok(Opened {
            outbox,
            waiting,
            unreadable,
        })
    }

    /// Puts an event on disk, and returns once it would survive a power cut.
    pub fn put(&mut self, event: &Value) -> io::Result<()> {
        let Some(id) = event["msg_id"].as_str() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "an event without a msg_id",
            ));
        };
        // Taken before trying: a failed write must never leave the next one reusing its name.
        let path = self.dir.join(format!("{:020}.json", self.next));
        self.next += 1;
        let tmp = path.with_extension("tmp");
        let mut file = File::create(&tmp)?;
        file.write_all(event.to_string().as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, &path)?;
        File::open(&self.dir)?.sync_all()?;
        self.files.insert(id.to_owned(), path);
        Ok(())
    }

    /// The Brain has it, so the file goes. An id this outbox does not hold is not an error: a
    /// receipt can name an event twice.
    pub fn remove(&mut self, msg_id: &str) -> io::Result<()> {
        let Some(path) = self.files.remove(msg_id) else {
            return Ok(());
        };
        match fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A directory of its own for each test, empty to start with.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mio-outbox-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn event(id: &str) -> Value {
        json!({ "msg_id": id, "kind": "EVT", "topic": "nav.result", "payload": { "success": true } })
    }

    fn ids(events: &[Value]) -> Vec<&str> {
        events
            .iter()
            .map(|e| e["msg_id"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn events_survive_a_restart_in_order_and_leave_only_when_removed() {
        let dir = scratch("restart");
        let mut outbox = Outbox::open(&dir).unwrap().outbox;
        for id in ["A", "B", "C"] {
            outbox.put(&event(id)).unwrap();
        }
        outbox.remove("B").unwrap();
        drop(outbox);

        let opened = Outbox::open(&dir).unwrap();
        assert_eq!(ids(&opened.waiting), ["A", "C"]);
        let mut outbox = opened.outbox;
        // Numbered after what is already there, so it comes out after it.
        outbox.put(&event("D")).unwrap();
        outbox.remove("A").unwrap();
        outbox.remove("A").unwrap(); // named twice: not an error
        outbox.remove("never-stored").unwrap();
        assert_eq!(ids(&Outbox::open(&dir).unwrap().waiting), ["C", "D"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_write_cut_short_is_not_an_event_and_a_broken_file_is_kept_aside() {
        let dir = scratch("broken");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("00000000000000000004.tmp"), "{\"msg_id\":").unwrap();
        fs::write(dir.join("00000000000000000002.json"), "not json").unwrap();
        fs::write(
            dir.join("00000000000000000001.json"),
            event("ok").to_string(),
        )
        .unwrap();

        let opened = Outbox::open(&dir).unwrap();
        assert_eq!(ids(&opened.waiting), ["ok"]);
        assert_eq!(
            opened.unreadable,
            [dir.join("00000000000000000002.bad")],
            "moved aside, not deleted"
        );
        assert!(!dir.join("00000000000000000004.tmp").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
