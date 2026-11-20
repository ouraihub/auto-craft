//! Fresh screenshot contract, bounded image decoding and reference-picture comparison.
use super::*;
use ca_engine::tools::web_common::{
    Image, MAX_SCREENSHOT_BYTES, block_similarity, decode_picture, decode_png,
    perceptual_similarity, resize,
};
mod runtime;

pub struct ScreenshotCheck;
impl Check for ScreenshotCheck {
    fn id(&self) -> &'static str {
        "app-screenshot"
    }
    fn applies(&self, cx: &CheckCx<'_>) -> bool {
        !cx.ws.capture_dir().join("runs").is_dir()
    }
    fn run(&self, cx: &mut CheckCx<'_>) -> Result<CheckResult> {
        let dir = cx.ws.checks_dir().join(format!("round-{}", cx.round));
        let shot = dir.join("screenshot.png");
        let saved = match process::option(cx, "--screenshot", &shot, MAX_SCREENSHOT_BYTES as u64)? {
            Ok(saved) => saved,
            Err(note) => return Ok(error_result(self.id(), cx.round, note)),
        };
        let artifacts = relative_artifact(cx.ws, &shot).into_iter().collect();
        let image = match decode_png(&saved.bytes) {
            Ok(image) => image,
            Err(_) => {
                return Ok(CheckResult {
                    artifacts,
                    ..error_result(
                        self.id(),
                        cx.round,
                        "程序输出不是有效、完整且尺寸受限的 PNG。",
                    )
                });
            }
        };
        if is_blank(&image) {
            return Ok(CheckResult {
                artifacts,
                ..error_result(
                    self.id(),
                    cx.round,
                    "截图为空白或单色，请绘制正常内容后再保存。",
                )
            });
        }
        let mut pictures = cx.ws.references();
        let original = cx.ws.capture_dir().join("window.png");
        if original.is_file() {
            pictures.push(original);
        }
        let (similarity, unreadable) = likeness(cx, &image, &pictures)?;
        let bar = cx.build_settings().picture_good_enough;
        let mut note = format!("程序已启动并绘制截图（{}×{}）。", image.width, image.height);
        if let Some(score) = similarity {
            note.push_str(&format!(" 与最接近的参考图片相似度为 {score:.0}%。"));
        }
        if unreadable > 0 {
            note.push_str(&format!(" {unreadable} 张参考图片不可读，未纳入比较。"));
        }
        let (running, claims, more_artifacts) =
            runtime::compare(cx, &saved.program, &saved.args, &dir)?;
        note.push_str(&running);
        let mut artifacts: Vec<PathBuf> = artifacts;
        artifacts.extend(more_artifacts);
        let unknowns = if bar.is_some() && !pictures.is_empty() && similarity.is_none() {
            vec!["参考图片均不可读，无法确认截图相似度阈值。".into()]
        } else {
            Vec::new()
        };
        Ok(CheckResult {
            artifacts,
            similarity: bar.and(similarity),
            good_enough: bar,
            claims,
            unknowns,
            ..passed_result(self.id(), cx.round, note)
        })
    }
}
fn is_blank(image: &Image) -> bool {
    let small = resize(image, 64, 36);
    let (mut sum, mut squares, mut count) = (0.0, 0.0, 0.0);
    for pixel in small.rgba.as_chunks::<4>().0 {
        // Transparent colour bytes are not visible content.
        let alpha = f64::from(pixel[3]) / 255.0;
        let luma = (0.299 * f64::from(pixel[0])
            + 0.587 * f64::from(pixel[1])
            + 0.114 * f64::from(pixel[2]))
            * alpha;
        sum += luma;
        squares += luma * luma;
        count += 1.0;
    }
    count == 0.0 || (squares / count - (sum / count).powi(2)).max(0.0).sqrt() < 2.0
}
fn likeness(cx: &CheckCx<'_>, shot: &Image, pictures: &[PathBuf]) -> Result<(Option<f64>, usize)> {
    let a = resize(shot, 640, 360);
    let (mut best, mut unreadable) = (None::<f64>, 0);
    for picture in pictures.iter().take(64) {
        cx.stop_now()?;
        let image = process::read_bounded(picture, MAX_SCREENSHOT_BYTES as u64)
            .ok()
            .and_then(|b| decode_picture(&b).ok());
        let Some(image) = image else {
            unreadable += 1;
            continue;
        };
        let b = resize(&image, 640, 360);
        let score = (perceptual_similarity(&a, &b) + block_similarity(&a, &b, 16, 24)) * 50.0;
        best = Some(best.map_or(score, |current| current.max(score)));
    }
    Ok((best, unreadable))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ca_engine::tools::web_common::encode_png;
    #[test]
    fn malformed_truncated_blank_and_transparent_png_are_not_drawn_screens() {
        assert!(decode_png(b"\x89PNG\r\n\x1a\nthis is not a PNG").is_err());
        let blank = Image {
            width: 16,
            height: 16,
            rgba: vec![255; 16 * 16 * 4],
        };
        let bytes = encode_png(&blank).unwrap();
        assert!(decode_png(&bytes[..bytes.len() / 2]).is_err());
        assert!(is_blank(&decode_png(&bytes).unwrap()));
        let mut transparent = blank;
        for (i, p) in transparent
            .rgba
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .enumerate()
        {
            p[0] = i as u8;
            p[3] = 0;
        }
        assert!(is_blank(&transparent));
    }
}
