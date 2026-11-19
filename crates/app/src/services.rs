//! macOS Services provider behind Finder's "Analyze Space with diskmap" (NSServices in Info.plist,
//! written by `script/bundle-macos.sh`). The selector must match the plist's `NSMessage`.
#![allow(unexpected_cfgs)] // objc 0.2 macros check a `cargo-clippy` feature

use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel, YES};
use objc::{class, msg_send, sel, sel_impl};
use std::ffi::{CStr, c_void};
use std::os::raw::c_char;
use std::path::PathBuf;

type Id = *mut Object;

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    static NSPasteboardURLReadingFileURLsOnlyKey: Id;
    fn NSUpdateDynamicServices();
}

extern "C" fn analyze_folder(_: &Object, _: Sel, pboard: Id, _user_data: Id, _error: *mut c_void) {
    // SAFETY: AppKit hands us a live NSPasteboard on the main thread; every object read here is
    // autoreleased by AppKit and only borrowed.
    unsafe {
        let url_class: &Class = class!(NSURL);
        let classes: Id = msg_send![class!(NSArray), arrayWithObject: url_class];
        let yes: Id = msg_send![class!(NSNumber), numberWithBool: YES];
        let options: Id = msg_send![class!(NSDictionary), dictionaryWithObject: yes forKey: NSPasteboardURLReadingFileURLsOnlyKey];
        let urls: Id = msg_send![pboard, readObjectsForClasses: classes options: options];
        if urls.is_null() {
            return;
        }
        let count: usize = msg_send![urls, count];
        for i in 0..count {
            let url: Id = msg_send![urls, objectAtIndex: i];
            let path: Id = msg_send![url, path];
            let utf8: *const c_char = if path.is_null() { std::ptr::null() } else { msg_send![path, UTF8String] };
            if !utf8.is_null() {
                crate::inbox::send(PathBuf::from(CStr::from_ptr(utf8).to_string_lossy().into_owned()));
                return;
            }
        }
    }
}

/// Call once AppKit has finished launching (inside `Application::run`).
pub fn register() {
    let Some(mut decl) = ClassDecl::new("DiskmapServicesProvider", class!(NSObject)) else { return };
    // SAFETY: the method signature matches the Services convention `-(void)msg:(NSPasteboard*)
    // userData:(NSString*) error:(NSString**)`; the provider is retained by us forever, as
    // NSApplication does not retain its services provider.
    unsafe {
        decl.add_method(sel!(analyzeFolder:userData:error:), analyze_folder as extern "C" fn(&Object, Sel, Id, Id, *mut c_void));
        let cls = decl.register();
        let provider: Id = msg_send![cls, new];
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, setServicesProvider: provider];
        NSUpdateDynamicServices();
    }
}
