//! The same visual treatment is used by every host's native editor and reader.
use makepad_widgets::*;
use article_core::document::{Block, BlockKind, Document};
use crate::rich_input::ArticleRichInputRef;

pub fn show_body_placeholder(document: &Document, index: usize) -> bool {
    index == 0 && document.blocks.iter().all(|block| block.text.is_empty()
        && !matches!(block.kind, BlockKind::Image | BlockKind::Divider))
}

pub fn color(hex: u32) -> Vec4f {
    vec4(((hex >> 16) & 255) as f32 / 255.0,
        ((hex >> 8) & 255) as f32 / 255.0, (hex & 255) as f32 / 255.0, 1.0)
}

pub fn style_input(cx: &mut Cx, mut input: ArticleRichInputRef, document: &Document, block: &Block) {
    let size = match block.kind {
        BlockKind::Heading2 => 20.0,
        BlockKind::Heading3 => 17.0,
        _ => if document.large_type { 16.0 } else { 14.0 },
    };
    let (_, ink, accent) = document.theme.colors();
    let ink = color(if block.kind == BlockKind::Quote { accent } else { ink });
    let spacing = if document.compact { 1.0 } else { 1.25 };
    script_apply_eval!(cx, input, {draw_text +: {color: #(ink) color_focus: #(ink) color_hover: #(ink) text_style +: {line_spacing: #(spacing)}}});
    input.set_block(cx, block, size);
}

pub fn style_html(cx: &mut Cx, mut html: HtmlRef, document: &Document) {
    let (paper, ink, accent) = document.theme.colors();
    let (paper, ink, accent) = (color(paper), color(ink), color(accent));
    // Published paper keeps its own palette even inside a dark application.
    // Inheriting the host's dark quote fill made black article text unreadable.
    let quote = paper * 0.94 + ink * 0.06;
    let size = if document.large_type { 16.0 } else { 14.0 };
    let spacing = if document.compact { 1.0 } else { 1.25 };
    script_apply_eval!(cx, html, {
        draw_block +: {table_header_bg_color: #x0000000a table_border_color: #xb8c2bd code_color: #xf3f5f4 line_color: #xb8c2bd quote_bg_color: #(quote) quote_fg_color: #(accent)}
        font_size: #(size) font_color: #(ink)
        draw_text +: {color: #(ink)}
        text_style_normal +: {line_spacing: #(spacing)}
        text_style_bold +: {line_spacing: #(spacing)}
        text_style_italic +: {line_spacing: #(spacing)}
        text_style_bold_italic +: {line_spacing: #(spacing)}
    });
}
