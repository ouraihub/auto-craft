//! Reference: craft-anything crates/ai/src/rebuild/locate.rs (300dba58).
//! Local target location and executable classification without execution.
use super::*;
/// The folder whose files show how a picked game was made (beside a launcher, above bin/).
pub fn game_folder(path: &Path) -> Option<PathBuf> {
    locate(path, true).1
}

/// The game's own folder when the person picked one of its inner folders (Unreal's Engine/, a
/// Binaries/Win64, Content/Paks…): up to two levels, never further, so a pick can't widen to a
/// whole game library.
pub(super) fn game_root(picked: &Path) -> PathBuf {
    const INNER: [&str; 8] = [
        "engine",
        "binaries",
        "win64",
        "win32",
        "content",
        "paks",
        "bin",
        "bin_win32",
    ];
    let mut dir = picked.to_path_buf();
    for _ in 0..2 {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match dir.parent() {
            Some(up) if INNER.contains(&name.as_str()) => dir = up.to_path_buf(),
            _ => break,
        }
    }
    dir
}

/// What to study for a local target: the program itself (a launcher script points at it), and
/// the folder whose maps, pictures and sounds show how it looks (for launchers, and for games
/// when `game`).
pub(crate) fn locate(path: &Path, game: bool) -> (PathBuf, Option<PathBuf>) {
    if path.is_dir() {
        if game {
            let root = game_root(path);
            return (
                main_program(&root).unwrap_or_else(|| path.to_path_buf()),
                Some(root),
            );
        }
        return (path.to_path_buf(), None);
    }
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if ["bat", "cmd", "sh", "command", "ps1"].contains(&ext.as_str()) {
        let program = script_program(path)
            .or_else(|| main_program(&dir))
            .unwrap_or_else(|| path.to_path_buf());
        return (program, Some(dir));
    }
    // A game's program often sits in bin/, bin_win32/ or Binaries/Win64; its content is above.
    let mut root = dir;
    for _ in 0..3 {
        let name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match root.parent() {
            Some(up)
                if name.starts_with("bin")
                    || ["win64", "win32", "x64", "x86"].contains(&name.as_str()) =>
            {
                root = up.to_path_buf()
            }
            _ => break,
        }
    }
    (path.to_path_buf(), game.then_some(root))
}

/// The program a launcher script starts: the first word in it that names an existing file.
pub(super) fn script_program(script: &Path) -> Option<PathBuf> {
    let dir = script.parent()?;
    let text = std::fs::read(script).ok()?;
    let text = String::from_utf8_lossy(&text[..text.len().min(64 * 1024)]);
    text.split(|c: char| c.is_whitespace() || c == '"' || c == '\'')
        .map(|w| {
            w.trim_start_matches("%~dp0")
                .trim_start_matches("$(dirname \"$0\")/")
                .trim_start_matches("./")
        })
        .filter(|w| !w.is_empty() && !w.starts_with('-'))
        .map(|w| dir.join(w.replace('\\', std::path::MAIN_SEPARATOR_STR)))
        .find(|p| p.is_file() && p.as_path() != script && is_program(p))
}

/// The biggest program in a folder (3 levels deep), skipping installers, servers and helpers.
pub(super) fn main_program(dir: &Path) -> Option<PathBuf> {
    const SKIP: [&str; 8] = [
        "unins", "setup", "install", "crash", "redist", "server", "update", "helper",
    ];
    let mut best: Option<(u64, PathBuf)> = None;
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth < 3 {
                    stack.push((p, depth + 1));
                }
                continue;
            }
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            if is_program(&p)
                && !SKIP.iter().any(|s| name.contains(s))
                && best.as_ref().is_none_or(|(b, _)| size > *b)
            {
                best = Some((size, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

pub fn is_console(program: &Path) -> bool {
    use std::io::Read as _;
    let read = |limit: u64| -> Option<Vec<u8>> {
        let mut bytes = Vec::new();
        std::fs::File::open(program)
            .and_then(|f| f.take(limit).read_to_end(&mut bytes))
            .ok()?;
        Some(bytes)
    };
    let Some(head) = read(64 * 1024) else {
        return false;
    };
    if head.starts_with(b"MZ") {
        return ca_formats::pe::headers(&head).is_ok_and(|h| h.subsystem == 3);
    }
    let Some(b) = read(MAX_READ).and_then(|bytes| ca_formats::parse_binary(&bytes).ok()) else {
        return false;
    };
    loads_no_window(&b)
}

/// An ELF or Mach-O executable that loads none of [`WINDOW_LIBRARIES`].
fn loads_no_window(b: &ca_formats::Binary) -> bool {
    use ca_formats::Format;
    matches!(b.format, Format::Elf | Format::MachO)
        && b.entry.is_some()
        && !b.libraries.iter().any(|l| {
            let l = l.to_lowercase();
            WINDOW_LIBRARIES.iter().any(|w| l.contains(w))
        })
}

const MAX_READ: u64 = 256 << 20;
const WINDOW_LIBRARIES: &[&str] = &[
    "libx11",
    "libxcb",
    "libwayland",
    "libgtk",
    "libgdk",
    "libqt",
    "libsdl",
    "libglfw",
    "libgl.",
    "libegl",
    "libvulkan",
    "appkit.framework",
    "cocoa.framework",
    "uikit.framework",
    "swiftui.framework",
    "metal.framework",
    "quartzcore.framework",
];

pub(super) fn captured(ws: &Workspace) -> Vec<Value> {
    ["no-arguments", "help", "version"]
        .iter()
        .filter_map(|name| {
            std::fs::read(ws.capture_dir().join("runs").join(format!("{name}.json"))).ok()
        })
        .filter_map(|b| serde_json::from_slice(&b).ok())
        .collect()
}
fn is_program(p: &Path) -> bool {
    ca_engine::outputs::is_program(p)
}
