#![allow(missing_docs)]
#![no_main]

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use std::ffi::c_void;

unsafe extern "C" {
    fn getuid() -> u32;
}

// Native audio PCM sub-format values (must match system/audio-hal-enums.h)
const AUDIO_FORMAT_PCM_16_BIT: u32 = 0x1;
const AUDIO_FORMAT_PCM_8_BIT: u32 = 0x2;
const AUDIO_FORMAT_PCM_32_BIT: u32 = 0x3;
const AUDIO_FORMAT_PCM_FLOAT: u32 = 0x5;
const AUDIO_FORMAT_PCM_24_BIT_PACKED: u32 = 0x6;

/// Maximum buffer allocation per fuzz iteration (256 KB)
const MAX_ALLOC: usize = 256 * 1024;

/// Returns bytes-per-sample for an audio format, matching lib.rs `bytes_per_sample`
fn bytes_per_sample(fmt: u32) -> usize {
    match fmt {
        AUDIO_FORMAT_PCM_8_BIT => 1,
        AUDIO_FORMAT_PCM_16_BIT => 2,
        AUDIO_FORMAT_PCM_24_BIT_PACKED => 3,
        AUDIO_FORMAT_PCM_32_BIT | AUDIO_FORMAT_PCM_FLOAT => 4,
        _ => 2, // default: matches lib.rs
    }
}

/// Audio format selection: all valid formats plus arbitrary raw values
#[derive(Arbitrary, Debug)]
enum Format {
    Pcm16,
    Pcm8,
    Pcm32,
    Float,
    Pcm24,
    /// Arbitrary u32 — exercises unknown-format / silence-fallback path
    Raw(u32),
}

impl Format {
    fn val(&self) -> u32 {
        match self {
            Self::Pcm16 => AUDIO_FORMAT_PCM_16_BIT,
            Self::Pcm8 => AUDIO_FORMAT_PCM_8_BIT,
            Self::Pcm32 => AUDIO_FORMAT_PCM_32_BIT,
            Self::Float => AUDIO_FORMAT_PCM_FLOAT,
            Self::Pcm24 => AUDIO_FORMAT_PCM_24_BIT_PACKED,
            Self::Raw(v) => *v,
        }
    }
}

#[derive(Arbitrary, Debug)]
enum Frames {
    Zero,
    One,
    Small(u8),
    Medium(u16),
    /// `frame_count * 2` overflows `usize` (tests checked_mul in read_samples)
    OverflowMul,
    /// `sample_count * 3` overflows `usize` (tests 24-bit packed byte-count guard)
    Overflow24,
}

/// Buffer alignment relative to an 8-byte boundary
#[derive(Arbitrary, Debug)]
enum Align {
    /// 8-byte aligned — valid for all formats
    Natural,
    /// +1 byte — misaligned for i16, f32, i32
    Off1,
    /// +2 bytes — aligned for i16, misaligned for f32/i32
    Off2,
    /// +3 bytes — misaligned for i16, f32, i32
    Off3,
}

/// Structured fuzzer input, derived from raw bytes via `Arbitrary`
#[derive(Arbitrary, Debug)]
struct Input {
    null_source: bool,
    null_buffer: bool,
    align: Align,
    frames: Frames,
    sample_rate: u32,
    channel_count: u32,
    format: Format,
    test_lifecycle: bool,
}

/// Returns a source pointer, created once per process. May be null if the
/// WAV file (`/system/etc/spoofed_mic_audio_default.wav`) is not installed
fn cached_source() -> *mut c_void {
    use std::sync::OnceLock;

    struct Ptr(*mut c_void);
    // SAFETY: libFuzzer is single-threaded; no concurrent access to the source
    unsafe impl Send for Ptr {}
    unsafe impl Sync for Ptr {}

    static S: OnceLock<Ptr> = OnceLock::new();
    // SAFETY: `getuid` is process-local and takes no arguments
    let uid = unsafe { getuid() as i32 };
    S.get_or_init(|| Ptr(micspoofing::mic_spoofing_create_source(uid))).0
}

fuzz_target!(|input: Input| {
    if input.test_lifecycle {
        // SAFETY: `getuid` is process-local and takes no arguments
        let uid = unsafe { getuid() as i32 };
        unsafe {
            let s = micspoofing::mic_spoofing_create_source(uid);
            micspoofing::mic_spoofing_destroy_source(s);
            // Null destroy must be a no-op
            micspoofing::mic_spoofing_destroy_source(std::ptr::null_mut());
        }
    }

    let source = if input.null_source {
        std::ptr::null_mut()
    } else {
        cached_source()
    };

    let fmt = input.format.val();
    let bps = bytes_per_sample(fmt);

    let frame_count: usize = match input.frames {
        Frames::Zero => 0,
        Frames::One => 1,
        Frames::Small(n) => n as usize,
        Frames::Medium(n) => n as usize,
        Frames::OverflowMul => usize::MAX / 2 + 1,
        Frames::Overflow24 => usize::MAX / 6 + 1,
    };

    let needed = frame_count
        .checked_mul(input.channel_count as usize)
        .and_then(|sc| sc.checked_mul(bps));

    let (fc, buf_len) = match needed {
        None => (frame_count, 64usize),
        Some(n) if n <= MAX_ALLOC => (frame_count, n.max(1)),
        Some(_) => {
            let ch = (input.channel_count as usize).max(1);
            let cap = MAX_ALLOC / (ch * bps);
            (cap, (cap * ch * bps).max(1))
        }
    };

    // Allocate with padding for alignment offset (up to 7 + 3 = 10 bytes)
    // Fill with 0xCD (uninitialized marker) to help detect partial writes
    let mut storage = vec![0xCDu8; buf_len + 16];

    let buffer = if input.null_buffer {
        std::ptr::null_mut()
    } else {
        let base = storage.as_mut_ptr();
        // Align base to 8-byte boundary, then apply the requested offset
        let aligned = ((base as usize + 7) & !7) as *mut u8;
        let off = match input.align {
            Align::Natural => 0usize,
            Align::Off1 => 1,
            Align::Off2 => 2,
            Align::Off3 => 3,
        };
        // SAFETY: aligned + off is within the storage Vec's allocation
        // (at most 10 bytes from base, storage has 16 bytes of padding)
        unsafe { aligned.add(off) }
    };

    let _result = unsafe {
        micspoofing::mic_spoofing_read_samples(
            source,
            buffer,
            fc,
            input.sample_rate,
            input.channel_count,
            fmt,
        )
    };
});
