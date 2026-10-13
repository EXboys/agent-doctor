//! Keep a file drag from quitting the app.
//!
//! The window library reads the legacy filenames list with `unwrap`. A drag
//! that only carries file URLs, or a list that is not an array of strings,
//! used to abort on the main thread. This makes that read return an array of
//! path strings instead.

use std::ffi::{c_char, CStr};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Once;

use objc2::exception::catch;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{msg_send, sel};

type PropertyListImp =
    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> *mut AnyObject;

static ORIGINAL: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());

pub(crate) fn keep_file_drags_from_aborting() {
    static START: Once = Once::new();
    START.call_once(|| unsafe {
        let Some(class) = AnyClass::get(c"NSPasteboard") else {
            return;
        };
        let Some(method) = class.instance_method(sel!(propertyListForType:)) else {
            return;
        };
        let previous = method.implementation();
        ORIGINAL.store(previous as *mut (), Ordering::Relaxed);
        let replacement: Imp = std::mem::transmute(property_list_for_type as PropertyListImp);
        let _ = method.set_implementation(replacement);
    });
}

unsafe extern "C-unwind" fn property_list_for_type(
    this: *mut AnyObject,
    cmd: Sel,
    kind: *mut AnyObject,
) -> *mut AnyObject {
    let original_ptr = ORIGINAL.load(Ordering::Relaxed);
    if original_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let original: PropertyListImp = std::mem::transmute(original_ptr);
    let found = original(this, cmd, kind);
    if this.is_null() || !is_filenames_kind(kind) {
        return found;
    }
    match catch(AssertUnwindSafe(|| filenames_list(this, found))) {
        Ok(list) => list,
        Err(_) => empty_array().unwrap_or(found),
    }
}

unsafe fn is_filenames_kind(kind: *mut AnyObject) -> bool {
    utf8_bytes(kind) == Some(b"NSFilenamesPboardType".as_slice())
}

unsafe fn filenames_list(pasteboard: *mut AnyObject, found: *mut AnyObject) -> *mut AnyObject {
    if string_list_is_safe(found) {
        return found;
    }
    if let Some(paths) = file_url_paths(pasteboard) {
        if let Some(list) = array_of_strings(&paths) {
            return list;
        }
    }
    if let Some(list) = safe_strings_from(found) {
        if let Some(array) = array_of_pointers(&list) {
            return array;
        }
    }
    empty_array().unwrap_or(found)
}

unsafe fn string_list_is_safe(found: *mut AnyObject) -> bool {
    if !is_kind(found, c"NSArray") {
        return false;
    }
    let count: usize = msg_send![found, count];
    for index in 0..count {
        let item: *mut AnyObject = msg_send![found, objectAtIndex: index];
        if utf8_bytes(item).is_none() {
            return false;
        }
    }
    true
}

unsafe fn safe_strings_from(found: *mut AnyObject) -> Option<Vec<*mut AnyObject>> {
    if found.is_null() {
        return None;
    }
    if is_kind(found, c"NSString") && utf8_bytes(found).is_some() {
        return Some(vec![found]);
    }
    if !is_kind(found, c"NSArray") {
        return None;
    }
    let count: usize = msg_send![found, count];
    let mut kept = Vec::new();
    for index in 0..count {
        let item: *mut AnyObject = msg_send![found, objectAtIndex: index];
        if utf8_bytes(item).is_some() {
            kept.push(item);
        }
    }
    if kept.is_empty() {
        None
    } else {
        Some(kept)
    }
}

