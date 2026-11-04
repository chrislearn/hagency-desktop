//! Promote complete Mermaid code blocks in the parsed chat view. The original
//! Matrix HTML/body stays untouched for copying, forwarding, exporting and edits.
use makepad_widgets::{
    live_id,
    makepad_html::{HtmlDoc, HtmlNode},
    LiveId,
};

const MAX_SOURCE: usize = 16 * 1024;
const MAX_DIAGRAMS: usize = 8;

/// Preserve node indices and the parser's resolved element boundaries. Changing
/// only the two tag pairs and their text keeps surrounding links, lists, quotes
/// and Matrix-specific spans on the existing chat rendering path.
pub(super) fn promote(doc: &mut HtmlDoc) -> usize {
    let existing = doc
        .nodes
        .iter()
        .filter(|node| matches!(node, HtmlNode::OpenTag { lc, .. } if *lc == live_id!(chatdiagram)))
        .count();
    let mut promoted = 0;
    let mut index = 0;
    while index < doc.nodes.len() && existing + promoted < MAX_DIAGRAMS {
        if !matches!(doc.nodes[index], HtmlNode::OpenTag { lc, .. } if lc == live_id!(pre)) {
            index += 1;
            continue;
        }
        let pre = index;
        let walker = doc.new_walker_with_index(pre);
        let Some(pre_close) = walker.close_index() else {
            index += 1;
            continue;
        };
        index = pre_close + 1;

        // Only one direct <code> child, with optional surrounding whitespace.
        let Some(code) = (pre + 1..pre_close).find(|&i| {
            !matches!(
                &doc.nodes[i],
                HtmlNode::Attribute { .. } | HtmlNode::Text { all_ws: true, .. }
            )
        }) else {
            continue;
        };
        let walker = doc.new_walker_with_index(code);
        if walker.open_tag_lc() != Some(live_id!(code))
            || !walker.find_attr_lc(live_id!(class)).is_some_and(|class| {
                class
                    .split_ascii_whitespace()
                    .any(|c| c.eq_ignore_ascii_case("language-mermaid"))
            })
        {
            continue;
        }
        let Some(code_close) = walker.close_index().filter(|&close| close < pre_close) else {
            continue;
        };
        if doc.nodes[code_close + 1..pre_close]
            .iter()
            .any(|node| !matches!(node, HtmlNode::Text { all_ws: true, .. }))
        {
            continue;
        }
        let mut source = String::new();
        let mut text_nodes = Vec::new();
        let mut plain = true;
        for i in code + 1..code_close {
            match &doc.nodes[i] {
                HtmlNode::Attribute { .. } => (),
                HtmlNode::Text { start, end, .. } => {
                    source.push_str(&doc.decoded[*start..*end]);
                    text_nodes.push(i);
                    if source.len() > MAX_SOURCE {
                        plain = false;
                        break;
                    }
                }
                _ => {
                    plain = false;
                    break;
                }
            }
        }
        if !plain
            || text_nodes.is_empty()
            || article_makepad::diagram::render("mermaid", &source).is_err()
        {
            // Invalid, incomplete, oversized or non-text blocks keep the full
            // normal code presentation. The shared renderer bounds and caches
            // both successful images and diagnostics.
            continue;
        }
        let payload = format!(
            "{}:{}",
            article_makepad::content::hex("mermaid"),
            article_makepad::content::hex(&source)
        );
        let start = doc.decoded.len();
        doc.decoded.push_str(&payload);
        let end = doc.decoded.len();
        doc.nodes[pre] = HtmlNode::OpenTag {
            lc: live_id!(p),
            nc: live_id!(p),
        };
        doc.nodes[pre_close] = HtmlNode::CloseTag {
            lc: live_id!(p),
            nc: live_id!(p),
        };
        doc.nodes[code] = HtmlNode::OpenTag {
            lc: live_id!(chatdiagram),
            nc: live_id!(chatdiagram),
        };
        doc.nodes[code_close] = HtmlNode::CloseTag {
            lc: live_id!(chatdiagram),
            nc: live_id!(chatdiagram),
        };
        for (position, i) in text_nodes.into_iter().enumerate() {
            doc.nodes[i] = HtmlNode::Text {
                start,
                end: if position == 0 { end } else { start },
                all_ws: position != 0,
            };
        }
        promoted += 1;
    }
    promoted
}

#[cfg(test)]
mod tests {
    use super::*;
    use makepad_widgets::{InternLiveId, makepad_html::parse_html};
    use matrix_sdk::ruma::events::room::message::FormattedBody;

