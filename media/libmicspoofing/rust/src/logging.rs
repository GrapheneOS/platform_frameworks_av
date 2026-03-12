use std::ffi::{c_char, CString};

pub(crate) const ANDROID_LOG_DEBUG: i32 = 3;
pub(crate) const ANDROID_LOG_ERROR: i32 = 6;

const TAG: &[u8] = b"MicSpoofing\0";
const DEBUG_LOGGING: bool = false;

unsafe extern "C" {
    fn __android_log_write(priority: i32, tag: *const c_char, text: *const c_char) -> i32;
}

pub(crate) fn android_log(priority: i32, msg: &str) {
    if !DEBUG_LOGGING && priority < ANDROID_LOG_ERROR {
        return;
    }

    if let Ok(c_msg) = CString::new(msg) {
        // SAFETY: `TAG` and `c_msg` are valid NUL-terminated C strings that live for the
        // duration of this call
        unsafe {
            __android_log_write(priority, TAG.as_ptr().cast(), c_msg.as_ptr());
        }
    }
}
