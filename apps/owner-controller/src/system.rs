//! macOS user preferences the controller follows: appearance, reduced
//! motion and preferred language. Read through CoreFoundation (already
//! linked by Makepad) so the controller needs no extra crates.
use owner_dashboard::Locale;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemPrefs {
    pub dark: bool,
    pub reduced_motion: bool,
    pub locale: Locale,
}

impl SystemPrefs {
    pub fn read() -> Self {
        imp::read()
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::SystemPrefs;
    use owner_dashboard::Locale;
    use std::ffi::{CString, c_char, c_void};

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFIndex = isize;
    const UTF8: u32 = 0x0800_0100;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFPreferencesAnyApplication: CFStringRef;
        fn CFPreferencesCopyAppValue(key: CFStringRef, app: CFStringRef) -> CFTypeRef;
        fn CFPreferencesAppSynchronize(app: CFStringRef) -> u8;
        fn CFStringCreateWithCString(alloc: *const c_void, s: *const c_char, encoding: u32) -> CFStringRef;
        fn CFStringGetCString(s: CFStringRef, buffer: *mut c_char, size: CFIndex, encoding: u32) -> u8;
        fn CFGetTypeID(cf: CFTypeRef) -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFBooleanGetTypeID() -> usize;
        fn CFBooleanGetValue(b: CFTypeRef) -> u8;
        fn CFNumberGetTypeID() -> usize;
        fn CFNumberGetValue(n: CFTypeRef, number_type: CFIndex, out: *mut c_void) -> u8;
        fn CFArrayGetCount(a: CFTypeRef) -> CFIndex;
        fn CFArrayGetValueAtIndex(a: CFTypeRef, i: CFIndex) -> CFTypeRef;
        fn CFLocaleCopyPreferredLanguages() -> CFTypeRef;
        fn CFRelease(cf: CFTypeRef);
    }

    struct Owned(CFTypeRef);
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: we own one reference from a Create/Copy call.
                unsafe { CFRelease(self.0) }
            }
        }
    }

    fn cfstr(s: &str) -> Owned {
        let c = CString::new(s).unwrap_or_default();
        // SAFETY: valid NUL-terminated UTF-8 input.
        Owned(unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8) })
    }

    fn to_string(value: CFTypeRef) -> Option<String> {
        // SAFETY: type-checked before use; buffer is sized and NUL-terminated.
        unsafe {
            if value.is_null() || CFGetTypeID(value) != CFStringGetTypeID() {
                return None;
            }
            let mut buffer = [0 as c_char; 128];
            if CFStringGetCString(value, buffer.as_mut_ptr(), buffer.len() as CFIndex, UTF8) == 0 {
                return None;
            }
            Some(std::ffi::CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned())
        }
    }

    fn pref(key: &str, domain: Option<&str>) -> Owned {
        let key = cfstr(key);
        let domain_ref = domain.map(cfstr);
        // SAFETY: valid CFString arguments; the result is owned (Copy rule).
        unsafe {
            let app = domain_ref.as_ref().map(|d| d.0).unwrap_or(kCFPreferencesAnyApplication);
            CFPreferencesAppSynchronize(app);
            Owned(CFPreferencesCopyAppValue(key.0, app))
        }
    }

    fn truthy(value: CFTypeRef) -> bool {
        // SAFETY: type-checked before reading.
        unsafe {
            if value.is_null() {
                return false;
            }
            let ty = CFGetTypeID(value);
            if ty == CFBooleanGetTypeID() {
                return CFBooleanGetValue(value) != 0;
            }
            if ty == CFNumberGetTypeID() {
                let mut n: i64 = 0;
                // kCFNumberSInt64Type = 4
                return CFNumberGetValue(value, 4, &mut n as *mut i64 as *mut c_void) != 0 && n != 0;
            }
            false
        }
    }

    fn first_language() -> Option<String> {
        // SAFETY: owned array; index checked against its count.
        unsafe {
            let languages = Owned(CFLocaleCopyPreferredLanguages());
            if languages.0.is_null() || CFArrayGetCount(languages.0) == 0 {
                return None;
            }
            to_string(CFArrayGetValueAtIndex(languages.0, 0))
        }
    }

    pub fn read() -> SystemPrefs {
        let style = pref("AppleInterfaceStyle", None);
        let motion = pref("reduceMotion", Some("com.apple.universalaccess"));
        SystemPrefs {
            dark: to_string(style.0).is_some_and(|s| s.eq_ignore_ascii_case("dark")),
            reduced_motion: truthy(motion.0),
            locale: first_language().map(|l| Locale::from_tag(&l)).unwrap_or_default(),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::SystemPrefs;
    use owner_dashboard::Locale;
    pub fn read() -> SystemPrefs {
        let tag = std::env::var("LANG").unwrap_or_default();
        SystemPrefs { dark: false, reduced_motion: false, locale: Locale::from_tag(&tag) }
    }
}
