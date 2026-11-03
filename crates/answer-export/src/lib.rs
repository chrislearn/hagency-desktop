//! Local-only Markdown answer export. Never fetches images, links, or fonts.
pub mod cjk_markdown;
use std::{collections::HashMap, ops::Range, sync::Arc};
use krilla::{
    Document,
    color::rgb,
    geom::{PathBuilder, Point, Rect},
    action::LinkAction,
    annotation::{Annotation, LinkAnnotation, Target},
    page::PageSettings,
    paint::Fill,
    text::{Font, GlyphId, KrillaGlyph},
};
use parley::{
    FontContext, LayoutContext,
    style::{FontWeight, LineHeight, StyleProperty, OverflowWrap},
};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

const MARGIN: f32 = 44.0;
const WIDTH: f32 = 595.0;
const HEIGHT: f32 = 842.0;
const TEXT_WIDTH: f32 = WIDTH - 2.0 * MARGIN;

/// The complete UTF-8 source, including Markdown and trailing newlines.
pub fn markdown(body: &str) -> Vec<u8> {
    body.as_bytes().to_vec()
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Ink {
    link: Option<String>,
}
#[derive(Default)]
struct Block {
    text: String,
    bold: Vec<Range<usize>>,
    code: Vec<Range<usize>>,
    links: Vec<(Range<usize>, String)>,
    size: f32,
    indent: f32,
    shaded: bool,
}
enum Part {
    Text(Block),
    Row(Vec<Block>, bool),
}

fn blocks(body: &str) -> Vec<Part> {
    let markdown = cjk_markdown::CjkMarkdown::new(body);
    let mut out = Vec::new();
    let mut b = Block::default();
    let mut row = Vec::new();
    let mut in_table = false;
    let mut header = false;
    let mut list: Vec<Option<u64>> = Vec::new();
    let mut bold = Vec::new();
    let mut links = Vec::new();
    let flush = |b: &mut Block, out: &mut Vec<Part>| {
        if !b.text.is_empty() {
            out.push(Part::Text(std::mem::take(b)));
        }
    };
    for event in Parser::new_ext(
        markdown.source(),
        Options::ENABLE_TABLES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_FOOTNOTES,
    ) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut b, &mut out);
                b.size = match level as u8 {
                    1 => 20.0,
                    2 => 16.0,
                    _ => 13.0,
                };
            }
            Event::End(TagEnd::Heading(_)) => {
                b.bold.push(0..b.text.len());
                flush(&mut b, &mut out);
            }
            Event::Start(Tag::CodeBlock(_)) => {
                flush(&mut b, &mut out);
                b.shaded = true;
                b.size = 9.5;
            }
            Event::End(TagEnd::CodeBlock) => {
                b.code.push(0..b.text.len());
                flush(&mut b, &mut out);
            }
            Event::Start(Tag::List(start)) => {
                flush(&mut b, &mut out);
                list.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                flush(&mut b, &mut out);
                list.pop();
            }
            Event::Start(Tag::Item) => {
                flush(&mut b, &mut out);
                b.indent = list.len().saturating_sub(1) as f32 * 14.0;
                if let Some(Some(number)) = list.last_mut() {
                    b.text = format!("{number}. ");
                    *number += 1;
                } else {
                    b.text.push_str("• ");
                }
            }
            Event::End(TagEnd::Item) => flush(&mut b, &mut out),
            Event::End(TagEnd::Paragraph) if !in_table => flush(&mut b, &mut out),
            Event::Start(Tag::Table(_)) => {
                flush(&mut b, &mut out);
                in_table = true;
            }
            Event::End(TagEnd::Table) => in_table = false,
            Event::Start(Tag::TableHead) => header = true,
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                out.push(Part::Row(std::mem::take(&mut row), header));
                header = false;
            }
            Event::End(TagEnd::TableCell) => {
                if header {
                    b.bold.push(0..b.text.len());
                }
                row.push(std::mem::take(&mut b));
            }
            Event::Start(Tag::Strong) => bold.push(b.text.len()),
            Event::End(TagEnd::Strong) => {
                if let Some(start) = bold.pop() {
                    b.bold.push(start..b.text.len());
                }
            }
            Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => {
                links.push((b.text.len(), dest_url.to_string()))
            }
            Event::End(TagEnd::Link | TagEnd::Image) => {
                if let Some((start, url)) = links.pop() {
                    // Preserve source URLs in print as well as clickable PDF annotations.
                    if b.text.get(start..) != Some(url.as_str()) {
                        b.text.push_str(&format!(" ({url})"));
                    }
                    if url.starts_with("https://")
                        || url.starts_with("http://")
                        || url.starts_with("mailto:")
                    {
                        b.links.push((start..b.text.len(), url));
                    }
                }
            }
            Event::Code(text) => {
                let start = b.text.len();
                b.text.push_str(&text);
                b.code.push(start..b.text.len());
            }
            Event::Text(text)
            | Event::Html(text)
            | Event::InlineHtml(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text) => b.text.push_str(&markdown.clean(&text)),
            Event::SoftBreak => b.text.push(' '),
            Event::HardBreak => b.text.push('\n'),
            Event::TaskListMarker(done) => b.text.push_str(if done { "[x] " } else { "[ ] " }),
            Event::FootnoteReference(label) => b.text.push_str(&format!("[{label}]")),
            Event::Start(Tag::FootnoteDefinition(label)) => {
                flush(&mut b, &mut out);
                b.text.push_str(&format!("[{label}] "));
            }
            Event::Rule => {
                flush(&mut b, &mut out);
                b.text.push_str("────────────");
                flush(&mut b, &mut out);
            }
            _ => {}
        }
    }
    flush(&mut b, &mut out);
    out
}

