//! Own-process AppKit AX fixture probe. Never compiled into production hosts.
//! This walks only content views registered by this fixture's Makepad process.
use makepad_widgets::makepad_platform::{
    os::apple::{apple_sys::*, macos::macos_app::try_with_macos_app},
    WindowId,
};
use serde_json::{Value, json};
use std::{collections::HashSet, ffi::CStr};

unsafe fn string(value: ObjcId) -> Option<String> {
    if value.is_null() { return None; }
    let is_string: BOOL = unsafe { msg_send![value, isKindOfClass: class!(NSString)] };
    if is_string != YES { return None; }
    let ptr: *const std::ffi::c_char = unsafe { msg_send![value, UTF8String] };
    (!ptr.is_null()).then(|| unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() })
}

unsafe fn walk(object: ObjcId, action: &str, seen: &mut HashSet<usize>, depth: usize) -> Value {
    if object.is_null() || depth > 40 || seen.len() > 16_384 || !seen.insert(object as usize) { return Value::Null; }
    let role: ObjcId = unsafe { msg_send![object, accessibilityRole] };
    let title: ObjcId = unsafe { msg_send![object, accessibilityTitle] };
    let label: ObjcId = unsafe { msg_send![object, accessibilityLabel] };
    let identifier: ObjcId = unsafe { msg_send![object, accessibilityIdentifier] };
    let value: ObjcId = unsafe { msg_send![object, accessibilityValue] };
    let frame: NSRect = unsafe { msg_send![object, accessibilityFrame] };
    let enabled: BOOL = unsafe { msg_send![object, isAccessibilityEnabled] };
    let id = unsafe { string(identifier) };
    let mut performed = false;
    let fields: Vec<_> = action.splitn(3, ':').collect();
    let (operation, target, payload) = if fields.len() >= 2 { (fields[0], fields[1], fields.get(2).copied().unwrap_or("")) }
        else { ("press", action, "") };
    if !target.is_empty() && id.as_deref() == Some(target) {
        match operation {
            "press" => {
                let result: BOOL = unsafe { msg_send![object, accessibilityPerformPress] };
                performed = result == YES;
            }
            "focus" => {
                let _: () = unsafe { msg_send![object, setAccessibilityFocused: YES] };
                performed = true;
            }
            "value" => {
                let value = str_to_nsstring(payload);
                let _: () = unsafe { msg_send![object, setAccessibilityValue: value] };
                performed = true;
            }
            "selection" => {
                if let Some((start, length)) = payload.split_once(',').and_then(|(s,l)| Some((s.parse().ok()?,l.parse().ok()?))) {
                    let range = NSRange {location: start, length};
                    let _: () = unsafe { msg_send![object, setAccessibilitySelectedTextRange: range] };
                    performed = true;
                }
            }
            _ => {}
        }
    }
    let children: ObjcId = unsafe { msg_send![object, accessibilityChildren] };
    let count: usize = if children.is_null() { 0 } else { unsafe { msg_send![children, count] } };
    let children: Vec<_> = (0..count.min(16_384)).map(|i| {
        let child: ObjcId = unsafe { msg_send![children, objectAtIndex: i] };
        unsafe { walk(child, action, seen, depth + 1) }
    }).filter(|v| !v.is_null()).collect();
    let value_text = unsafe { string(value) }.or_else(|| {
        if value.is_null() { return None; }
        let number: BOOL = unsafe { msg_send![value, isKindOfClass: class!(NSNumber)] };
        if number != YES { return None; }
        let description: ObjcId = unsafe { msg_send![value, description] };
        unsafe { string(description) }
    });
    json!({"role": unsafe { string(role) }, "title": unsafe { string(title) },
        "label": unsafe { string(label) }, "id": id, "value": value_text,
        "enabled": enabled == YES, "pressed": performed,
        "frame": [frame.origin.x, frame.origin.y, frame.size.width, frame.size.height], "children": children})
}

/// Queue a real NSTextInputClient call after the current Makepad callback has
/// returned. NSInvocation retains its target and arguments until the main-runloop
/// selector runs; this avoids re-entering Cx from the fixture's Custom event.
unsafe fn queue_ime(view: ObjcId, operation: &str, text: &str) -> bool {
    let selector = match operation {
        "ime-mark" => sel!(setMarkedText:selectedRange:replacementRange:),
        "ime-commit" => sel!(insertText:replacementRange:),
        _ => return false,
    };
    let signature: ObjcId = unsafe { msg_send![view, methodSignatureForSelector: selector] };
    if signature.is_null() { return false; }
    let invocation: ObjcId = unsafe { msg_send![class!(NSInvocation), invocationWithMethodSignature: signature] };
    let _: () = unsafe { msg_send![invocation, setTarget: view] };
    let _: () = unsafe { msg_send![invocation, setSelector: selector] };
    let value = str_to_nsstring(text);
    let value_ptr = (&value as *const ObjcId).cast::<std::ffi::c_void>();
    let _: () = unsafe { msg_send![invocation, setArgument: value_ptr atIndex: 2usize] };
    let replacement = NSRange {location: u64::MAX, length: 0};
    if operation == "ime-mark" {
        let selection = NSRange {location: text.encode_utf16().count() as u64, length: 0};
        let selection_ptr = (&selection as *const NSRange).cast::<std::ffi::c_void>();
        let _: () = unsafe { msg_send![invocation, setArgument: selection_ptr atIndex: 3usize] };
        let replacement_ptr = (&replacement as *const NSRange).cast::<std::ffi::c_void>();
        let _: () = unsafe { msg_send![invocation, setArgument: replacement_ptr atIndex: 4usize] };
    } else {
        let replacement_ptr = (&replacement as *const NSRange).cast::<std::ffi::c_void>();
        let _: () = unsafe { msg_send![invocation, setArgument: replacement_ptr atIndex: 3usize] };
    }
    let _: () = unsafe { msg_send![invocation, retainArguments] };
    let _: () = unsafe { msg_send![invocation, performSelector: sel!(invoke) withObject: nil afterDelay: 0f64] };
    true
}

pub(super) fn probe(windows: Vec<WindowId>, action: &str) -> Value {
    let mut output = Vec::new();
    for window in windows {
        // Release the application RefCell before calling AppKit: AX can call
        // back synchronously while answering native queries.
        let native = try_with_macos_app(|app| app.cocoa_window_for_id(window)).flatten();
        if let Some(native) = native {
            let view: ObjcId = unsafe { msg_send![native, contentView] };
            let queued = action.split_once(':').filter(|(operation, _)| operation.starts_with("ime-"))
                .is_some_and(|(operation, text)| unsafe { queue_ime(view, operation, text) });
            let marked: BOOL = unsafe { msg_send![view, hasMarkedText] };
            let hit = action.strip_prefix("hit:").and_then(|p| p.split_once(',')).and_then(|(x,y)| Some((x.parse::<f64>().ok()?,y.parse::<f64>().ok()?)))
                .map(|(x,y)| {
                    let target: ObjcId = unsafe { msg_send![view, accessibilityHitTest: NSPoint{x,y}] };
                    unsafe { walk(target, "", &mut HashSet::new(), 0) }
                });
            output.push(json!({"window": window.0, "imeQueued": queued, "hasMarkedText": marked == YES, "hit": hit,
                "native": unsafe { walk(view, action, &mut HashSet::new(), 0) }}));
        }
    }
    json!({"inspection": "own-process AppKit NSAccessibility", "windows": output})
}
