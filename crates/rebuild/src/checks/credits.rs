//! Reference: craft-anything crates/ai/src/rebuild/kinds/game.rs (300dba58).
//! Deep game builds must attribute original files and mark private use.
use super::*;
const PRIVATE_USE_NOTE: &str = "This build contains the original game's own files, for the private use of the person who owns the game, on their own computer. It must not be shared or published.";
pub struct CreditsCheck;
impl Check for CreditsCheck {
    fn id(&self) -> &'static str {
        "credits"
    }
    fn run(&self, cx: &mut CheckCx<'_>) -> Result<CheckResult> {
        let app = cx.ws.app_dir();
        let read = |name: &str| {
            fs::read_to_string(app.join(name))
                .unwrap_or_default()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        let mut missing = Vec::new();
        if !read("README.md").contains(PRIVATE_USE_NOTE) {
            missing.push(format!(
                "start app/README.md with this line, as written: \"{PRIVATE_USE_NOTE}\""
            ));
        }
        if read("CREDITS.md").is_empty() {
            missing.push("list every original file you used in app/CREDITS.md, with its path in the game's folder".to_owned());
        }
        if missing.is_empty() {
            Ok(passed_result(
                self.id(),
                cx.round,
                "app/README.md marks private use and app/CREDITS.md lists original files.",
            ))
        } else {
            Ok(error_result(
                self.id(),
                cx.round,
                format!(
                    "This build uses the original game's own files, so {}.",
                    missing.join(", and ")
                ),
            ))
        }
    }
}