unsafe fn file_url_paths(pasteboard: *mut AnyObject) -> Option<Vec<*mut AnyObject>> {
    let url_class = AnyClass::get(c"NSURL")?;
    let classes = array_of_pointers(&[url_class as *const _ as *mut AnyObject])?;
    let key = ns_string(c"NSPasteboardURLReadingFileURLsOnlyKey")?;
    let yes = number_yes()?;
    let options = dictionary(key, yes)?;
    let urls: *mut AnyObject =
        msg_send![pasteboard, readObjectsForClasses: classes, options: options];
    if !is_kind(urls, c"NSArray") {
        return None;
    }
    let count: usize = msg_send![urls, count];
    let mut paths = Vec::new();
    for index in 0..count {
        let url: *mut AnyObject = msg_send![urls, objectAtIndex: index];
        if url.is_null() {
            continue;
        }
        let path: *mut AnyObject = msg_send![url, path];
        if utf8_bytes(path).is_some() {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        None
    } else {
        Some(paths)
    }
}

unsafe fn array_of_strings(paths: &[*mut AnyObject]) -> Option<*mut AnyObject> {
    array_of_pointers(paths)
}

unsafe fn array_of_pointers(items: &[*mut AnyObject]) -> Option<*mut AnyObject> {
    let class = AnyClass::get(c"NSMutableArray")?;
    let array: *mut AnyObject = msg_send![class, arrayWithCapacity: items.len()];
    if array.is_null() {
        return None;
    }
    for item in items {
        if !item.is_null() {
            let _: () = msg_send![array, addObject: *item];
        }
    }
    Some(array)
}

unsafe fn empty_array() -> Option<*mut AnyObject> {
    let class = AnyClass::get(c"NSArray")?;
    let array: *mut AnyObject = msg_send![class, array];
    if array.is_null() {
        None
    } else {
        Some(array)
    }
}

unsafe fn dictionary(key: *mut AnyObject, value: *mut AnyObject) -> Option<*mut AnyObject> {
    let class = AnyClass::get(c"NSDictionary")?;
    let dict: *mut AnyObject = msg_send![class, dictionaryWithObject: value, forKey: key];
    if dict.is_null() {
        None
    } else {
        Some(dict)
    }
}

unsafe fn number_yes() -> Option<*mut AnyObject> {
    let class = AnyClass::get(c"NSNumber")?;
    let number: *mut AnyObject = msg_send![class, numberWithBool: true];
    if number.is_null() {
        None
    } else {
        Some(number)
    }
}

unsafe fn ns_string(text: &CStr) -> Option<*mut AnyObject> {
    let class = AnyClass::get(c"NSString")?;
    let string: *mut AnyObject = msg_send![class, stringWithUTF8String: text.as_ptr()];
    if string.is_null() {
        None
    } else {
        Some(string)
    }
}

unsafe fn is_kind(object: *mut AnyObject, class_name: &CStr) -> bool {
    if object.is_null() {
        return false;
    }
    let Some(class) = AnyClass::get(class_name) else {
        return false;
    };
    let yes: bool = msg_send![object, isKindOfClass: class];
    yes
}

unsafe fn utf8_bytes<'a>(object: *mut AnyObject) -> Option<&'a [u8]> {
    if !is_kind(object, c"NSString") {
        return None;
    }
    let bytes: *const c_char = msg_send![object, UTF8String];
    if bytes.is_null() {
        return None;
    }
    Some(CStr::from_ptr(bytes).to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pasteboard() -> *mut AnyObject {
        keep_file_drags_from_aborting();
        let class = AnyClass::get(c"NSPasteboard").expect("NSPasteboard");
        let name = unsafe { ns_string(c"agent-doctor.drag-test") }.expect("name");
        let board: *mut AnyObject = unsafe { msg_send![class, pasteboardWithName: name] };
        assert!(!board.is_null());
        unsafe {
            let _: () = msg_send![board, clearContents];
        }
        board
    }

    fn filenames_kind() -> *mut AnyObject {
        unsafe { ns_string(c"NSFilenamesPboardType") }.expect("type")
    }

    unsafe fn list_paths(board: *mut AnyObject) -> Vec<String> {
        let kind = filenames_kind();
        let list: *mut AnyObject = msg_send![board, propertyListForType: kind];
        assert!(
            string_list_is_safe(list),
            "filenames list must be an array of strings"
        );
        let count: usize = msg_send![list, count];
        let mut paths = Vec::new();
        for index in 0..count {
            let item: *mut AnyObject = msg_send![list, objectAtIndex: index];
            let bytes = utf8_bytes(item).expect("path");
            paths.push(String::from_utf8_lossy(bytes).into_owned());
        }
        paths
    }

    #[test]
    fn missing_filenames_list_is_an_empty_array() {
        let board = pasteboard();
        let paths = unsafe { list_paths(board) };
        assert!(paths.is_empty());
    }

    #[test]
    fn existing_filenames_list_is_kept() {
        let board = pasteboard();
        unsafe {
            let kind = filenames_kind();
            let path = ns_string(c"/tmp/agent-doctor-drag.txt").expect("path");
            let list = array_of_pointers(&[path]).expect("list");
            let types = array_of_pointers(&[kind]).expect("types");
            let _: () = msg_send![board, declareTypes: types, owner: std::ptr::null::<AnyObject>()];
            let wrote: bool = msg_send![board, setPropertyList: list, forType: kind];
            assert!(wrote);
        }
        let paths = unsafe { list_paths(board) };
        assert_eq!(paths, vec!["/tmp/agent-doctor-drag.txt".to_string()]);
    }

    #[test]
    fn file_url_drag_becomes_a_path_list() {
        let board = pasteboard();
        unsafe {
            let path = ns_string(c"/tmp/agent-doctor-drag.txt").expect("path");
            let url_class = AnyClass::get(c"NSURL").expect("NSURL");
            let url: *mut AnyObject = msg_send![url_class, fileURLWithPath: path];
            assert!(!url.is_null());
            let items = array_of_pointers(&[url]).expect("items");
            let wrote: bool = msg_send![board, writeObjects: items];
            assert!(wrote);
        }
        let paths = unsafe { list_paths(board) };
        assert_eq!(paths, vec!["/tmp/agent-doctor-drag.txt".to_string()]);
    }
}