struct Run {
    x: f32,
    glyphs: Vec<KrillaGlyph>,
    font: Font,
    size: f32,
    text: Arc<String>,
    ink: Ink,
    width: f32,
}
struct Line {
    runs: Vec<Run>,
    height: f32,
    baseline: f32,
}
struct Typesetter {
    fonts: FontContext,
    layout: LayoutContext<Ink>,
    cache: HashMap<(u64, u32), Font>,
}
impl Typesetter {
    fn lines(&mut self, b: &Block, width: f32) -> Result<Vec<Line>, String> {
        let text = Arc::new(b.text.clone());
        let mut builder = self
            .layout
            .ranged_builder(&mut self.fonts, &text, 1.0, false);
        builder.push_default(StyleProperty::FontSize(if b.size > 0.0 {
            b.size
        } else {
            10.5
        }));
        builder.push_default(StyleProperty::FontFamily(
            parley::style::FontFamily::Source("sans-serif".into()),
        ));
        builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(1.5)));
        builder.push_default(StyleProperty::OverflowWrap(OverflowWrap::Anywhere));
        for range in &b.bold {
            builder.push(StyleProperty::FontWeight(FontWeight::BOLD), range.clone());
        }
        for range in &b.code {
            builder.push(
                StyleProperty::FontFamily(parley::style::FontFamily::Source("monospace".into())),
                range.clone(),
            );
        }
        for (range, url) in &b.links {
            builder.push(
                StyleProperty::Brush(Ink {
                    link: Some(url.clone()),
                }),
                range.clone(),
            );
        }
        let mut layout = builder.build(text.as_str());
        layout.break_all_lines(Some(width));
        let mut lines = Vec::new();
        for line in layout.lines() {
            let metrics = line.metrics();
            let mut result = Line {
                runs: Vec::new(),
                height: metrics.line_height,
                baseline: metrics.ascent
                    + (metrics.line_height - metrics.ascent - metrics.descent) / 2.0,
            };
            let mut x = 0.0;
            for run in line.runs() {
                let font = run.font().clone();
                let (data, id) = font.data.into_raw_parts();
                let key = (id, font.index);
                if let std::collections::hash_map::Entry::Vacant(entry) = self.cache.entry(key) {
                    entry.insert(
                        Font::new(data.into(), font.index)
                            .ok_or("Unable to embed a system font in the PDF")?,
                    );
                }
                let font = self.cache[&key].clone();
                let size = run.font_size();
                for cluster in run.visual_clusters() {
                    if cluster.is_ligature_continuation() {
                        if let Some(last) = result.runs.last_mut().and_then(|r| r.glyphs.last_mut())
                        {
                            last.text_range.end = cluster.text_range().end;
                        }
                        continue;
                    }
                    let mut glyphs = Vec::new();
                    let mut ink = Ink::default();
                    let mut advance = 0.0;
                    for glyph in cluster.glyphs() {
                        if glyph.id == 0
                            && !text[cluster.text_range()].chars().all(char::is_whitespace)
                        {
                            return Err(format!(
                                "No installed font can render {:?}. Install a font for this language and try again.",
                                &text[cluster.text_range()]
                            ));
                        }
                        ink = layout.styles()[glyph.style_index as usize].brush.clone();
                        glyphs.push(KrillaGlyph::new(
                            GlyphId::new(glyph.id),
                            glyph.advance / size,
                            glyph.x / size,
                            glyph.y / size,
                            0.0,
                            cluster.text_range(),
                            None,
                        ));
                        advance += glyph.advance;
                    }
                    result.runs.push(Run {
                        x,
                        glyphs,
                        font: font.clone(),
                        size,
                        text: text.clone(),
                        ink,
                        width: advance,
                    });
                    x += advance;
                }
            }
            lines.push(result);
        }
        Ok(lines)
    }
}

