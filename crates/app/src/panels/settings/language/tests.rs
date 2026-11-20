//! Changing the language updates an existing view and persists the same preference.

use super::LanguagePage;
use gpui_kit::{AppContext as _, TestAppContext, test::TestWindowExt as _};

#[gpui_kit::test]
fn switching_language_refreshes_existing_window_and_persists(cx: &mut TestAppContext) {
    let (fixture, page) = crate::testing::open(cx, |state, _, cx| LanguagePage::new(state, cx));
    drop(std::panic::take_hook());
    cx.run_until_parked();
    let click = |cx: &mut TestAppContext, index: usize| {
        cx.update_window(fixture.window, |_, window, cx| {
            window.render_frame(cx);
            window.click("locale-choice", cx);
            window.within("popup-menu").click(index, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    click(cx, 1);
    cx.update(|cx| {
        let page = page.read(cx);
        assert_eq!(page.preference.as_deref(), Some("en"));
        assert_eq!(
            page.state
                .read(cx)
                .services
                .app_prefs
                .app_prefs()
                .unwrap()
                .locale,
            "en"
        );
    });
    click(cx, 0);
    cx.update(|cx| {
        let page = page.read(cx);
        assert_eq!(page.preference.as_deref(), Some("zh-CN"));
        assert_eq!(
            page.state
                .read(cx)
                .services
                .app_prefs
                .app_prefs()
                .unwrap()
                .locale,
            "zh-CN"
        );
    });
}
