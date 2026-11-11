//! Read native Claude credentials without prompting or changing Keychain items.
use std::ffi::{c_char, c_void, CString};

type Ref = *const c_void;

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecClass: Ref;
    static kSecClassGenericPassword: Ref;
    static kSecAttrService: Ref;
    static kSecAttrModificationDate: Ref;
    static kSecAttrCreationDate: Ref;
    static kSecReturnAttributes: Ref;
    static kSecReturnPersistentRef: Ref;
    static kSecValuePersistentRef: Ref;
    static kSecReturnData: Ref;
    static kSecMatchLimit: Ref;
    static kSecMatchLimitAll: Ref;
    static kSecMatchLimitOne: Ref;
    static kSecUseAuthenticationUI: Ref;
    static kSecUseAuthenticationUIFail: Ref;
    fn SecItemCopyMatching(query: Ref, result: *mut Ref) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: Ref;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
    fn CFStringCreateWithCString(allocator: Ref, value: *const c_char, encoding: u32) -> Ref;
    fn CFDictionaryCreate(
        allocator: Ref,
        keys: *const Ref,
        values: *const Ref,
        count: isize,
        key_callbacks: Ref,
        value_callbacks: Ref,
    ) -> Ref;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFDictionaryGetValue(dictionary: Ref, key: Ref) -> Ref;
    fn CFArrayGetTypeID() -> usize;
    fn CFArrayGetCount(array: Ref) -> isize;
    fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
    fn CFGetTypeID(value: Ref) -> usize;
    fn CFDateGetTypeID() -> usize;
    fn CFDateGetAbsoluteTime(date: Ref) -> f64;
    fn CFDataGetTypeID() -> usize;
    fn CFDataGetLength(value: Ref) -> isize;
    fn CFDataGetBytePtr(value: Ref) -> *const u8;
    fn CFRelease(value: Ref);
}

struct Owned(Ref);
impl Owned {
    fn new(value: Ref) -> Option<Self> {
        if value.is_null() {
            None
        } else {
            Some(Self(value))
        }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        // Only Create/Copy-owned, non-null CF objects enter this guard.
        unsafe { CFRelease(self.0) }
    }
}

fn matching(pairs: &[(Ref, Ref)]) -> Option<Owned> {
    let keys: Vec<_> = pairs.iter().map(|pair| pair.0).collect();
    let values: Vec<_> = pairs.iter().map(|pair| pair.1).collect();
    // All pointers are live CF objects; the standard callbacks retain query values.
    unsafe {
        let query = Owned::new(CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            keys.len() as isize,
            std::ptr::addr_of!(kCFTypeDictionaryKeyCallBacks).cast(),
            std::ptr::addr_of!(kCFTypeDictionaryValueCallBacks).cast(),
        ))?;
        let mut result = std::ptr::null();
        let status = SecItemCopyMatching(query.0, &mut result);
        let result = Owned::new(result)?;
        (status == 0).then_some(result)
    }
}

/// Pick the newest service item, like CodexBar, then read that exact persistent ref.
pub(crate) fn read_usage_secure_store(service: &str) -> Option<Vec<u8>> {
    let service = CString::new(service).ok()?;
    // Result children are borrowed only while their owning array remains live.
    // AuthenticationUIFail makes locked/unauthorized items a miss, never a prompt.
    unsafe {
        let service = Owned::new(CFStringCreateWithCString(
            std::ptr::null(),
            service.as_ptr(),
            0x0800_0100,
        ))?;
        let items = matching(&[
            (kSecClass, kSecClassGenericPassword),
            (kSecAttrService, service.0),
            (kSecMatchLimit, kSecMatchLimitAll),
            (kSecReturnAttributes, kCFBooleanTrue),
            (kSecReturnPersistentRef, kCFBooleanTrue),
            (kSecUseAuthenticationUI, kSecUseAuthenticationUIFail),
        ])?;
        if CFGetTypeID(items.0) != CFArrayGetTypeID() {
            return None;
        }
        let count = CFArrayGetCount(items.0);
        if !(1..=1024).contains(&count) {
            return None;
        }
        let selected = (0..count)
            .filter_map(|index| {
                let item = CFArrayGetValueAtIndex(items.0, index);
                if item.is_null() || CFGetTypeID(item) != CFDictionaryGetTypeID() {
                    return None;
                }
                let reference = CFDictionaryGetValue(item, kSecValuePersistentRef);
                if reference.is_null() || CFGetTypeID(reference) != CFDataGetTypeID() {
                    return None;
                }
                let date = [kSecAttrModificationDate, kSecAttrCreationDate]
                    .into_iter()
                    .find_map(|key| {
                        let value = CFDictionaryGetValue(item, key);
                        if value.is_null() || CFGetTypeID(value) != CFDateGetTypeID() {
                            return None;
                        }
                        let value = CFDateGetAbsoluteTime(value);
                        value.is_finite().then_some(value)
                    })
                    .unwrap_or(f64::NEG_INFINITY);
                Some((reference, date))
            })
            .max_by(|left, right| left.1.total_cmp(&right.1))?;
        let data = matching(&[
            (kSecClass, kSecClassGenericPassword),
            (kSecValuePersistentRef, selected.0),
            (kSecMatchLimit, kSecMatchLimitOne),
            (kSecReturnData, kCFBooleanTrue),
            (kSecUseAuthenticationUI, kSecUseAuthenticationUIFail),
        ])?;
        if CFGetTypeID(data.0) != CFDataGetTypeID() {
            return None;
        }
        let length = CFDataGetLength(data.0);
        let pointer = CFDataGetBytePtr(data.0);
        if !(1..=1_048_576).contains(&length) || pointer.is_null() {
            return None;
        }
        Some(std::slice::from_raw_parts(pointer, length as usize).to_vec())
    }
}
