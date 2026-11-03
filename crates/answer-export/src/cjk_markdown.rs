//! Display-only compatibility for emphasis next to Chinese punctuation.
//! Source text, escapes, code, raw HTML and link destinations stay untouched.
use std::borrow::Cow;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

pub struct CjkMarkdown<'a> {
    source: Cow<'a, str>,
    marker: Option<char>,
}

impl<'a> CjkMarkdown<'a> {
    pub fn new(body: &'a str) -> Self {
        if !body.contains("**") { return Self { source: Cow::Borrowed(body), marker: None }; }
        let mut literal = Vec::<std::ops::Range<usize>>::new();
        let mut code = false;
        for (event, range) in Parser::new_ext(body, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH).into_offset_iter() {
            match event {
                Event::Start(Tag::CodeBlock(_)) => code = true,
                Event::End(TagEnd::CodeBlock) => code = false,
                Event::Text(_) if !code => {
                    if let Some(last) = literal.last_mut().filter(|last| last.end == range.start) {
                        last.end = range.end;
                    } else { literal.push(range); }
                }
                _ => {}
            }
        }
        let punctuation = |c: char| "，。！？；：、（）「」『』【】《》〈〉〔〕［］｛｝“”‘’…—".contains(c);
        let mut positions = Vec::new();
        for range in literal {
            for (offset, _) in body[range.clone()].match_indices("**") {
                let p = range.start + offset;
                let before = &body[..p];
                let after = &body[p + 2..];
                if before.ends_with('*') || after.starts_with('*')
                    || before.bytes().rev().take_while(|b| *b == b'\\').count() % 2 != 0 { continue; }
                let prev = before.chars().next_back();
                let next = after.chars().next();
                if prev.is_some_and(punctuation) && next.is_some_and(char::is_alphanumeric) {
                    positions.push(p);
                }
                if prev.is_some_and(char::is_alphanumeric) && next.is_some_and(punctuation) {
                    positions.push(p + 2);
                }
            }
        }
        if positions.is_empty() { return Self { source: Cow::Borrowed(body), marker: None }; }
        // A non-punctuation sentinel relaxes only these unmatched delimiters.
        // Pick one absent from the source, then remove exactly that sentinel
        // after parsing; real zero-width characters and original PUA survive.
        let marker = (0xE000..=0xF8FF).filter_map(char::from_u32).find(|c| !body.contains(*c));
        let Some(marker) = marker else { return Self { source: Cow::Borrowed(body), marker: None }; };
        positions.sort_unstable(); positions.dedup();
        let mut source = String::with_capacity(body.len() + positions.len() * marker.len_utf8());
        let mut start = 0;
        for p in positions { source.push_str(&body[start..p]); source.push(marker); start = p; }
        source.push_str(&body[start..]);
        Self { source: Cow::Owned(source), marker: Some(marker) }
    }
    pub fn changed(&self) -> bool { self.marker.is_some() }
    pub fn source(&self) -> &str { &self.source }
    pub fn clean(&self, text: &str) -> String {
        match self.marker { Some(marker) => text.replace(marker, ""), None => text.to_owned() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn html(body: &str) -> String {
        let fixed = CjkMarkdown::new(body);
        let mut html = String::new();
        pulldown_cmark::html::push_html(&mut html, Parser::new_ext(fixed.source(), Options::ENABLE_TABLES));
        fixed.clean(&html)
    }
    #[test]
    fn chinese_punctuation_emphasis_without_spaces() {
        for (input, expected) in [
            ("**时间点很重要：**截至美国得州中部时间", "<strong>时间点很重要：</strong>截至美国得州中部时间"),
            ("**民主党：**得州州众议员塔拉里科；", "<strong>民主党：</strong>得州州众议员塔拉里科；"),
            ("这是**“预测”**而不是结果", "这是<strong>“预测”</strong>而不是结果"),
            ("**但得州的共和党基础仍然强。**2024 年", "<strong>但得州的共和党基础仍然强。</strong>2024 年"),
        ] { assert!(html(input).contains(expected), "{}", html(input)); }
    }
    #[test]
    fn code_escapes_links_html_and_original_characters_stay_literal() {
        for input in [
            "`**时间点：**截至`", "```\n**时间点：**截至\n```",
            "    **时间点：**截至", r"\*\*时间点：\*\*截至",
            "[link](https://example.org/**时间点：**截至)",
            "<pre>**时间点：**截至</pre>",
            "**正常加粗**文字", "2 ** 3", "不成对的 **中文", "***正常***",
        ] { assert!(!CjkMarkdown::new(input).changed(), "{input}"); }
        assert_eq!(html("\u{e000}\u{200b}**时间点：**截至"), "<p>\u{e000}\u{200b}<strong>时间点：</strong>截至</p>\n");
    }
}
