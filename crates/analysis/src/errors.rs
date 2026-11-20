//! Stable upstream error classification at the application service boundary.
use autocraft_core::{Error, ErrorKind};
use ca_evidence::{ErrorCode, ToolError};

pub(crate) fn convert(tool: &str, error: ToolError) -> Error {
    let kind = match error.code {
        ErrorCode::BadInput | ErrorCode::BadSettings | ErrorCode::NotSupported => {
            ErrorKind::Invalid
        }
        ErrorCode::StaleRevision => ErrorKind::Conflict,
        ErrorCode::NotFound => ErrorKind::NotFound,
        ErrorCode::FeatureMissing
            if error
                .details
                .as_ref()
                .and_then(|v| v.get("reason"))
                .and_then(serde_json::Value::as_str)
                == Some("clean_room") =>
        {
            ErrorKind::DeepRequired
        }
        ErrorCode::FeatureMissing => ErrorKind::Invalid,
        ErrorCode::EngineMissing => ErrorKind::EngineMissing,
        ErrorCode::ConsentRequired => ErrorKind::ConsentRequired,
        ErrorCode::TimedOut => ErrorKind::Timeout,
        ErrorCode::Cancelled => ErrorKind::Cancelled,
        ErrorCode::NoPermission => ErrorKind::Forbidden,
        ErrorCode::FileChanged | ErrorCode::FileHashMismatch | ErrorCode::EvidenceHashMismatch => {
            ErrorKind::Corrupt
        }
        ErrorCode::GarbledOutput
        | ErrorCode::ProgramFailed
        | ErrorCode::RecordingFailed
        | ErrorCode::FileError
        | ErrorCode::CleanupFailed => ErrorKind::External,
        ErrorCode::OverLimit | ErrorCode::Truncated | ErrorCode::Internal => ErrorKind::ToolFailed,
    };
    // Upstream ToolError carries no typed I/O source and its prose may contain private paths/arguments.
    // Keep the stable classification here; MCP errors retain their upstream protocol body.
    let context = if tool.starts_with("project") {
        "项目操作失败"
    } else if tool.starts_with("settings") {
        "分析设置保存失败"
    } else {
        "分析工具执行失败"
    };
    Error::new(kind, context)
}

#[cfg(test)]
mod tests;
