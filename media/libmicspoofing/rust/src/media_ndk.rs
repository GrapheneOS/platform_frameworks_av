use crate::media_ndk_sys::{
    self, AMediaCodec, AMediaCodecBufferInfo, AMediaExtractor, AMediaFormat,
    AMEDIACODEC_BUFFER_FLAG_END_OF_STREAM, AMEDIACODEC_INFO_OUTPUT_BUFFERS_CHANGED,
    AMEDIACODEC_INFO_OUTPUT_FORMAT_CHANGED, AMEDIACODEC_INFO_TRY_AGAIN_LATER, AMEDIA_OK,
    AMEDIAEXTRACTOR_SAMPLE_FLAG_ENCRYPTED, AMEDIAEXTRACTOR_SEEK_PREVIOUS_SYNC, MediaStatus,
};
use std::ffi::{c_char, CStr, CString};
use std::ptr::NonNull;

pub(crate) const BUFFER_FLAG_END_OF_STREAM: u32 = AMEDIACODEC_BUFFER_FLAG_END_OF_STREAM;
pub(crate) const SAMPLE_FLAG_ENCRYPTED: u32 = AMEDIAEXTRACTOR_SAMPLE_FLAG_ENCRYPTED;

fn media_status_error(operation: &str, status: MediaStatus) -> String {
    format!("{operation} failed with media status {status}")
}

pub(crate) struct MediaFormat {
    raw: NonNull<AMediaFormat>,
}

impl MediaFormat {
    fn from_raw(raw: *mut AMediaFormat, operation: &str) -> Result<Self, String> {
        let raw = NonNull::new(raw).ok_or_else(|| format!("{operation} returned null"))?;
        Ok(Self { raw })
    }

    pub(crate) fn as_ptr(&self) -> *const AMediaFormat {
        self.raw.as_ptr()
    }

    pub(crate) fn description(&self) -> String {
        // SAFETY: `self.raw` is a live AMediaFormat owned by this wrapper
        let ptr = unsafe { media_ndk_sys::AMediaFormat_toString(self.raw.as_ptr()) };
        if ptr.is_null() {
            "<null media format>".to_string()
        } else {
            // SAFETY: `AMediaFormat_toString` returns a NUL-terminated string owned by the format
            unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
        }
    }

    pub(crate) fn get_i32(&self, key: *const c_char) -> Option<i32> {
        let mut value = 0;
        // SAFETY: `self.raw` is valid and `value` points to writable storage
        let ok = unsafe { media_ndk_sys::AMediaFormat_getInt32(self.raw.as_ptr(), key, &mut value) };
        ok.then_some(value)
    }

    pub(crate) fn get_string(&self, key: *const c_char) -> Option<String> {
        let mut value = std::ptr::null();
        // SAFETY: `self.raw` is valid and `value` points to writable storage
        let ok = unsafe { media_ndk_sys::AMediaFormat_getString(self.raw.as_ptr(), key, &mut value) };
        if !ok || value.is_null() {
            return None;
        }

        // SAFETY: `AMediaFormat_getString` returns a NUL-terminated string owned by the format
        Some(unsafe { CStr::from_ptr(value) }.to_string_lossy().into_owned())
    }
}

impl Drop for MediaFormat {
    fn drop(&mut self) {
        // SAFETY: `self.raw` is owned by this wrapper and deleted exactly once here
        unsafe {
            let _ = media_ndk_sys::AMediaFormat_delete(self.raw.as_ptr());
        }
    }
}

pub(crate) struct MediaExtractor {
    raw: NonNull<AMediaExtractor>,
}

// SAFETY: extractor handles are moved into a dedicated decoder thread and never shared concurrently
unsafe impl Send for MediaExtractor {}

impl MediaExtractor {
    pub(crate) fn new() -> Result<Self, String> {
        // SAFETY: direct constructor call into libmediandk
        let raw = unsafe { media_ndk_sys::AMediaExtractor_new() };
        let raw = NonNull::new(raw).ok_or_else(|| "AMediaExtractor_new returned null".to_string())?;
        Ok(Self { raw })
    }

    pub(crate) fn set_data_source_fd(&mut self, fd: i32, offset: i64, length: i64) -> Result<(), String> {
        // SAFETY: `self.raw` is valid and the fd/offset/length are plain values
        let status = unsafe {
            media_ndk_sys::AMediaExtractor_setDataSourceFd(self.raw.as_ptr(), fd, offset, length)
        };
        if status == AMEDIA_OK {
            Ok(())
        } else {
            Err(media_status_error("AMediaExtractor_setDataSourceFd", status))
        }
    }

    pub(crate) fn track_count(&self) -> usize {
        // SAFETY: `self.raw` is valid
        unsafe { media_ndk_sys::AMediaExtractor_getTrackCount(self.raw.as_ptr()) }
    }

