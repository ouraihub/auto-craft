//! Reference: craft-anything ai/src/rebuild/activity.rs (300dba58).
//! Bounded, scrubbed activity memory and the project's rotating activity file.
use autocraft_core::{ActivityLine, IoContext, LineKind, Result};
use ca_engine::{
    journal,
    workspace::{ACTIVITY_LOG, ACTIVITY_LOG_OLD},
};
use std::{
    collections::VecDeque,
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::Instant,
};
const KEPT: usize = 1000;
const LOG_LIMIT: u64 = 8 << 20;

pub(crate) struct Activity {
    root: PathBuf,
    started: Instant,
    lines: VecDeque<ActivityLine>,
}
impl Activity {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            started: Instant::now(),
            lines: VecDeque::new(),
        }
    }
    pub(crate) fn record(&mut self, kind: LineKind, text: &str) -> Result<()> {
        let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let text: String = journal::scrubbed_line(&one_line)
            .chars()
            .take(600)
            .collect();
        let path = self.root.join(ACTIVITY_LOG);
        match path.metadata() {
            Ok(meta) if meta.len() >= LOG_LIMIT => {
                let old = self.root.join(ACTIVITY_LOG_OLD);
                match std::fs::remove_file(&old) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => {
                        return Err(autocraft_core::Error::Io {
                            context: "无法轮转构建活动日志".into(),
                            source: e,
                        });
                    }
                }
                std::fs::rename(&path, old).context("无法轮转构建活动日志")?;
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(autocraft_core::Error::Io {
                    context: "无法读取构建活动日志".into(),
                    source: e,
                });
            }
        }
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let (_, _, time) = journal::now_utc();
        let mut file = options.open(path).context("无法打开构建活动日志")?;
        writeln!(file, "{time} {:<5} {text}", tag(kind)).context("无法保存构建活动日志")?;
        self.lines.push_back(ActivityLine {
            at: self.started.elapsed(),
            kind,
            text,
        });
        if self.lines.len() > KEPT {
            self.lines.pop_front();
        }
        Ok(())
    }
    pub(crate) fn lines(&self, offset: usize, limit: usize) -> Vec<ActivityLine> {
        let end = self.lines.len().saturating_sub(offset);
        let start = end.saturating_sub(limit.min(2000));
        self.lines.range(start..end).cloned().collect()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}
fn tag(kind: LineKind) -> &'static str {
    match kind {
        LineKind::Stage => "stage",
        LineKind::Started => "start",
        LineKind::Done => "done",
        LineKind::Failed => "FAIL",
        LineKind::Said => "said",
        LineKind::Note => "note",
        LineKind::Output => "out",
        LineKind::Problem => "ERROR",
    }
}
pub(crate) fn object(text: &str) -> String {
    journal::scrubbed_line(text).chars().take(200).collect()
}
pub(crate) fn read_log(root: &Path, offset: usize, limit: usize) -> Result<(Vec<String>, usize)> {
    let mut file = match std::fs::File::open(root.join(ACTIVITY_LOG)) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
        Err(e) => {
            return Err(autocraft_core::Error::Io {
                context: "无法读取构建活动日志".into(),
                source: e,
            });
        }
    };
    let start = file
        .metadata()
        .context("无法读取活动日志大小")?
        .len()
        .saturating_sub(4 << 20);
    file.seek(SeekFrom::Start(start))
        .context("无法读取构建活动日志")?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .context("无法读取构建活动日志")?;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<_> = text.lines().skip(usize::from(start > 0)).collect();
    let end = lines.len().saturating_sub(offset);
    let from = end.saturating_sub(limit.min(2000));
    Ok((
        lines[from..end]
            .iter()
            .map(|s| journal::scrubbed_line(s))
            .collect(),
        lines.len(),
    ))
}
pub(crate) fn read_lines(root: &Path, offset: usize, limit: usize) -> Result<Vec<ActivityLine>> {
    let (lines, _) = read_log(root, offset, limit)?;
    Ok(lines
        .into_iter()
        .filter_map(|line| {
            let mut parts = line.splitn(3, ' ');
            let time = parts.next()?;
            let rest = line.get(time.len()..)?.trim_start();
            let (tag, text) = rest.split_once(' ')?;
            let kind = match tag {
                "stage" => LineKind::Stage,
                "start" => LineKind::Started,
                "done" => LineKind::Done,
                "FAIL" => LineKind::Failed,
                "said" => LineKind::Said,
                "note" => LineKind::Note,
                "out" => LineKind::Output,
                "ERROR" => LineKind::Problem,
                _ => return None,
            };
            let mut hms = time.split(':');
            let hours = hms.next()?.parse::<u64>().ok()?;
            let minutes = hms.next()?.parse::<u64>().ok()?;
            let seconds = hms.next()?.parse::<u64>().ok()?;
            Some(ActivityLine {
                at: std::time::Duration::from_secs(hours * 3600 + minutes * 60 + seconds),
                kind,
                text: text.trim_start().into(),
            })
        })
        .collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_scrubs_and_persists() {
        let root = std::env::temp_dir().join(format!(
            "autocraft-activity-{}",
            ca_engine::inbox::next_counter()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut log = Activity::new(root.clone());
        for _ in 0..1002 {
            log.record(
                LineKind::Note,
                &format!("token=secret-value {}", "界".repeat(700)),
            )
            .unwrap();
        }
        let lines = log.lines(0, 2000);
        assert_eq!(lines.len(), 1000);
        assert!(lines.iter().all(|l| l.text.chars().count() <= 600));
        let (saved, total) = read_log(&root, 1, 2).unwrap();
        assert_eq!(saved.len(), 2);
        assert_eq!(total, 1002);
        assert!(!saved[0].contains("secret-value"));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rotation_preserves_previous_log_and_bounds_objects() {
        let root = tempfile::tempdir().unwrap();
        let file = std::fs::File::create(root.path().join(ACTIVITY_LOG)).unwrap();
        file.set_len(LOG_LIMIT).unwrap();
        drop(file);
        let mut activity = Activity::new(root.path().to_path_buf());
        activity.record(LineKind::Stage, "A new stage").unwrap();
        assert_eq!(
            std::fs::metadata(root.path().join(ACTIVITY_LOG_OLD))
                .unwrap()
                .len(),
            LOG_LIMIT
        );
        let (lines, _) = read_log(root.path(), 0, 200).unwrap();
        assert!(lines.last().unwrap().contains("A new stage"));
        assert_eq!(object(&"界".repeat(201)).chars().count(), 200);
    }
}
