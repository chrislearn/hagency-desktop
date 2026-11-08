//! PDFKit text extraction. The host runs this in its bounded parser child.
use objc2::{
    msg_send,
    rc::{Allocated, Retained, autoreleasepool},
    runtime::{AnyClass, AnyObject},
};
use objc2_foundation::{NSData, NSString};
use std::io;

#[link(name = "PDFKit", kind = "framework")]
unsafe extern "C" {}

/// Visit up to `max_pages` text layers; returning false stops immediately.
/// Returns the document's total page count. No scripts, links, files or URLs
/// are opened. `bytes` is copied into owned NSData before PDFKit retains it.
pub fn visit(
    bytes: &[u8],
    max_pages: usize,
    mut page_text: impl FnMut(usize, &str) -> bool,
) -> io::Result<usize> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "unreadable or protected PDF");
    if bytes.len() > 8 * 1024 * 1024 || max_pages > 200 {
        return Err(invalid());
    }
    autoreleasepool(|_| unsafe {
        // These selectors are PDFDocument/PDFPage's documented synchronous
        // interfaces. Retained owns every alloc/init or autoreleased result;
        // no borrowed Objective-C object outlives this autorelease pool.
        let class = AnyClass::get(c"PDFDocument").ok_or_else(invalid)?;
        let data = NSData::with_bytes(bytes);
        let allocated: Allocated<AnyObject> = msg_send![class, alloc];
        let document: Option<Retained<AnyObject>> = msg_send![allocated, initWithData: &*data];
        let document = document.ok_or_else(invalid)?;
        let locked: bool = msg_send![&*document, isLocked];
        if locked {
            return Err(invalid());
        }
        let total_pages: usize = msg_send![&*document, pageCount];
        for index in 0..total_pages.min(max_pages) {
            let page: Option<Retained<AnyObject>> = msg_send![&*document, pageAtIndex: index];
            let page = page.ok_or_else(invalid)?;
            let text: Option<Retained<NSString>> = msg_send![&*page, string];
            if !page_text(index + 1, &text.map(|s| s.to_string()).unwrap_or_default()) {
                break;
            }
        }
        Ok(total_pages)
    })
}
