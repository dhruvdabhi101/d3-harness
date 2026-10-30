use crate::conversation::Message;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    workspace: PathBuf,
    messages: Vec<Message>,
}

pub struct SessionSummary {
    pub id: String,
    pub title: String,
}

struct SavedSession {
    summary: SessionSummary,
    modified: SystemTime,
    messages: Vec<Message>,
}

pub struct Session {
    directory: PathBuf,
    workspace: PathBuf,
    path: Option<PathBuf>,
}

impl Session {
    pub fn new(workspace: &Path) -> Result<Self> {
        let workspace = workspace
            .canonicalize()
            .context("resolving conversation workspace")?;
        let root = std::process::Command::new("git")
            .arg("-C")
            .arg(&workspace)
            .args(["rev-parse", "--show-toplevel"])
            .output();
        let workspace = match root {
            Ok(output) if output.status.success() => {
                PathBuf::from(String::from_utf8(output.stdout)?.trim_end()).canonicalize()?
            }
            _ => workspace,
        };
        let hash = workspace
            .to_string_lossy()
            .as_bytes()
            .iter()
            .fold(14695981039346656037u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(1099511628211)
            });
        let directory = dirs::data_local_dir()
            .context("cannot find local data directory for sessions")?
            .join("d3/sessions")
            .join(format!("{hash:016x}"));
        Ok(Self {
            directory,
            workspace: workspace.to_path_buf(),
            path: None,
        })
    }

    fn saved_sessions(&self) -> Result<Vec<SavedSession>> {
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).context("reading saved conversations"),
        };
        let mut candidates = Vec::new();
        for entry in entries {
            let entry = entry?;
            if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let bytes = fs::read(entry.path()).context("reading saved conversation")?;
            let snapshot: Snapshot =
                serde_json::from_slice(&bytes).context("parsing saved conversation")?;
            if snapshot.version != 1 {
                bail!(
                    "unsupported conversation format version {}",
                    snapshot.version
                );
            }
            if snapshot.workspace == self.workspace && !snapshot.messages.is_empty() {
                let id = entry
                    .path()
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                let title = snapshot
                    .messages
                    .iter()
                    .find(|message| message.role == crate::conversation::Role::User)
                    .map(|message| {
                        message
                            .text()
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                candidates.push(SavedSession {
                    summary: SessionSummary {
                        id,
                        title: title.chars().take(100).collect(),
                    },
                    modified: entry.metadata()?.modified()?,
                    messages: snapshot.messages,
                });
            }
        }
        candidates.sort_by(|a, b| {
            b.modified
                .cmp(&a.modified)
                .then_with(|| b.summary.id.cmp(&a.summary.id))
        });
        Ok(candidates)
    }

    pub fn list(&self) -> Result<Vec<SessionSummary>> {
        Ok(self
            .saved_sessions()?
            .into_iter()
            .map(|session| session.summary)
            .collect())
    }

    pub fn resume(&mut self, id: Option<&str>) -> Result<Vec<Message>> {
        let sessions = self.saved_sessions()?;
        let selected = match id {
            Some(id) => sessions
                .into_iter()
                .find(|session| session.summary.id == id)
                .with_context(|| {
                    format!("no saved conversation with ID '{id}' in this workspace")
                })?,
            None => sessions
                .into_iter()
                .next()
                .context("no saved conversation for this workspace")?,
        };
        self.path = None;
        Ok(selected.messages)
    }

    pub fn resume_latest(&mut self) -> Result<Vec<Message>> {
        self.resume(None)
    }

    pub fn new_conversation(&mut self) {
        self.path = None;
    }

    pub fn save(&mut self, messages: &[Message]) -> Result<()> {
        if messages.is_empty() {
            return Ok(());
        }
        fs::create_dir_all(&self.directory).context("creating session directory")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                self.directory.parent().unwrap(),
                fs::Permissions::from_mode(0o700),
            )?;
            fs::set_permissions(&self.directory, fs::Permissions::from_mode(0o700))?;
        }
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = self.path.get_or_insert_with(|| {
            self.directory
                .join(format!("{stamp}-{}.json", std::process::id()))
        });
        let temp = self
            .directory
            .join(format!(".{stamp}-{}.tmp", std::process::id()));
        let result = (|| -> Result<()> {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp)?;
            serde_json::to_writer(
                &mut file,
                &Snapshot {
                    version: 1,
                    workspace: self.workspace.clone(),
                    messages: messages.to_vec(),
                },
            )?;
            file.flush()?;
            file.sync_all()?;
            fs::rename(&temp, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result.context("saving conversation")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_session_is_scoped_and_new_keeps_old_session() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("project");
        let mut session = Session {
            directory: dir.path().join("sessions"),
            workspace: workspace.clone(),
            path: None,
        };
        session.save(&[Message::user("first")]).unwrap();
        session.new_conversation();
        session.save(&[Message::user("second")]).unwrap();
        let mut resumed = Session {
            directory: session.directory.clone(),
            workspace,
            path: None,
        };
        let messages = resumed.resume_latest().unwrap();
        assert_eq!(messages[0].text(), "second");
        let mut another = Session {
            directory: session.directory.clone(),
            workspace: resumed.workspace.clone(),
            path: None,
        };
        assert_eq!(another.resume_latest().unwrap()[0].text(), "second");
        resumed.save(&[Message::user("branch one")]).unwrap();
        another.save(&[Message::user("branch two")]).unwrap();
        assert_eq!(fs::read_dir(&session.directory).unwrap().count(), 4);
        assert_eq!(resumed.resume_latest().unwrap()[0].text(), "branch two");
        let mut other = Session {
            directory: session.directory,
            workspace: dir.path().join("other"),
            path: None,
        };
        assert!(other.resume_latest().is_err());
    }
}