struct Positioned {
    line: Line,
    x: f32,
    y: f32,
    width: f32,
    shaded: bool,
    cell: bool,
}
struct Pages {
    pages: Vec<Vec<Positioned>>,
    y: f32,
}
impl Pages {
    fn new() -> Self {
        Self {
            pages: vec![Vec::new()],
            y: MARGIN,
        }
    }
    fn room(&mut self, height: f32) {
        if self.y + height > HEIGHT - MARGIN {
            self.pages.push(Vec::new());
            self.y = MARGIN;
        }
    }
    fn text(&mut self, lines: Vec<Line>, b: &Block) {
        if b.size >= 13.0 {
            self.room(lines.iter().map(|l| l.height).sum::<f32>() + 32.0);
        }
        for line in lines {
            self.room(line.height);
            let height = line.height;
            self.pages.last_mut().unwrap().push(Positioned {
                line,
                x: MARGIN + b.indent + 5.0,
                y: self.y,
                width: TEXT_WIDTH - b.indent,
                shaded: b.shaded,
                cell: false,
            });
            self.y += height;
        }
        self.y += if b.size >= 13.0 { 8.0 } else { 6.0 };
    }
    fn row(&mut self, cells: Vec<Vec<Line>>, header: bool) {
        if cells.is_empty() {
            return;
        }
        let width = TEXT_WIDTH / cells.len() as f32;
        let mut cells: Vec<_> = cells
            .into_iter()
            .map(|cell| cell.into_iter().peekable())
            .collect();
        // Split oversized rows at line boundaries so a cell can never run off the page.
        while cells.iter_mut().any(|c| c.peek().is_some()) {
            let height = cells
                .iter_mut()
                .filter_map(|c| c.peek())
                .map(|l| l.height)
                .fold(16.0, f32::max);
            self.room(height + 4.0);
            for (index, cell) in cells.iter_mut().enumerate() {
                let mut line = cell.next().unwrap_or(Line {
                    runs: Vec::new(),
                    height,
                    baseline: 12.0,
                });
                line.height = height + 4.0;
                self.pages.last_mut().unwrap().push(Positioned {
                    line,
                    x: MARGIN + index as f32 * width + 5.0,
                    y: self.y,
                    width,
                    shaded: header,
                    cell: true,
                });
            }
            self.y += height + 4.0;
        }
    }
}

