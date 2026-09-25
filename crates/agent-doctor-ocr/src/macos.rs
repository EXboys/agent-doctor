//! Read printed words from a local image with Vision (no extra model or key).

#![allow(unexpected_cfgs)]

use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::NSString;
use std::ffi::CStr;
use std::path::Path;
use std::ptr;

#[link(name = "Vision", kind = "framework")]
extern "C" {}

fn require_class(name: &CStr) -> Option<&'static AnyClass> {
    AnyClass::get(name)
}

unsafe fn nsstring_to_rust(value: *mut AnyObject) -> String {
    if value.is_null() {
        return String::new();
    }
    let utf8: *const std::ffi::c_char = msg_send![value, UTF8String];
    if utf8.is_null() {
        return String::new();
    }
    std::ffi::CStr::from_ptr(utf8)
        .to_string_lossy()
        .trim()
        .to_string()
}

pub fn recognize_text(path: &Path) -> (bool, String) {
    unsafe { recognize_text_inner(path) }
}

unsafe fn recognize_text_inner(path: &Path) -> (bool, String) {
    let Some(url_class) = require_class(c"NSURL") else {
        return (false, String::new());
    };
    let Some(handler_class) = require_class(c"VNImageRequestHandler") else {
        return (false, String::new());
    };
    let Some(request_class) = require_class(c"VNRecognizeTextRequest") else {
        return (false, String::new());
    };
    let Some(array_class) = require_class(c"NSArray") else {
        return (false, String::new());
    };

    let path_ns = NSString::from_str(&path.to_string_lossy());
    let url: *mut AnyObject = msg_send![url_class, fileURLWithPath: &*path_ns];
    if url.is_null() {
        return (false, String::new());
    }

    let handler_alloc: *mut AnyObject = msg_send![handler_class, alloc];
    let nil: *mut AnyObject = ptr::null_mut();
    let handler: *mut AnyObject = msg_send![handler_alloc, initWithURL: url, options: nil];
    if handler.is_null() {
        return (false, String::new());
    }

    let request_alloc: *mut AnyObject = msg_send![request_class, alloc];
    let request: *mut AnyObject = msg_send![request_alloc, init];
    if request.is_null() {
        return (false, String::new());
    }
    // VNRequestTextRecognitionLevelAccurate = 0
    let _: () = msg_send![request, setRecognitionLevel: 0u64];
    let _: () = msg_send![request, setUsesLanguageCorrection: true];

    let requests: *mut AnyObject = msg_send![array_class, arrayWithObject: request];
    let mut err: *mut AnyObject = ptr::null_mut();
    let ok: bool = msg_send![handler, performRequests: requests, error: &mut err];
    if !ok {
        return (false, String::new());
    }

    let observations: *mut AnyObject = msg_send![request, results];
    let mut lines: Vec<String> = Vec::new();
    if !observations.is_null() {
        let count: usize = msg_send![observations, count];
        for index in 0..count {
            let obs: *mut AnyObject = msg_send![observations, objectAtIndex: index];
            if obs.is_null() {
                continue;
            }
            let candidates: *mut AnyObject = msg_send![obs, topCandidates: 1usize];
            if candidates.is_null() {
                continue;
            }
            let first: *mut AnyObject = msg_send![candidates, firstObject];
            if first.is_null() {
                continue;
            }
            let string: *mut AnyObject = msg_send![first, string];
            let line = nsstring_to_rust(string);
            if !line.is_empty() {
                lines.push(line);
            }
        }
    }

    let text = lines.join("\n");
    (!text.is_empty(), text)
}
