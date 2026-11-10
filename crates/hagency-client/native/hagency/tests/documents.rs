//! Runs the same early CLI entry the Desktop/owner daemon uses for PDF parsing.
use lopdf::{
    Document, Object, Stream,
    content::{Content, Operation},
    dictionary,
};
use std::{
    io::Write,
    process::{Command, Stdio},
};

fn pdf(chinese: bool, pages: usize) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let page_tree = doc.new_object_id();
    let font = if chinese {
        let cmap = b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n2 beginbfchar\n<0001> <4E2D>\n<0002> <6587>\nendbfchar\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend";
        let unicode = doc.add_object(Stream::new(dictionary! {}, cmap.to_vec()));
        let descendant = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "CIDFontType0", "BaseFont" => "HeiseiMin-W3",
            "CIDSystemInfo" => dictionary! {"Registry" => Object::string_literal("Adobe"), "Ordering" => Object::string_literal("Japan1"), "Supplement" => 0},
        });
        doc.add_object(dictionary! {"Type" => "Font", "Subtype" => "Type0", "BaseFont" => "HeiseiMin-W3", "Encoding" => "Identity-H", "DescendantFonts" => vec![descendant.into()], "ToUnicode" => unicode})
    } else {
        doc.add_object(
            dictionary! {"Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica"},
        )
    };
    let resources = doc.add_object(dictionary! {"Font" => dictionary! {"F1" => font}});
    let mut kids = Vec::new();
    for index in 0..pages {
        let text = if chinese {
            vec![0, 1, 0, 2]
        } else {
            format!("Document body page {}", index + 1).into_bytes()
        };
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 14.into()]),
                Operation::new("Td", vec![40.into(), 700.into()]),
                Operation::new("Tj", vec![Object::string_literal(text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let stream = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        kids.push(
            doc.add_object(
                dictionary! {"Type" => "Page", "Parent" => page_tree, "Contents" => stream},
            )
            .into(),
        );
    }
    doc.objects.insert(page_tree, dictionary! {"Type" => "Pages", "Kids" => kids, "Count" => pages as i64, "Resources" => resources, "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()]}.into());
    let catalog = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => page_tree});
    doc.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

fn extract(bytes: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_hagency-document-probe"))
        .arg("--hagency-extract-document")
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn pdf_worker_reads_real_document_pages_and_chinese_unicode() {
    for chinese in [false, true] {
        let out = extract(&pdf(chinese, 2));
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(text["complete"], true);
        assert_eq!(text["pages_read"], 2);
        assert_eq!(text["total_pages"], 2);
        assert!(text["text"].as_str().unwrap().contains(if chinese {
            "中文"
        } else {
            "Document body page 2"
        }));
    }
}

#[test]
fn pdf_worker_rejects_malformed_data_and_reports_the_page_limit() {
    let invalid = extract(b"%PDF-1.7\ninvalid document");
    assert!(!invalid.status.success());
    assert!(invalid.stdout.is_empty());
    let out = extract(&pdf(false, 201));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(text["complete"], false);
    assert_eq!(text["pages_read"], 200);
    assert_eq!(text["total_pages"], 201);
}
