use std::ffi::{c_char, c_void};

pub enum AMediaCodec {}
pub enum AMediaExtractor {}
pub enum AMediaFormat {}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct AMediaCodecBufferInfo {
    pub offset: i32,
    pub size: i32,
    pub presentation_time_us: i64,
    pub flags: u32,
}

pub type MediaStatus = i32;

pub const AMEDIA_OK: MediaStatus = 0;
pub const AMEDIACODEC_BUFFER_FLAG_END_OF_STREAM: u32 = 4;
pub const AMEDIACODEC_INFO_OUTPUT_BUFFERS_CHANGED: isize = -3;
pub const AMEDIACODEC_INFO_OUTPUT_FORMAT_CHANGED: isize = -2;
pub const AMEDIACODEC_INFO_TRY_AGAIN_LATER: isize = -1;
pub const AMEDIAEXTRACTOR_SAMPLE_FLAG_ENCRYPTED: u32 = 2;
pub const AMEDIAEXTRACTOR_SEEK_PREVIOUS_SYNC: i32 = 0;

unsafe extern "C" {
    pub fn AMediaExtractor_new() -> *mut AMediaExtractor;
    pub fn AMediaExtractor_delete(extractor: *mut AMediaExtractor) -> MediaStatus;
    pub fn AMediaExtractor_setDataSourceFd(
        extractor: *mut AMediaExtractor,
        fd: i32,
        offset: i64,
        length: i64,
    ) -> MediaStatus;
    pub fn AMediaExtractor_getTrackCount(extractor: *mut AMediaExtractor) -> usize;
    pub fn AMediaExtractor_getTrackFormat(
        extractor: *mut AMediaExtractor,
        index: usize,
    ) -> *mut AMediaFormat;
    pub fn AMediaExtractor_getFileFormat(extractor: *mut AMediaExtractor) -> *mut AMediaFormat;
    pub fn AMediaExtractor_selectTrack(
        extractor: *mut AMediaExtractor,
        index: usize,
    ) -> MediaStatus;
    pub fn AMediaExtractor_getSampleSize(extractor: *mut AMediaExtractor) -> isize;
    pub fn AMediaExtractor_readSampleData(
        extractor: *mut AMediaExtractor,
        buffer: *mut u8,
        capacity: usize,
    ) -> isize;
    pub fn AMediaExtractor_getSampleTime(extractor: *mut AMediaExtractor) -> i64;
    pub fn AMediaExtractor_getSampleFlags(extractor: *mut AMediaExtractor) -> u32;
    pub fn AMediaExtractor_advance(extractor: *mut AMediaExtractor) -> bool;
    pub fn AMediaExtractor_seekTo(
        extractor: *mut AMediaExtractor,
        seek_pos_us: i64,
        mode: i32,
    ) -> MediaStatus;

    pub fn AMediaCodec_createDecoderByType(mime_type: *const c_char) -> *mut AMediaCodec;
    pub fn AMediaCodec_delete(codec: *mut AMediaCodec) -> MediaStatus;
    pub fn AMediaCodec_configure(
        codec: *mut AMediaCodec,
        format: *const AMediaFormat,
        surface: *mut c_void,
        crypto: *mut c_void,
        flags: u32,
    ) -> MediaStatus;
    pub fn AMediaCodec_start(codec: *mut AMediaCodec) -> MediaStatus;
    pub fn AMediaCodec_stop(codec: *mut AMediaCodec) -> MediaStatus;
    pub fn AMediaCodec_flush(codec: *mut AMediaCodec) -> MediaStatus;
    pub fn AMediaCodec_dequeueInputBuffer(codec: *mut AMediaCodec, timeout_us: i64) -> isize;
    pub fn AMediaCodec_getInputBuffer(
        codec: *mut AMediaCodec,
        index: usize,
        out_size: *mut usize,
    ) -> *mut u8;
    pub fn AMediaCodec_queueInputBuffer(
        codec: *mut AMediaCodec,
        index: usize,
        offset: usize,
        size: usize,
        presentation_time_us: i64,
        flags: u32,
    ) -> MediaStatus;
    pub fn AMediaCodec_dequeueOutputBuffer(
        codec: *mut AMediaCodec,
        info: *mut AMediaCodecBufferInfo,
        timeout_us: i64,
    ) -> isize;
    pub fn AMediaCodec_getOutputBuffer(
        codec: *mut AMediaCodec,
        index: usize,
        out_size: *mut usize,
    ) -> *mut u8;
    pub fn AMediaCodec_getOutputFormat(codec: *mut AMediaCodec) -> *mut AMediaFormat;
    pub fn AMediaCodec_releaseOutputBuffer(
        codec: *mut AMediaCodec,
        index: usize,
        render: bool,
    ) -> MediaStatus;

    pub fn AMediaFormat_delete(format: *mut AMediaFormat) -> MediaStatus;
    pub fn AMediaFormat_toString(format: *mut AMediaFormat) -> *const c_char;
    pub fn AMediaFormat_getInt32(
        format: *mut AMediaFormat,
        name: *const c_char,
        out: *mut i32,
    ) -> bool;
    pub fn AMediaFormat_getString(
        format: *mut AMediaFormat,
        name: *const c_char,
        out: *mut *const c_char,
    ) -> bool;

    pub static AMEDIAFORMAT_KEY_CHANNEL_COUNT: *const c_char;
    pub static AMEDIAFORMAT_KEY_MIME: *const c_char;
    pub static AMEDIAFORMAT_KEY_PCM_ENCODING: *const c_char;
    pub static AMEDIAFORMAT_KEY_SAMPLE_RATE: *const c_char;
}