    pub(crate) fn track_format(&self, index: usize) -> Result<MediaFormat, String> {
        // SAFETY: `self.raw` is valid and `index` is forwarded to the NDK extractor
        let raw = unsafe { media_ndk_sys::AMediaExtractor_getTrackFormat(self.raw.as_ptr(), index) };
        MediaFormat::from_raw(raw, "AMediaExtractor_getTrackFormat")
    }

    pub(crate) fn file_format(&self) -> Option<MediaFormat> {
        // SAFETY: `self.raw` is valid
        let raw = unsafe { media_ndk_sys::AMediaExtractor_getFileFormat(self.raw.as_ptr()) };
        MediaFormat::from_raw(raw, "AMediaExtractor_getFileFormat").ok()
    }

    pub(crate) fn select_track(&mut self, index: usize) -> Result<(), String> {
        // SAFETY: `self.raw` is valid and `index` is forwarded to the extractor
        let status = unsafe { media_ndk_sys::AMediaExtractor_selectTrack(self.raw.as_ptr(), index) };
        if status == AMEDIA_OK {
            Ok(())
        } else {
            Err(media_status_error("AMediaExtractor_selectTrack", status))
        }
    }

    pub(crate) fn sample_size(&self) -> isize {
        // SAFETY: `self.raw` is valid
        unsafe { media_ndk_sys::AMediaExtractor_getSampleSize(self.raw.as_ptr()) }
    }

    pub(crate) fn read_sample_data(&mut self, buffer: &mut [u8]) -> Result<usize, String> {
        // SAFETY: `self.raw` is valid and `buffer` points to writable storage of `buffer.len()`
        let size = unsafe {
            media_ndk_sys::AMediaExtractor_readSampleData(
                self.raw.as_ptr(),
                buffer.as_mut_ptr(),
                buffer.len(),
            )
        };
        if size < 0 {
            Err(format!("AMediaExtractor_readSampleData returned {size}"))
        } else {
            Ok(size as usize)
        }
    }

    pub(crate) fn sample_time_us(&self) -> i64 {
        // SAFETY: `self.raw` is valid
        unsafe { media_ndk_sys::AMediaExtractor_getSampleTime(self.raw.as_ptr()) }
    }

    pub(crate) fn sample_flags(&self) -> u32 {
        // SAFETY: `self.raw` is valid
        unsafe { media_ndk_sys::AMediaExtractor_getSampleFlags(self.raw.as_ptr()) }
    }

    pub(crate) fn advance(&mut self) -> bool {
        // SAFETY: `self.raw` is valid
        unsafe { media_ndk_sys::AMediaExtractor_advance(self.raw.as_ptr()) }
    }

    pub(crate) fn seek_to_start(&mut self) -> Result<(), String> {
        // SAFETY: `self.raw` is valid and the seek mode matches the NDK enum
        let status = unsafe {
            media_ndk_sys::AMediaExtractor_seekTo(
                self.raw.as_ptr(),
                0,
                AMEDIAEXTRACTOR_SEEK_PREVIOUS_SYNC,
            )
        };
        if status == AMEDIA_OK {
            Ok(())
        } else {
            Err(media_status_error("AMediaExtractor_seekTo", status))
        }
    }
}

