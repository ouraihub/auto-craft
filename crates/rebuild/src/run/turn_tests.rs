//! Authoritative snapshot completion must ignore historical assistant output.
use super::*;
#[test]
fn reply_ignores_old_messages_and_thinking() {
    let old = HashSet::from(["old".to_owned()]);
    let messages = vec![
        Message {
            id: "old".into(),
            role: Role::Assistant,
            content: vec![ContentBlock::Text {
                text: "historical".into(),
            }],
        },
        Message {
            id: "new".into(),
            role: Role::Assistant,
            content: vec![
                ContentBlock::Thinking {
                    text: "private".into(),
                },
                ContentBlock::Text {
                    text: "built it".into(),
                },
            ],
        },
    ];
    let (reply, decompiled) = reply(&messages, &old);
    assert_eq!(reply, "built it");
    assert!(!decompiled);
}
#[test]
fn prefixed_pseudocode_tool_marks_decompilation() {
    let messages = vec![Message {
        id: "t".into(),
        role: Role::Assistant,
        content: vec![ContentBlock::Tool {
            tool_call_id: "c".into(),
            name: "mcp__craft__native_function_pseudocode".into(),
            status: ToolStatus::Completed,
            input: None,
            output: None,
        }],
    }];
    assert!(reply(&messages, &HashSet::new()).1);
}