/// An A4 PDF with embedded system fonts, selectable Unicode text and clickable source links.
pub fn pdf(body: &str) -> Result<Vec<u8>, String> {
    let mut typesetter = Typesetter {
        fonts: FontContext::default(),
        layout: LayoutContext::new(),
        cache: HashMap::new(),
    };
    let mut pages = Pages::new();
    for part in blocks(body) {
        match part {
            Part::Text(b) => {
                let lines = typesetter.lines(&b, (TEXT_WIDTH - b.indent - 10.0).max(40.0))?;
                pages.text(lines, &b);
            }
            Part::Row(cells, header) => {
                let width = TEXT_WIDTH / cells.len().max(1) as f32 - 10.0;
                let lines = cells
                    .iter()
                    .map(|b| typesetter.lines(b, width.max(10.0)))
                    .collect::<Result<Vec<_>, _>>()?;
                pages.row(lines, header);
            }
        }
    }
    let mut document = Document::new();
    for lines in pages.pages {
        let mut page = document.start_page_with(PageSettings::from_wh(WIDTH, HEIGHT).unwrap());
        let mut annotations = Vec::new();
        let mut surface = page.surface();
        for p in lines {
            if p.shaded || p.cell {
                let mut path = PathBuilder::new();
                path.push_rect(Rect::from_xywh(p.x - 5.0, p.y, p.width, p.line.height).unwrap());
                surface.set_fill(Some(Fill {
                    paint: rgb::Color::new(243, 245, 248).into(),
                    ..Default::default()
                }));
                if p.cell {
                    surface.set_stroke(Some(krilla::paint::Stroke {
                        paint: rgb::Color::new(210, 216, 223).into(),
                        width: 0.4,
                        ..Default::default()
                    }));
                }
                if p.shaded {
                    surface.draw_path(&path.finish().unwrap());
                } else {
                    surface.set_fill(None);
                    surface.draw_path(&path.finish().unwrap());
                }
                surface.set_stroke(None);
            }
            let mut links: Vec<(String, f32, f32)> = Vec::new();
            for run in p.line.runs {
                surface.set_fill(Some(Fill {
                    paint: if run.ink.link.is_some() {
                        rgb::Color::new(30, 90, 170)
                    } else {
                        rgb::Color::new(30, 35, 42)
                    }
                    .into(),
                    ..Default::default()
                }));
                surface.draw_glyphs(
                    Point::from_xy(p.x + run.x, p.y + p.line.baseline),
                    &run.glyphs,
                    run.font,
                    &run.text,
                    run.size,
                    false,
                );
                if let Some(url) = run.ink.link {
                    if let Some((last, _, end)) = links.last_mut()
                        && last == &url
                        && (run.x - *end).abs() < 0.1
                    {
                        *end = run.x + run.width;
                    } else {
                        links.push((url, run.x, run.x + run.width));
                    }
                }
            }
            for (url, start, end) in links {
                if let Some(rect) = Rect::from_xywh(p.x + start, p.y, end - start, p.line.height) {
                    annotations.push(Annotation::new_link(
                        LinkAnnotation::new(
                            rect,
                            Target::Action(LinkAction::new(url.clone()).into()),
                        ),
                        Some(url),
                    ));
                }
            }
        }
        surface.finish();
        for annotation in annotations {
            page.add_annotation(annotation);
        }
        page.finish();
    }
    document
        .finish()
        .map_err(|e| format!("Unable to create PDF: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markdown_retains_complete_utf8_source() {
        let source = format!("## 中文\n\n{}\n\nEND\n", "long answer 👩‍💻\n".repeat(500));
        assert_eq!(markdown(&source), source.as_bytes());
    }
    #[test]
    fn pdf_bold_ranges_follow_chinese_display_without_changing_markdown_export() {
        let source = "**时间点很重要：**截至\n\n`**代码：**截至`";
        assert_eq!(markdown(source), source.as_bytes());
        let parts = blocks(source);
        let Part::Text(first) = &parts[0] else { panic!("paragraph expected") };
        assert_eq!(first.text, "时间点很重要：截至");
        assert_eq!(first.bold, vec![0.."时间点很重要：".len()]);
        let Part::Text(second) = &parts[1] else { panic!("code paragraph expected") };
        assert_eq!(second.text, "**代码：**截至");
    }
    #[test]
    fn pdf_paginates_and_embeds_fonts_and_source_links() {
        let source = format!(
            "# 中文答案\n\n**结论** [来源](https://example.com/report)\n\n| 模型 | 速度 |\n|---|---|\n| 测试 | 120 tokens/s |\n\n```rust\nlet value = 42;\n```\n\n{}\n\n终点 FINAL-END",
            "长文分页验证。Full answer retained.\n\n".repeat(150)
        );
        let bytes = pdf(&source).unwrap();
        let doc = lopdf::Document::load_mem(&bytes).unwrap();
        let pages: Vec<_> = doc.get_pages().keys().copied().collect();
        assert!(pages.len() > 2);
        // lopdf 0.36 cannot read krilla's ToUnicode CMap format. Verify the
        // structure here; the real-answer acceptance check uses MuPDF text extraction.
        for page in doc.get_pages().values() {
            assert!(!doc.get_page_content(*page).unwrap().is_empty());
        }
        assert!(doc.objects.values().any(|o| {
            o.as_dict()
                .is_ok_and(|d| d.get(b"FontFile2").is_ok() || d.get(b"FontFile3").is_ok())
        }));
        assert!(doc.objects.values().any(|o| o.as_dict().is_ok_and(|d| {
            d.get(b"A")
                .and_then(|a| a.as_dict())
                .is_ok_and(|a| a.get(b"URI").is_ok())
        })));
    }
}
