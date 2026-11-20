//! Persisting a new default wakes blocked consumers without rebinding existing chats.
use super::*;

impl CodingPage {
    pub(super) fn is_default(&self, backend: BackendKind) -> bool {
        self.prefs.default_agent == Some(backend)
    }

    pub(super) fn set_default(&mut self, backend: BackendKind, cx: &mut Context<Self>) {
        let prefs = self.state.read(cx).services.prefs.clone();
        run(
            cx,
            move || prefs.set_default_agent(Some(backend)),
            |this, result, cx| {
                match result {
                    Ok(prefs) => {
                        this.prefs = prefs;
                        this.state.update(cx, |_, cx| cx.emit(AppEvent::Routing));
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
}