    const GRAPHENE_OS: &str = include_str!("../../tests/fixtures/grapheneos.mmd");

    fn parse(html: &str) -> HtmlDoc {
        parse_html(html, &mut None, InternLiveId::No)
    }
    fn html(source: &str) -> String {
        let mut formatted = FormattedBody::markdown(&format!("```mermaid\n{source}\n```")).unwrap();
        formatted.sanitize_html(
            matrix_sdk::ruma::html::HtmlSanitizerMode::Strict,
            matrix_sdk::ruma::html::RemoveReplyFallback::No,
        );
        formatted.body
    }
    fn sources(doc: &HtmlDoc) -> Vec<String> {
        (0..doc.nodes.len())
            .filter_map(|i| {
                let node = doc.new_walker_with_index(i);
                if node.open_tag_lc() != Some(live_id!(chatdiagram)) {
                    return None;
                }
                let text = node.find_text().unwrap();
                article_makepad::content::unhex(text.split_once(':').unwrap().1)
            })
            .collect()
    }

    #[test]
    fn exact_grapheneos_source_renders_and_retains_decoded_copy_source() {
        let original = html(GRAPHENE_OS.trim_end());
        let mut doc = parse(&original);
        let node_count = doc.nodes.len();
        assert_eq!(promote(&mut doc), 1);
        assert_eq!(sources(&doc), [GRAPHENE_OS]);
        assert_eq!(doc.nodes.len(), node_count);
        assert_eq!(
            promote(&mut doc),
            0,
            "unchanged draw must not re-promote or change widget IDs"
        );
        assert!(original.contains("language-mermaid"));
        let image = article_makepad::diagram::render("mermaid", GRAPHENE_OS).unwrap();
        assert!(image.width > 100 && image.height > 100 && image.png.len() > 1000);
    }

    #[test]
    fn surrounding_matrix_markup_and_other_code_stay_intact() {
        let before = "<p><a href=\"https://matrix.to/#/@user:test\">User</a><span data-mx-spoiler=\"reason\">Hidden</span></p>";
        let after = "<pre><code class=\"language-rust\">fn main() {}</code></pre><p>After</p>";
        // HTML entities are decoded exactly once: use a quoted label containing
        // escaped markup to exercise source preservation without interpreting it.
        let original = format!(
            "{before}{}{after}",
            html("flowchart LR\nA[\"&lt;b&gt; & 中文\"]-->B")
        );
        let mut doc = parse(&original);
        let prefix_len = doc.decoded.len();
        let decoded = doc.decoded.clone();
        assert_eq!(promote(&mut doc), 1);
        assert_eq!(&doc.decoded[..prefix_len], decoded);
        assert!(sources(&doc)[0].contains("&lt;b&gt; & 中文"));
        assert!(doc.decoded.contains("https://matrix.to/#/@user:test"));
        assert!(doc.decoded.contains("fn main() {}"));
    }

    #[test]
    fn malformed_oversized_inline_and_unmarked_code_remain_source() {
        for original in [
            html("flowchart TD\nsubgraph missing_end\nA-->B"),
            html(&format!("flowchart TD\nA[{}]", "x".repeat(MAX_SOURCE))),
            "<code class=\"language-mermaid\">flowchart TD\nA-->B</code>".into(),
            "<pre><code>flowchart TD\nA-->B</code></pre>".into(),
            "<pre><code class=\"language-mermaid\">flowchart TD\n<b>A</b>-->B</code></pre>".into(),
            "<pre><code class=\"language-mermaid\">flowchart TD\nA-->B".into(),
            "<pre><code class=\"language-mermaid\">flowchart TD\nA-->B</code>extra</pre>".into(),
        ] {
            let mut doc = parse(&original);
            assert_eq!(promote(&mut doc), 0, "{original}");
            assert!(doc == parse(&original));
        }
    }

    #[test]
    fn mixed_blocks_class_tokens_and_per_message_limit() {
        let valid = "<PRE>\n<CODE class='highlight LANGUAGE-MERMAID'>flowchart LR\nA--&gt;B\n</CODE>\n</PRE>";
        let mut doc = parse(&format!(
            "{}{}",
            html("invalid diagram"),
            valid.repeat(MAX_DIAGRAMS + 1)
        ));
        assert_eq!(promote(&mut doc), MAX_DIAGRAMS);
        assert_eq!(
            promote(&mut doc),
            0,
            "redraws must preserve the per-message limit"
        );
        assert_eq!(sources(&doc).len(), MAX_DIAGRAMS);
    }
}
