// Linggo 长文本自动分段。受 ctx=512 约束：每段估算 Token 数 ≤ 预算，
// 保证「系统提示 + 原文 + 生成额度」放得进一个上下文。超长文本自动拆段，逐段翻译后拼接。
//
// 另一份责任：粗估 token 数与翻译生成额度（与 FlashTrans 同思路，1:1 任务按原文估再留余量）。

/// 单个分段的目标预算（token 数）。ctx=512 下留出系统提示与推理生成空间
pub const CHUNK_TOKEN_BUDGET: usize = 384;

/// 粗估一段文本的 token 数。CJK 基本一字一 token，拉丁文约四字符一 token。
/// 只用来估预算，不需要准——准的那个要等模型加载完才能分词，太晚了。
pub fn estimate_tokens(text: &str) -> usize {
    let units: usize = text
        .chars()
        .map(|c| if c as u32 >= 0x2E80 { 4 } else { 1 })
        .sum();
    units / 4 + 1
}

/// 翻译一次最多生成多少 token：按原文长度估，留一倍余量（换语言后长度会变）。
/// ctx=512 的天花板由 generate 侧按剩余位置二次钳制。
pub fn translate_budget(text: &str) -> usize {
    (estimate_tokens(text) * 2).clamp(64, 512)
}

fn is_sentence_boundary(c: char) -> bool {
    matches!(c, '。' | '！' | '？' | '．' | '.' | '!' | '?' | '；' | ';' | '\n')
}

/// 尝试在 `chars[..len]` 中从 `start` 处往前找最近的自然断句点，返回它的字符下标。
fn last_boundary(chars: &[char], up_to: usize, window: usize) -> Option<usize> {
    let from = up_to.saturating_sub(window);
    for k in (from..up_to).rev() {
        if is_sentence_boundary(chars[k]) {
            return Some(k + 1);
        }
    }
    None
}

/// 把长文本切成 ≤ 预算的若干段。优先在自然断句处切，找不到则硬切（保证每段放得进 ctx）。
pub fn segment_text(text: &str) -> Result<Vec<String>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(Vec::new());
    }
    let chars: Vec<char> = t.chars().collect();
    let mut segments = Vec::new();
    let mut start = 0usize;
    while start < chars.len() {
        if estimate_tokens(&chars[start..].iter().collect::<String>()) <= CHUNK_TOKEN_BUDGET {
            break; // 剩余全部放进最后一段
        }
        // 以段落为步进累加，找到刚好不超过预算的最大前缀
        let mut end = start;
        let mut acc = 0usize;
        while end < chars.len() && acc <= CHUNK_TOKEN_BUDGET {
            acc = estimate_tokens(&chars[start..=end].iter().collect::<String>());
            if acc <= CHUNK_TOKEN_BUDGET {
                end += 1;
            }
        }
        // end 是首个超预算的位置（若未超则已到结尾）
        let limit = end.min(chars.len());
        let cut = last_boundary(&chars, limit, 60).unwrap_or(limit);
        segments.push(chars[start..cut].iter().collect::<String>());
        start = cut;
    }
    if start < chars.len() {
        segments.push(chars[start..].iter().collect::<String>());
    }
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_no_segments() {
        assert_eq!(segment_text("   ").unwrap().len(), 0);
    }

    #[test]
    fn short_text_stays_one_segment() {
        assert_eq!(segment_text("你好，这是测试。").unwrap().len(), 1);
    }

    #[test]
    fn long_text_splits_and_never_exceeds_budget() {
        let long = "今天天气很好。".repeat(300);
        for seg in segment_text(&long).unwrap() {
            assert!(estimate_tokens(&seg) <= CHUNK_TOKEN_BUDGET);
        }
    }

    #[test]
    fn translation_budget_is_bounded() {
        assert!(translate_budget("hi") >= 64);
        assert!(translate_budget(&"字".repeat(2000)) <= 512);
    }
}