//! Streaming decoder entry points for mic spoofing

mod decoder;
mod logging;
mod media_ndk;
mod media_ndk_sys;

use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};

use logging::{android_log, ANDROID_LOG_ERROR};

#[no_mangle]
/// # Safety
/// `out_sample_rate` and `out_channel_count` must be valid writable pointers
pub unsafe extern "C" fn mic_spoofing_decoder_start(
    source_fd: i32,
    out_sample_rate: *mut u32,
    out_channel_count: *mut u32,
) -> i32 {
    let owned_fd = if source_fd >= 0 {
        // SAFETY: the caller transfers ownership of `source_fd`
        Some(unsafe { OwnedFd::from_raw_fd(source_fd) })
    } else {
        None
    };

    if owned_fd.is_none() || out_sample_rate.is_null() || out_channel_count.is_null() {
        android_log(
            ANDROID_LOG_ERROR,
            "mic_spoofing_decoder_start: invalid source fd or output pointers",
        );
        return -1;
    }

    // SAFETY: the pointers are validated above and point to caller-owned writable storage
    unsafe {
        *out_sample_rate = 0;
        *out_channel_count = 0;
    }

    match decoder::start_decoder_thread(owned_fd.unwrap()) {
        Ok((pipe_fd, sample_rate, channel_count)) => {
            // SAFETY: the pointers are still valid for the duration of this call
            unsafe {
                *out_sample_rate = sample_rate;
                *out_channel_count = channel_count;
            }

            pipe_fd.into_raw_fd()
        }
        Err(e) => {
            android_log(
                ANDROID_LOG_ERROR,
                &format!("mic_spoofing_decoder_start failed: {e}"),
            );

            -1
        }
    }
}

#[cfg(test)]
mod decoder_tests;
