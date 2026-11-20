//! Host-owned LumaKit labels. Shared widgets keep Chinese defaults for other products.
use std::{borrow::Cow, collections::HashMap, sync::LazyLock};

const LABELS: &[(&str, &str)] = &[
    (
        "搜索模型…（可附加等级：opus high）",
        "Search models… (optional effort: opus high)",
    ),
    (
        "搜索模型，可附加等级",
        "Search models, optionally with an effort level",
    ),
    ("选择模型", "Select model"),
    ("模型与推理强度", "Model and reasoning effort"),
    ("模型分组", "Model group"),
    ("已展开", "expanded"),
    ("已折叠", "collapsed"),
    ("API 密钥", "API key"),
    ("没有匹配的模型", "No matching models"),
    (
        "没有可用模型，请先配置服务商",
        "No models available. Configure a provider first.",
    ),
    ("设为默认", "Set default"),
    ("重置为 off", "Reset to off"),
    ("更新中…", "Refreshing…"),
    ("刷新", "Refresh"),
    ("默认", "Default"),
    ("使用服务商默认强度", "Use the provider's default effort"),
    ("推理强度", "Reasoning effort"),
    ("力度", "Effort"),
    ("复制诊断 ID", "Copy diagnostic ID"),
    ("复制目录刷新诊断 ID", "Copy catalog refresh diagnostic ID"),
    ("就绪", "Ready"),
    ("生成中", "Generating"),
    ("压缩中", "Compacting"),
    ("状态未知", "Unknown state"),
    ("上下文", "Context"),
    ("来源", "Source"),
    ("估算", "Estimate"),
    (
        "宿主报告；缺失值保持未知",
        "Host-reported; missing values remain unknown",
    ),
    ("累计请求 tokens", "Cumulative request tokens"),
    ("显示", "Displayed"),
    ("状态", "State"),
    ("自动压缩", "Automatic compaction"),
    ("已启用", "Enabled"),
    ("未启用或未报告", "Disabled or not reported"),
    (
        "? 表示未知，~ 表示上下文估算；有请求时保存的用户单价才显示金额，否则显示 token。金额不是服务商账单。",
        "? means unknown; ~ means estimated context. Amounts use user prices saved with requests; otherwise tokens are shown. Amounts are not provider bills.",
    ),
    ("上下文用量", "Context usage"),
    ("查看用量详情", "View usage details"),
    ("复制密钥", "Copy key"),
    ("修改密钥", "Edit key"),
    ("删除密钥…", "Delete key…"),
    ("隐藏密钥", "Hide key"),
    ("显示密钥", "Show key"),
    ("使用此密钥", "Use this key"),
    ("取消修改", "Cancel edit"),
    ("保存修改后的密钥", "Save edited key"),
    ("保存新密钥", "Save new key"),
    ("添加密钥", "Add key"),
    (
        "空草稿不会清空原密钥",
        "An empty draft does not erase the saved key",
    ),
    ("密钥管理", "Manage keys"),
    ("确认删除", "Confirm deletion"),
    ("取消", "Cancel"),
    ("密钥使用方式", "Key selection policy"),
    ("API 地址", "API URL"),
    ("模型", "Models"),
];

pub(super) fn install(locale: &str) {
    let translator = match locale {
        "zh-CN" => chinese,
        "en-XA" => pseudo,
        _ => english,
    };
    lumakit_model_picker::localization::set_translator(translator);
    lumakit_usage_status::localization::set_translator(translator);
    lumakit_design::localization::set_translator(translator);
    lumakit_gpui::localization::set_translator(translator);
}

fn chinese(source: &'static str) -> Cow<'static, str> {
    Cow::Borrowed(source)
}

fn english(source: &'static str) -> Cow<'static, str> {
    LABELS.iter().find(|(key, _)| *key == source).map_or_else(
        || format!("⟦{source}⟧").into(),
        |(_, label)| Cow::Borrowed(*label),
    )
}

fn pseudo(source: &'static str) -> Cow<'static, str> {
    static LABELS_PSEUDO: LazyLock<HashMap<&'static str, String>> = LazyLock::new(|| {
        LABELS
            .iter()
            .map(|(key, label)| (*key, pseudo_label(key, label)))
            .collect()
    });
    LABELS_PSEUDO.get(source).map_or_else(
        || format!("⟦{source}⟧").into(),
        |label| Cow::Borrowed(label.as_str()),
    )
}

fn pseudo_label(source: &str, label: &str) -> String {
    match source {
        "重置为 off" => super::pseudo("Reset to %{effort}").replace("%{effort}", "off"),
        "搜索模型…（可附加等级：opus high）" => {
            super::pseudo("Search models… (optional effort: %{example})")
                .replace("%{example}", "opus high")
        }
        _ => super::pseudo(label),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pseudo_preserves_wire_examples_and_missing_labels_remain_visible() {
        let unknown = english("unknown-shared-label");
        assert!(unknown.contains("unknown-shared-label"));
        assert!(unknown.starts_with('⟦') && unknown.ends_with('⟧'));
        assert_ne!(pseudo("重置为 off"), english("重置为 off"));
        assert!(pseudo("重置为 off").contains("off"));
        assert!(pseudo("搜索模型…（可附加等级：opus high）").contains("opus high"));
    }
}
