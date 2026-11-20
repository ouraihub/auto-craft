//! Description images remain in memory until creation, then the project service stores them.
use super::HomeView;
use autocraft_core::{Error, ProjectId, ProjectService, Result};
use gpui_kit::{ClipboardEntry, ClipboardItem, Context, Image};

impl HomeView {
    pub(super) fn paste_references(
        &mut self,
        clipboard: &ClipboardItem,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.busy {
            return false;
        }
        let mut handled = false;
        for entry in &clipboard.entries {
            match entry {
                ClipboardEntry::Image(image) => {
                    self.references.push(image.clone());
                    handled = true;
                }
                ClipboardEntry::String(_) | ClipboardEntry::ExternalPaths(_) => {}
            }
        }
        if handled {
            cx.notify();
        }
        handled
    }
}

pub(super) fn store(
    service: &dyn ProjectService,
    project: &ProjectId,
    images: &[Image],
) -> Result<()> {
    if images.is_empty() {
        return Ok(());
    }
    let directory = std::env::temp_dir().join(format!(
        "autocraft-reference-{}",
        autocraft_core::ChatId::generate()
    ));
    std::fs::create_dir(&directory)
        .map_err(|_| Error::invalid(rust_i18n::t!("app.home.reference_failed")))?;
    let result = (|| {
        let mut files = Vec::with_capacity(images.len());
        for (index, image) in images.iter().enumerate() {
            let path = directory.join(format!("reference-{index}.{}", image.format.extension()));
            std::fs::write(&path, &image.bytes)
                .map_err(|_| Error::invalid(rust_i18n::t!("app.home.reference_failed")))?;
            files.push(path);
        }
        service.add_references(project, &files)?;
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(&directory)
        .map_err(|_| Error::invalid(rust_i18n::t!("app.home.reference_cleanup_failed")));
    match (result, cleanup) {
        (Err(error), Err(_)) => {
            tracing::warn!(
                code = "REFERENCE_CLEANUP_FAILED",
                "home.references.cleanup_failed"
            );
            Err(error)
        }
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}