impl Drop for MediaExtractor {
    fn drop(&mut self) {
        // SAFETY: `self.raw` is owned by this wrapper and deleted exactly once here
        unsafe {
            let _ = media_ndk_sys::AMediaExtractor_delete(self.raw.as_ptr());
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BufferInfo {
    pub(crate) offset: i32,
    pub(crate) size: i32,
    pub(crate) flags: u32,
}

impl From<AMediaCodecBufferInfo> for BufferInfo {
    fn from(value: AMediaCodecBufferInfo) -> Self {
        Self {
            offset: value.offset,
            size: value.size,
            flags: value.flags,
        }
    }
}

fn validate_output_buffer_range(offset: usize, size: usize, out_size: usize) -> Result<(), String> {
    let end = offset.checked_add(size).ok_or_else(|| {
        format!("output buffer range overflow: offset={offset} size={size} out_size={out_size}")
    })?;
    if end > out_size {
        return Err(format!(
            "invalid output buffer bounds: offset={offset} size={size} out_size={out_size}"
        ));
    }

    Ok(())
}

fn validate_output_buffer_bounds(info: &BufferInfo, out_size: usize) -> Result<(usize, usize), String> {
    if info.offset < 0 || info.size < 0 {
        return Err(format!("invalid output buffer bounds: {info:?}"));
    }

    let offset = info.offset as usize;
    let size = info.size as usize;
    validate_output_buffer_range(offset, size, out_size)?;
    Ok((offset, size))
}

pub(crate) enum OutputBufferResult {
    TryAgainLater,
    OutputBuffersChanged,
    OutputFormatChanged,
    Buffer { index: usize, info: BufferInfo },
}

pub(crate) struct MediaCodec {
    raw: NonNull<AMediaCodec>,
    started: bool,
}

// SAFETY: codec handles are moved into a dedicated decoder thread and never shared concurrently
unsafe impl Send for MediaCodec {}

impl MediaCodec {
    pub(crate) fn create_decoder_by_type(mime: &str) -> Result<Self, String> {
        let mime = CString::new(mime).map_err(|_| format!("invalid MIME type: {mime:?}"))?;
        // SAFETY: `mime` is a valid C string for the duration of this call
        let raw = unsafe { media_ndk_sys::AMediaCodec_createDecoderByType(mime.as_ptr()) };
        let raw =
            NonNull::new(raw).ok_or_else(|| "AMediaCodec_createDecoderByType returned null".to_string())?;
        Ok(Self { raw, started: false })
    }

    pub(crate) fn configure(&mut self, format: &MediaFormat) -> Result<(), String> {
        // SAFETY: `self.raw` and `format` are valid live NDK objects
        let status = unsafe {
            media_ndk_sys::AMediaCodec_configure(
                self.raw.as_ptr(),
                format.as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        };
        if status == AMEDIA_OK {
            Ok(())
        } else {
            Err(media_status_error("AMediaCodec_configure", status))
        }
    }

    pub(crate) fn start(&mut self) -> Result<(), String> {
        // SAFETY: `self.raw` is valid
        let status = unsafe { media_ndk_sys::AMediaCodec_start(self.raw.as_ptr()) };
        if status == AMEDIA_OK {
            self.started = true;
            Ok(())
        } else {
            Err(media_status_error("AMediaCodec_start", status))
        }
    }

    pub(crate) fn flush(&mut self) -> Result<(), String> {
        // SAFETY: `self.raw` is valid
        let status = unsafe { media_ndk_sys::AMediaCodec_flush(self.raw.as_ptr()) };
        if status == AMEDIA_OK {
            Ok(())
        } else {
            Err(media_status_error("AMediaCodec_flush", status))
        }
    }

    pub(crate) fn dequeue_input_buffer(&mut self, timeout_us: i64) -> Result<Option<usize>, String> {
        // SAFETY: `self.raw` is valid
        let index = unsafe { media_ndk_sys::AMediaCodec_dequeueInputBuffer(self.raw.as_ptr(), timeout_us) };
        if index >= 0 {
            Ok(Some(index as usize))
        } else if index == AMEDIACODEC_INFO_TRY_AGAIN_LATER {
            Ok(None)
        } else {
            Err(format!("AMediaCodec_dequeueInputBuffer returned {index}"))
        }
    }

    pub(crate) fn input_buffer(&mut self, index: usize) -> Result<&mut [u8], String> {
        let mut size = 0usize;
        // SAFETY: `self.raw` is valid and `size` points to writable storage
        let ptr = unsafe { media_ndk_sys::AMediaCodec_getInputBuffer(self.raw.as_ptr(), index, &mut size) };
        if ptr.is_null() {
            return Err(format!("AMediaCodec_getInputBuffer returned null for index {index}"));
        }

        // SAFETY: the buffer is owned by the codec and valid until it is queued back
        Ok(unsafe { std::slice::from_raw_parts_mut(ptr, size) })
    }

    pub(crate) fn queue_input_buffer(
        &mut self,
        index: usize,
        size: usize,
        presentation_time_us: i64,
        flags: u32,
    ) -> Result<(), String> {
        // SAFETY: `self.raw` is valid and `index` references a dequeued input buffer
        let status = unsafe {
            media_ndk_sys::AMediaCodec_queueInputBuffer(
                self.raw.as_ptr(),
                index,
                0,
                size,
                presentation_time_us,
                flags,
            )
        };
        if status == AMEDIA_OK {
            Ok(())
        } else {
            Err(media_status_error("AMediaCodec_queueInputBuffer", status))
        }
    }

    pub(crate) fn dequeue_output_buffer(
        &mut self,
        timeout_us: i64,
    ) -> Result<OutputBufferResult, String> {
        let mut info = AMediaCodecBufferInfo::default();
        // SAFETY: `self.raw` is valid and `info` points to writable storage
        let index = unsafe {
            media_ndk_sys::AMediaCodec_dequeueOutputBuffer(self.raw.as_ptr(), &mut info, timeout_us)
        };
        match index {
            i if i >= 0 => Ok(OutputBufferResult::Buffer {
                index: i as usize,
                info: info.into(),
            }),
            AMEDIACODEC_INFO_TRY_AGAIN_LATER => Ok(OutputBufferResult::TryAgainLater),
            AMEDIACODEC_INFO_OUTPUT_BUFFERS_CHANGED => Ok(OutputBufferResult::OutputBuffersChanged),
            AMEDIACODEC_INFO_OUTPUT_FORMAT_CHANGED => Ok(OutputBufferResult::OutputFormatChanged),
            other => Err(format!("AMediaCodec_dequeueOutputBuffer returned {other}")),
        }
    }

    pub(crate) fn output_buffer<'a>(
        &'a mut self,
        index: usize,
        info: &BufferInfo,
    ) -> Result<&'a [u8], String> {
        let mut out_size = 0usize;
        // SAFETY: `self.raw` is valid and `out_size` points to writable storage
        let ptr = unsafe { media_ndk_sys::AMediaCodec_getOutputBuffer(self.raw.as_ptr(), index, &mut out_size) };
        if ptr.is_null() {
            return Err(format!("AMediaCodec_getOutputBuffer returned null for index {index}"));
        }

        let (offset, size) = validate_output_buffer_bounds(info, out_size)?;

        // SAFETY: the range is validated above to fit inside the codec-reported output buffer
        Ok(unsafe { std::slice::from_raw_parts(ptr.add(offset), size) })
    }

    pub(crate) fn output_format(&mut self) -> Result<MediaFormat, String> {
        // SAFETY: `self.raw` is valid
        let raw = unsafe { media_ndk_sys::AMediaCodec_getOutputFormat(self.raw.as_ptr()) };
        MediaFormat::from_raw(raw, "AMediaCodec_getOutputFormat")
    }

    pub(crate) fn release_output_buffer(&mut self, index: usize) -> Result<(), String> {
        // SAFETY: `self.raw` is valid and `index` refers to a dequeued output buffer
        let status =
            unsafe { media_ndk_sys::AMediaCodec_releaseOutputBuffer(self.raw.as_ptr(), index, false) };
        if status == AMEDIA_OK {
            Ok(())
        } else {
            Err(media_status_error("AMediaCodec_releaseOutputBuffer", status))
        }
    }

    fn stop_if_started(&mut self) {
        if !self.started {
            return;
        }

        // SAFETY: `self.raw` is valid and stopping during cleanup is always best-effort
        unsafe {
            let _ = media_ndk_sys::AMediaCodec_stop(self.raw.as_ptr());
        }
        self.started = false;
    }
}

impl Drop for MediaCodec {
    fn drop(&mut self) {
        self.stop_if_started();
        // SAFETY: `self.raw` is owned by this wrapper and deleted exactly once here
        unsafe {
            let _ = media_ndk_sys::AMediaCodec_delete(self.raw.as_ptr());
        }
    }
}

pub(crate) fn key_channel_count() -> *const c_char {
    // SAFETY: libmediandk exports this static C string key for the process lifetime
    unsafe { media_ndk_sys::AMEDIAFORMAT_KEY_CHANNEL_COUNT }
}

pub(crate) fn key_mime() -> *const c_char {
    // SAFETY: libmediandk exports this static C string key for the process lifetime
    unsafe { media_ndk_sys::AMEDIAFORMAT_KEY_MIME }
}

pub(crate) fn key_pcm_encoding() -> *const c_char {
    // SAFETY: libmediandk exports this static C string key for the process lifetime
    unsafe { media_ndk_sys::AMEDIAFORMAT_KEY_PCM_ENCODING }
}

pub(crate) fn key_sample_rate() -> *const c_char {
    // SAFETY: libmediandk exports this static C string key for the process lifetime
    unsafe { media_ndk_sys::AMEDIAFORMAT_KEY_SAMPLE_RATE }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_output_buffer_bounds_accepts_in_range_slice() {
        let info = BufferInfo {
            offset: 4,
            size: 8,
            flags: 0,
        };

        assert_eq!(validate_output_buffer_bounds(&info, 16).unwrap(), (4, 8));
    }

    #[test]
    fn validate_output_buffer_bounds_rejects_negative_values() {
        let info = BufferInfo {
            offset: -1,
            size: 8,
            flags: 0,
        };

        assert!(validate_output_buffer_bounds(&info, 16).is_err());
    }

    #[test]
    fn validate_output_buffer_bounds_rejects_out_of_range_slice() {
        let info = BufferInfo {
            offset: 12,
            size: 8,
            flags: 0,
        };

        assert!(validate_output_buffer_bounds(&info, 16).is_err());
    }

    #[test]
    fn validate_output_buffer_range_rejects_overflow() {
        assert!(validate_output_buffer_range(usize::MAX, 1, usize::MAX).is_err());
    }
}
