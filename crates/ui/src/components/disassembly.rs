//! Small assembly lexer; byte ranges always coincide with UTF-8 boundaries.
use crate::theme::Roles;
use gpui_kit::HighlightStyle;
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token {
    Mnemonic,
    Register,
    Immediate,
    Address,
    Comment,
}
fn tokens(line: &str) -> Vec<(Range<usize>, Token)> {
    let mut result = Vec::new();
    let mut chars = line.char_indices().peekable();
    let mut mnemonic = false;
    while let Some((start, ch)) = chars.next() {
        if ch == ';' || (ch == '/' && chars.peek().is_some_and(|(_, ch)| *ch == '/')) {
            result.push((start..line.len(), Token::Comment));
            break;
        }
        if !(ch.is_ascii_alphanumeric() || matches!(ch, '%' | '$' | '_' | '.')) {
            continue;
        }
        let mut end = start + ch.len_utf8();
        while let Some(&(index, next)) = chars.peek() {
            if !(next.is_ascii_alphanumeric() || matches!(next, '_' | '.')) {
                break;
            }
            chars.next();
            end = index + next.len_utf8();
        }
        let word = &line[start..end];
        let plain = word.trim_start_matches(['%', '$']);
        let hex = plain
            .strip_prefix("0x")
            .or_else(|| plain.strip_prefix("0X"));
        let numeric = hex
            .is_some_and(|value| !value.is_empty() && value.bytes().all(|b| b.is_ascii_hexdigit()))
            || plain.bytes().all(|b| b.is_ascii_digit())
            || (plain.len() >= 6 && plain.bytes().all(|b| b.is_ascii_hexdigit()));
        let token = if numeric {
            if line[end..].starts_with(':')
                || (!mnemonic && start == line.len() - line.trim_start().len())
            {
                Token::Address
            } else {
                Token::Immediate
            }
        } else if register(plain) {
            Token::Register
        } else if !mnemonic {
            mnemonic = true;
            Token::Mnemonic
        } else {
            continue;
        };
        result.push((start..end, token));
    }
    result
}
fn register(word: &str) -> bool {
    const NAMES: &[&str] = &[
        "rax", "rbx", "rcx", "rdx", "rsi", "rdi", "rsp", "rbp", "rip", "eax", "ebx", "ecx", "edx",
        "esi", "edi", "esp", "ebp", "eip", "ax", "bx", "cx", "dx", "al", "bl", "cl", "dl", "ah",
        "bh", "ch", "dh", "sp", "bp", "si", "di", "pc", "lr", "fp", "ip", "xzr", "wzr", "cs", "ds",
        "es", "fs", "gs", "ss",
    ];
    if NAMES.iter().any(|name| word.eq_ignore_ascii_case(name)) {
        return true;
    }
    for prefix in ["xmm", "ymm", "zmm", "r", "x", "w", "v", "s", "d"] {
        if word
            .get(..prefix.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
        {
            let number = &word[prefix.len()..];
            let number = number.trim_end_matches(['b', 'B', 'w', 'W', 'd', 'D']);
            if !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}
pub(super) fn highlights(line: &str, roles: Roles) -> Vec<(Range<usize>, HighlightStyle)> {
    tokens(line)
        .into_iter()
        .map(|(range, token)| {
            let color = match token {
                Token::Mnemonic => roles.primary,
                Token::Register => roles.feedback_info,
                Token::Immediate => roles.feedback_warning,
                Token::Address => roles.text_link,
                Token::Comment => roles.text_subtle,
            };
            (
                range,
                HighlightStyle {
                    color: Some(color.into()),
                    ..Default::default()
                },
            )
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::{Token, tokens};
    #[test]
    fn utf8_comments_and_register_boundaries() {
        let line = "00401000: mov rax, [r8+0x20] ; 注释 rax";
        let found = tokens(line);
        assert_eq!(&line[found[0].0.clone()], "00401000");
        assert_eq!(found[0].1, Token::Address);
        assert!(
            found
                .iter()
                .any(|(range, kind)| &line[range.clone()] == "mov" && *kind == Token::Mnemonic)
        );
        assert!(
            found
                .iter()
                .any(|(range, kind)| &line[range.clone()] == "r8" && *kind == Token::Register)
        );
        assert_eq!(
            found.last().map(|(range, _)| &line[range.clone()]),
            Some("; 注释 rax")
        );
        assert!(
            !tokens("call rax_label")
                .iter()
                .any(|(_, kind)| *kind == Token::Register)
        );
    }
}
