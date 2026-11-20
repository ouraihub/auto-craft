//! Error kinds are semantic and must never depend on upstream prose.
use super::*;
#[test]
fn maps_cleanroom_conflicts_cancel_and_untyped_io_without_leaking_content() {
    let private = "private tool argument C:/person/private";
    for (upstream, expected) in [
        (ErrorCode::StaleRevision, ErrorKind::Conflict),
        (ErrorCode::Cancelled, ErrorKind::Cancelled),
        (ErrorCode::FileError, ErrorKind::External),
        (ErrorCode::EngineMissing, ErrorKind::EngineMissing),
    ] {
        let error = convert(
            "native_function_pseudocode",
            ToolError::new(upstream, private),
        );
        assert_eq!(error.kind(), Some(expected));
        assert!(!error.detail().contains(private));
    }
    let clean = ToolError::new(ErrorCode::FeatureMissing, private)
        .with_details(serde_json::json!({"reason":"clean_room"}));
    assert_eq!(
        convert("native_function_pseudocode", clean).kind(),
        Some(ErrorKind::DeepRequired)
    );
    let ordinary = ToolError::new(ErrorCode::FeatureMissing, "clean_room");
    assert_eq!(
        convert("native_function_pseudocode", ordinary).kind(),
        Some(ErrorKind::Invalid)
    );
}
