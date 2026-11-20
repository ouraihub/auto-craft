//! Reference: craft-anything engine/feed.rs and workspace.rs (300dba58).
//! Core DTO projection and validated, atomic attachment storage.
use autocraft_core::*;
use ca_engine::{
    feed::{FeedItem, PlanStatus},
    inbox::{Request, RequestKind},
    workspace::{CheckResult, Claim, PageLikeness, Step, Workspace},
};
use std::path::{Path, PathBuf};

pub fn read_feed(root: &Path) -> Result<Vec<FeedEntryDto>> {
    let ws = Workspace::open(root).map_err(|_| Error::external("无法打开项目对话"))?;
    with_questions(&ws, &ca_engine::feed::load(&ws))
}
pub(super) fn with_questions(ws: &Workspace, items: &[FeedItem]) -> Result<Vec<FeedEntryDto>> {
    let mut asked = ca_engine::questions::load(ws);
    asked.sort_by_key(|q| (q.after, q.at_ms));
    let mut asked = asked.into_iter().peekable();
    let mut out = Vec::new();
    for position in 0..=items.len() {
        while asked.peek().is_some_and(|q| q.after <= position) {
            if let Some(q) = asked.next() {
                out.push(FeedEntryDto::Question {
                    question: q.question,
                    answer: q.answer,
                    from_session: q.from_session,
                    backend: q.backend,
                    at_ms: q.at_ms,
                });
            }
        }
        if let Some(item) = items.get(position) {
            out.extend(items_dto(std::slice::from_ref(item))?);
        }
    }
    for q in asked {
        out.push(FeedEntryDto::Question {
            question: q.question,
            answer: q.answer,
            from_session: q.from_session,
            backend: q.backend,
            at_ms: q.at_ms,
        });
    }
    Ok(out)
}
pub(super) fn items_dto(items: &[FeedItem]) -> Result<Vec<FeedEntryDto>> {
    items
        .iter()
        .filter_map(|item| {
            Some(Ok(match item {
                FeedItem::User(user) => FeedEntryDto::User {
                    text: user.text().into(),
                    pictures: user.pictures().to_vec(),
                },
                FeedItem::Agent { text, streaming } => FeedEntryDto::Agent {
                    text: text.clone(),
                    streaming: *streaming,
                },
                FeedItem::Work { tools, notes } => FeedEntryDto::Work {
                    tools: tools
                        .iter()
                        .map(|(name, ok)| FeedToolDto {
                            name: name.clone(),
                            ok: *ok,
                        })
                        .collect(),
                    notes: notes.clone(),
                },
                FeedItem::Ask { request, answer } => FeedEntryDto::Ask {
                    request: request_dto(request),
                    answer: answer.as_ref().map(|text| InboxAnswer::Choice {
                        choice: None,
                        text: Some(text.clone()),
                    }),
                },
                FeedItem::Check(result) => FeedEntryDto::Check {
                    result: check_dto(result),
                },
                FeedItem::Step { step, text } => FeedEntryDto::Step {
                    step: match step {
                        Step::Study => StepDto::Study,
                        Step::Build => StepDto::Build,
                        Step::Check => StepDto::Check,
                        Step::Done => StepDto::Done,
                    },
                    text: text.clone(),
                },
                FeedItem::Error {
                    message,
                    remediation,
                    ..
                } => FeedEntryDto::Error {
                    message: message.clone(),
                    remediation: remediation.clone(),
                },
                FeedItem::Paused => FeedEntryDto::Paused,
                FeedItem::Plan(steps) => FeedEntryDto::Plan {
                    steps: steps
                        .iter()
                        .map(|s| FeedPlanStepDto {
                            text: s.text.clone(),
                            doing: s.doing.clone(),
                            status: match s.status {
                                PlanStatus::Todo => FeedPlanStatus::Todo,
                                PlanStatus::Doing => FeedPlanStatus::Doing,
                                PlanStatus::Done => FeedPlanStatus::Done,
                                PlanStatus::Stopped => FeedPlanStatus::Stopped,
                            },
                        })
                        .collect(),
                },
                FeedItem::PlanDismissed => return None,
            }))
        })
        .collect()
}
pub(super) fn request_dto(r: &Request) -> InboxRequest {
    InboxRequest {
        id: InboxRequestId::from(r.id.clone()),
        kind: match r.kind {
            RequestKind::Question => InboxKind::Question,
            RequestKind::Approval => InboxKind::Approval,
        },
        title: r.title.clone(),
        detail: r.detail.clone(),
        choices: r.choices.clone(),
        allow_text: r.allow_text,
        remember: r.action.clone(),
        created_ms: r.created_ms,
    }
}
pub(crate) fn check_dto(c: &CheckResult) -> CheckResultDto {
    CheckResultDto {
        round: c.round,
        check: c.check.clone(),
        passed: c.passed,
        note: c.note.clone(),
        similarity: c.similarity,
        good_enough: c.good_enough,
        claims: c
            .claims
            .iter()
            .map(|c| ClaimDto {
                title: c.title.clone(),
                passed: c.passed,
                detail: c.detail.clone(),
            })
            .collect(),
        unknowns: c.unknowns.clone(),
        artifacts: c.artifacts.clone(),
        pages: c
            .pages
            .iter()
            .map(|p| PageLikenessDto {
                name: p.name.clone(),
                dir: p.dir.clone(),
                similarity: p.similarity,
            })
            .collect(),
        at_ms: c.at_ms,
    }
}
pub(super) fn source_check(c: &CheckResultDto) -> CheckResult {
    CheckResult {
        round: c.round,
        check: c.check.clone(),
        passed: c.passed,
        note: c.note.clone(),
        similarity: c.similarity,
        good_enough: c.good_enough,
        claims: c
            .claims
            .iter()
            .map(|c| Claim {
                title: c.title.clone(),
                passed: c.passed,
                detail: c.detail.clone(),
            })
            .collect(),
        unknowns: c.unknowns.clone(),
        artifacts: c.artifacts.clone(),
        pages: c
            .pages
            .iter()
            .map(|p| PageLikeness {
                name: p.name.clone(),
                dir: p.dir.clone(),
                similarity: p.similarity,
            })
            .collect(),
        at_ms: c.at_ms,
    }
}
/// Copy source-compatible picture attachments, checking content as well as extension.
pub fn attach_pictures(root: &Path, pictures: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let ws = Workspace::open(root).map_err(|_| Error::external("无法打开项目附件"))?;
    if pictures.iter().any(|p| {
        !ca_engine::workspace::attachable_picture(p) || !ca_engine::workspace::picture_content(p)
    }) {
        return Err(Error::invalid(
            "图片必须是有效 PNG、JPEG、WebP 或 GIF，且不超过 20 MB",
        ));
    }
    ws.add_attachments(&pictures)
        .map_err(|_| Error::external("无法保存图片附件"))
}
/// Validate decoded dimensions before saving clipboard bytes, never trusting the MIME alone.
pub fn attach_image(root: &Path, bytes: Vec<u8>, mime: &str) -> Result<PathBuf> {
    let (extension, format) = match mime {
        "image/png" => ("png", image::ImageFormat::Png),
        "image/jpeg" => ("jpg", image::ImageFormat::Jpeg),
        _ => return Err(Error::invalid("剪贴板图片必须是 PNG 或 JPEG")),
    };
    if bytes.is_empty() || bytes.len() > 20 * 1024 * 1024 {
        return Err(Error::invalid("图片大小必须在 1 字节到 20 MB 之间"));
    }
    let reader = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format);
    let (w, h) = reader
        .into_dimensions()
        .map_err(|_| Error::invalid("图片数据无效"))?;
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 15360 * 8640 {
        return Err(Error::invalid("图片尺寸超过 16K 限制"));
    }
    image::load_from_memory_with_format(&bytes, format)
        .map_err(|_| Error::invalid("图片数据不完整"))?;
    let ws = Workspace::open(root).map_err(|_| Error::external("无法打开项目附件"))?;
    let dir = ws.attachments_dir();
    std::fs::create_dir_all(&dir).context("无法创建图片附件目录")?;
    let path = dir.join(format!(
        "picture-{}-{}.{}",
        ca_engine::inbox::now_ms(),
        ca_engine::inbox::next_counter(),
        extension
    ));
    ca_engine::project::write_atomic(&path, &bytes)
        .map_err(|_| Error::external("无法保存图片附件"))?;
    Ok(path)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reopened_feed_interleaves_real_question_file() {
        let temp = tempfile::tempdir().unwrap();
        let ws = Workspace::create(
            temp.path(),
            "feed",
            "request",
            ca_engine::workspace::TargetRef::None,
        )
        .unwrap();
        let items = vec![
            FeedItem::User("make it blue".to_string().into()),
            FeedItem::Agent {
                text: "Done".into(),
                streaming: false,
            },
        ];
        ca_engine::feed::save(&ws.root, &items).unwrap();
        ca_engine::questions::add(
            &ws,
            ca_engine::questions::AskedQuestion::new(
                "Where?".into(),
                "In app/".into(),
                false,
                "pi".into(),
                None,
                1,
            ),
        )
        .unwrap();
        let loaded = read_feed(&ws.root).unwrap();
        assert!(
            matches!(&loaded[1],FeedEntryDto::Question {question,answer,..} if question=="Where?"&&answer=="In app/")
        );
        assert!(matches!(&loaded[2],FeedEntryDto::Agent {text,..} if text=="Done"));
    }
    #[test]
    fn validates_clipboard_and_persists_original_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let ws = Workspace::create(
            temp.path(),
            "image",
            "request",
            ca_engine::workspace::TargetRef::None,
        )
        .unwrap();
        assert!(attach_image(&ws.root, b"not an image".to_vec(), "image/png").is_err());
        assert!(!ws.attachments_dir().exists());
        let image = image::DynamicImage::new_rgba8(2, 2);
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let bytes = bytes.into_inner();
        let path = attach_image(&ws.root, bytes.clone(), "image/png").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(path.starts_with(ws.attachments_dir()));
        assert!(attach_image(&ws.root, vec![], "image/jpeg").is_err());
    }
}
