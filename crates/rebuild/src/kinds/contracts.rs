//! Reference: craft-anything crates/ai/src/rebuild/kinds/screenshot.rs (300dba58).
//! Builder-facing screenshot, mechanics and self-test contracts.
use super::*;
/// What every app and game brief asks for, so the check can see the result.
pub const SCREENSHOT_CONTRACT: &str = "Give the program a `--screenshot <file.png>` option: start normally, draw about 120 frames, save the window's picture as a PNG to that file and exit. With the option, open the window without taking focus, so a check never gets in the way of what the person is doing, and on Windows also behind other windows (a covered window still draws there; on macOS and Linux it may stop drawing, so don't lower it there). Keep it drawing while it isn't focused, or the picture never comes. In egui: `ViewportBuilder::with_active(false)`, on Windows `with_window_level(WindowLevel::AlwaysOnBottom)`, and on macOS `with_activate_ignoring_other_apps(false)` through `NativeOptions::event_loop_builder`; in Unity, `Application.runInBackground = true`; other stacks, their own way. Started without the option, the program opens as usual. autocraft runs it that way after each build to check the result, and a build without the option, or with a blank picture, doesn't pass. Try it yourself: run it with a file inside app/ (for example app/shots/now.png) and check that it saves the picture.";

/// Include the shared catalogue's supported entrypoints so the builder knows how checks start it.
/// A builder that can look at pictures is also told to look at its own.
pub fn screenshot_contract(sees_pictures: SeesPictures) -> String {
    let entries = ca_engine::toolchains::catalogue()
        .iter()
        .filter(|t| !t.launch.is_empty())
        .map(|t| {
            format!(
                "{}: {}",
                t.name,
                t.launch
                    .iter()
                    .map(|r| r.file.as_str())
                    .collect::<Vec<_>>()
                    .join(" or ")
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "{SCREENSHOT_CONTRACT}{} If not packaging an executable, keep a supported entrypoint directly in app/ ({entries}); wildcard entrypoints must match exactly one file.",
        match sees_pictures {
            SeesPictures::Yes => " Then open it with your file tools and look at it.",
            SeesPictures::No => "",
            SeesPictures::Unknown =>
                " Open it with your file tools; if you cannot view images, use the recorded measurements instead.",
        }
    )
}
/// What every game brief asks for, so the check can read the numbers.
pub const MECHANICS_CONTRACT: &str = "Keep the same values in app/mechanics.json, which autocraft reads after each build: {\"version\": 1, \"values\": [{\"name\": \"walk speed\", \"group\": \"movement\", \"value\": 4.5, \"unit\": \"m/s\", \"source\": \"function 0x140012a0\"}, {\"name\": \"crouch\", \"group\": \"controls\", \"value\": \"hold\", \"estimated\": true}]}. A value is a number, text or true/false; give each one its source or \"estimated\": true, and use the sheet's numbers in the code as they're written there.";
/// What every game brief asks for, so the check can see the core loop played.
pub const SELF_TEST_CONTRACT: &str = "Give the game a `--self-test <file.json>` option too: start a game the way a player would, drive the player with scripted input for a few seconds, measure from the game's own state whether each step of the core loop works, save the results to that file and exit: {\"version\": 1, \"steps\": [{\"step\": \"move\", \"passed\": true, \"detail\": \"walked 3.1 m forward\"}, {\"step\": \"act\", \"passed\": true, \"detail\": \"fired 5 shots, 2 hit\"}, {\"step\": \"hud\", \"passed\": true, \"detail\": \"health, ammo and the objective are drawn\"}]}. \"move\": the player (or what they steer) moves; \"act\": their main action works (firing, mining, jumping, eating…); \"hud\": the on-screen information the original shows (score, health, ammo…) is drawn. autocraft runs it after each build, and the game isn't ready until every step passes.";
