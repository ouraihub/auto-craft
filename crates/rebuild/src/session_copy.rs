//! Reference: craft-anything ai/src/rebuild/mod.rs session preservation (300dba58).
//! Portable native transcripts and a backend-independent, bounded feed handoff.
use autocraft_core::{Error, IoContext, NativeSessionKey, Result};
use ca_engine::feed::FeedItem;
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
const MAX_BYTES: u64 = 64 << 20;
const HANDOFF_CHARS: usize = 16_000;

fn copy_path(root: &Path, backend: &str) -> Result<Option<PathBuf>> {
    match backend {
        "pi" | "omp" => Ok(Some(
            root.join("builder-session")
                .join(format!("{backend}.jsonl")),
        )),
        "dsh" => Ok(None),
        _ => Err(Error::invalid("未知构建方引擎")),
    }
}
fn bounded_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    let file = File::open(path).context("无法打开构建方会话副本")?;
    if file.metadata().context("无法读取构建方会话大小")?.len() > MAX_BYTES {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("无法读取构建方会话副本")?;
    Ok((bytes.len() as u64 <= MAX_BYTES).then_some(bytes))
}
/// Returns false for DSH or a transcript exceeding the portable-copy limit.
pub fn save(root: &Path, backend: &str, key: &NativeSessionKey) -> Result<bool> {
    let Some(path) = copy_path(root, backend)? else {
        return Ok(false);
    };
    let Some(bytes) = bounded_bytes(Path::new(key.as_str()))? else {
        return Ok(false);
    };
    std::fs::create_dir_all(root.join("builder-session")).context("无法创建会话副本目录")?;
    ca_engine::project::write_atomic(&path, &bytes)
        .map_err(|_| Error::external("无法保存构建方会话副本"))?;
    Ok(true)
}
/// Restore into the actual engine directory, never resume the project-owned backup in place.
pub fn restore(root: &Path, backend: &str, session_dir: &Path) -> Result<Option<NativeSessionKey>> {
    let Some(path) = copy_path(root, backend)? else {
        return Ok(None);
    };
    if !path.try_exists().context("无法检查会话副本")? {
        return Ok(None);
    }
    let Some(bytes) = bounded_bytes(&path)? else {
        return Ok(None);
    };
    std::fs::create_dir_all(session_dir).context("无法创建引擎会话目录")?;
    let name = format!(
        "autocraft-{}-{}.jsonl",
        ca_engine::inbox::now_ms(),
        ca_engine::inbox::next_counter()
    );
    let dest = session_dir.join(name);
    ca_engine::project::write_atomic(&dest, &bytes)
        .map_err(|_| Error::external("无法恢复构建方会话副本"))?;
    Ok(Some(NativeSessionKey::from(
        dest.to_string_lossy().into_owned(),
    )))
}
/// Latest user and builder words in chronological order; tools and checks are omitted.
pub fn handoff(items: &[FeedItem]) -> String {
    let mut selected = Vec::new();
    let mut remaining = HANDOFF_CHARS;
    for item in items.iter().rev() {
        let (who, words) = match item {
            FeedItem::User(user) => ("Person", user.text()),
            FeedItem::Agent { text, .. } => ("Builder", text.as_str()),
            FeedItem::Plan(_)
            | FeedItem::Step { .. }
            | FeedItem::Work { .. }
            | FeedItem::Ask { .. }
            | FeedItem::Check(_)
            | FeedItem::Error { .. }
            | FeedItem::Paused
            | FeedItem::PlanDismissed => continue,
        };
        if remaining <= who.len() + 4 {
            break;
        }
        let text: String = words
            .chars()
            .take(1500.min(remaining - who.len() - 4))
            .collect();
        let line = format!("{who}: {text}\n\n");
        remaining -= line.chars().count();
        selected.push(line);
    }
    selected.reverse();
    selected.concat()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_latest_words_bounded_and_in_order() {
        let items: Vec<_> = (0..30)
            .map(|i| FeedItem::Agent {
                text: format!("{i}:{}", "界".repeat(2000)),
                streaming: false,
            })
            .collect();
        let text = handoff(&items);
        assert!(text.chars().count() <= HANDOFF_CHARS);
        assert!(text.contains("29:"));
        assert!(!text.contains("Builder: 0:"));
        for line in text.lines().filter(|l| !l.is_empty()) {
            assert!(line.chars().count() <= 1509);
        }
    }
    #[test]
    fn restores_to_engine_not_project() {
        let root = std::env::temp_dir().join(format!(
            "autocraft-session-{}",
            ca_engine::inbox::next_counter()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let original = root.join("original.jsonl");
        std::fs::write(&original, b"{\"session\":1}\n").unwrap();
        assert!(
            save(
                &root,
                "pi",
                &NativeSessionKey::from(original.to_string_lossy().into_owned())
            )
            .unwrap()
        );
        std::fs::remove_file(&original).unwrap();
        let engine = root.join("engine/sessions");
        let key = restore(&root, "pi", &engine).unwrap().unwrap();
        assert!(Path::new(key.as_str()).starts_with(&engine));
        assert_eq!(std::fs::read(key.as_str()).unwrap(), b"{\"session\":1}\n");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn oversized_transcript_never_replaces_good_backup() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("source.jsonl");
        std::fs::write(&original, b"good session").unwrap();
        let key = NativeSessionKey::from(original.to_string_lossy().into_owned());
        assert!(save(root.path(), "omp", &key).unwrap());
        File::options()
            .write(true)
            .open(&original)
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();
        assert!(!save(root.path(), "omp", &key).unwrap());
        assert_eq!(
            std::fs::read(root.path().join("builder-session/omp.jsonl")).unwrap(),
            b"good session"
        );
        assert!(!save(root.path(), "dsh", &NativeSessionKey::from("native-id")).unwrap());
    }
}
