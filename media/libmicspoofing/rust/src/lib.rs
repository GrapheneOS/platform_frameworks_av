//! Microphone spoofing runtime support for media capture clients

use std::ffi::{c_char, CStr};
use std::fs::File;
use std::io::Read;
use std::os::fd::{FromRawFd, OwnedFd};

use jni::sys::{jsize, JNI_GetCreatedJavaVMs, JNI_OK};
use jni::{JNIEnv, JavaVM};
use packagemanager_aidl::aidl::android::content::pm::IPackageManagerNative::IPackageManagerNative;

const SPOOFED_AUDIO_WAV_PATH: &str = "/system/etc/spoofed_mic_audio_default.wav";
const PROP_VALUE_MAX: usize = 92;
const DEBUGGABLE_PROP: &[u8] = b"ro.debuggable\0";
const TEST_DEFAULT_WAV_PATH_PROP: &[u8] = b"debug.mic_spoofing.default_audio_path\0";
const TEST_DEFAULT_WAV_PATH_PREFIX: &str = "/data/local/tmp/";

// ~30 seconds of 48 kHz stereo = ~11 MB as f32. Prevents audioserver OOM from oversized WAV.
const MAX_WAV_SAMPLES: u32 = 2_880_000;

const TAG: &[u8] = b"MicSpoofing\0";
const DEBUG_LOGGING: bool = false;
const ANDROID_LOG_DEBUG: i32 = 3;
const ANDROID_LOG_ERROR: i32 = 6;

// Float-to-PCM conversion constants, matching clamp*_from_float behavior.
const PCM_I16_MAX: f32 = 32_767.0;
const PCM_I16_MIN: f32 = -32_768.0;
const PCM_I24_MAX: f32 = 8_388_607.0;
const PCM_I24_MIN: f32 = -8_388_608.0;
const PCM_I32_MAX: f32 = 2_147_483_647.0;
const PCM_I32_MIN: f32 = -2_147_483_648.0;
const PCM_U8_SCALE: f32 = 127.0;
const PCM_U8_OFFSET: f32 = 128.0;

// Native audio PCM sub-format values
const AUDIO_FORMAT_PCM_16_BIT: u32 = 0x1;
const AUDIO_FORMAT_PCM_8_BIT: u32 = 0x2;
const AUDIO_FORMAT_PCM_32_BIT: u32 = 0x3;
const AUDIO_FORMAT_PCM_FLOAT: u32 = 0x5;
const AUDIO_FORMAT_PCM_24_BIT_PACKED: u32 = 0x6;

extern "C" {
    fn __android_log_write(priority: i32, tag: *const c_char, text: *const c_char) -> i32;
    fn __system_property_get(name: *const c_char, value: *mut c_char) -> i32;
}

fn android_log(priority: i32, msg: &str) {
    if !DEBUG_LOGGING && priority < ANDROID_LOG_ERROR {
        return;
    }

    if let Ok(c_msg) = std::ffi::CString::new(msg) {
        // SAFETY: `TAG` and `c_msg` are valid NUL-terminated C strings that live
        // for the duration of this call
        unsafe {
            __android_log_write(priority, TAG.as_ptr().cast(), c_msg.as_ptr());
        }
    }
}

fn read_system_property(name: &[u8]) -> Option<String> {
    let mut buf = [0 as c_char; PROP_VALUE_MAX];
    // SAFETY: `name` is a static NUL-terminated C string key and `buf` is writable
    let len = unsafe { __system_property_get(name.as_ptr().cast(), buf.as_mut_ptr()) };
    if len <= 0 {
        return None;
    }
    // SAFETY: `__system_property_get` writes a NUL-terminated C string into `buf`
    let c_str = unsafe { CStr::from_ptr(buf.as_ptr()) };
    c_str.to_str().ok().map(ToOwned::to_owned)
}

fn resolve_default_wav_path() -> String {
    let is_debuggable = read_system_property(DEBUGGABLE_PROP).as_deref() == Some("1");
    if !is_debuggable {
        return SPOOFED_AUDIO_WAV_PATH.to_string();
    }

    let Some(path) = read_system_property(TEST_DEFAULT_WAV_PATH_PROP) else {
        return SPOOFED_AUDIO_WAV_PATH.to_string();
    };

    let trimmed = path.trim();
    if trimmed.is_empty() {
        return SPOOFED_AUDIO_WAV_PATH.to_string();
    }

    if !trimmed.starts_with(TEST_DEFAULT_WAV_PATH_PREFIX) {
        android_log(
            ANDROID_LOG_ERROR,
            "Ignoring debug.mic_spoofing.default_audio_path outside /data/local/tmp/",
        );
        return SPOOFED_AUDIO_WAV_PATH.to_string();
    }

    trimmed.to_owned()
}

fn bytes_per_sample(audio_format: u32) -> usize {
    match audio_format {
        AUDIO_FORMAT_PCM_8_BIT => 1,
        AUDIO_FORMAT_PCM_16_BIT => 2,
        AUDIO_FORMAT_PCM_24_BIT_PACKED => 3,
        AUDIO_FORMAT_PCM_32_BIT | AUDIO_FORMAT_PCM_FLOAT => 4,
        _ => 2, // default to 16-bit
    }
}

unsafe fn fill_silence(buffer: *mut u8, frame_count: usize, channel_count: u32, audio_format: u32) {
    let bps = bytes_per_sample(audio_format);
    let Some(byte_count) =
        frame_count.checked_mul(channel_count as usize).and_then(|n| n.checked_mul(bps))
    else {
        return;
    };
    let silence = if audio_format == AUDIO_FORMAT_PCM_8_BIT { 128u8 } else { 0u8 };
    // SAFETY: Caller guarantees `buffer` points to writable memory for at least
    // `byte_count` bytes. `byte_count` is overflow-checked above
    unsafe {
        std::slice::from_raw_parts_mut(buffer, byte_count).fill(silence);
    }
}

fn is_enabled_for_uid_inner(uid: i32) -> Result<bool, String> {
    let pm: binder::Strong<dyn IPackageManagerNative> =
        binder::check_interface("package_native")
            .map_err(|e| format!("cannot find package_native service: {:?}", e))?;

    pm.isMicSpoofingEnabledForUid(uid).map_err(|e| format!("binder call failed: {}", e))
}

fn get_custom_source_fd(uid: i32) -> Result<Option<File>, String> {
    let _ = uid;

    let mut vm_ptr = std::ptr::null_mut();
    let mut vm_count: jsize = 0;
    // SAFETY: `vm_ptr` and `vm_count` are valid writable out-pointers for one Java VM slot
    let status = unsafe { JNI_GetCreatedJavaVMs(&mut vm_ptr, 1, &mut vm_count) };
    if status != JNI_OK || vm_count == 0 || vm_ptr.is_null() {
        return Err(format!(
            "failed to get Java VM instance: status={} vm_count={}",
            status, vm_count
        ));
    }

    // SAFETY: `vm_ptr` came from `JNI_GetCreatedJavaVMs` for this process and is non-null
    let vm = unsafe { JavaVM::from_raw(vm_ptr) }
        .map_err(|e| format!("failed to wrap Java VM: {}", e))?;
    let mut env = vm
        .attach_current_thread()
        .map_err(|e| format!("failed to attach current thread to Java VM: {}", e))?;

    let parcel_file_descriptor = match env.call_static_method(
        "android/ext/micspoofing/MicSpoofingApi",
        "openCustomSourceFdForSelf",
        "()Landroid/os/ParcelFileDescriptor;",
        &[],
    ) {
        Ok(value) => value
            .l()
            .map_err(|e| format!("openCustomSourceFdForSelf returned invalid type: {}", e))?,
        Err(e) => {
            clear_pending_jni_exception(
                &mut env,
                "openCustomSourceFdForSelf JNI call failed",
            );
            return Err(format!("openCustomSourceFdForSelf JNI call failed: {}", e));
        }
    };

    if parcel_file_descriptor.is_null() {
        return Ok(None);
    }

    let detached_fd = match env.call_method(parcel_file_descriptor, "detachFd", "()I", &[]) {
        Ok(value) => value
            .i()
            .map_err(|e| format!("ParcelFileDescriptor.detachFd returned invalid type: {}", e))?,
        Err(e) => {
            clear_pending_jni_exception(
                &mut env,
                "ParcelFileDescriptor.detachFd JNI call failed",
            );
            return Err(format!("ParcelFileDescriptor.detachFd failed: {}", e));
        }
    };

    if detached_fd < 0 {
        return Err(format!(
            "ParcelFileDescriptor.detachFd returned invalid fd: {}",
            detached_fd
        ));
    }

    // SAFETY: `detachFd` transfers ownership of a live file descriptor to the caller once
    Ok(Some(unsafe { File::from(OwnedFd::from_raw_fd(detached_fd)) }))
}

fn clear_pending_jni_exception(env: &mut JNIEnv, context: &str) {
    match env.exception_check() {
        Ok(true) => {
            let _ = env.exception_clear();
            android_log(
                ANDROID_LOG_ERROR,
                &format!("mic_spoofing: cleared pending JNI exception after {}", context),
            );
        }
        Ok(false) => {}
        Err(e) => {
            android_log(
                ANDROID_LOG_ERROR,
                &format!(
                    "mic_spoofing: failed to check pending JNI exception after {}: {}",
                    context, e
                ),
            );
        }
    }
}

fn create_source_with_fallback<F>(
    uid: i32,
    default_wav_path: &str,
    get_custom_source_fd_fn: F,
) -> Option<SpoofedAudioSource>
where
    F: FnOnce(i32) -> Result<Option<File>, String>,
{
    match get_custom_source_fd_fn(uid) {
        Ok(Some(custom_fd)) => match SpoofedAudioSource::from_reader(custom_fd) {
            Ok(src) => {
                return Some(src);
            }
            Err(e) => {
                android_log(
                    ANDROID_LOG_ERROR,
                    &format!("mic_spoofing_create_source: custom source FD failed: {}", e),
                );
            }
        },
        Ok(None) => {
            android_log(
                ANDROID_LOG_DEBUG,
                &format!("mic_spoofing_create_source: no custom source FD for uid={}", uid),
            );
        }
        Err(e) => {
            android_log(
                ANDROID_LOG_ERROR,
                &format!("mic_spoofing_create_source: custom source binder call failed: {}", e),
            );
        }
    }

    open_default_source_with_system_fallback(default_wav_path, SPOOFED_AUDIO_WAV_PATH)
}

fn open_default_source_with_system_fallback(
    default_wav_path: &str,
    system_wav_path: &str,
) -> Option<SpoofedAudioSource> {
    match SpoofedAudioSource::new(default_wav_path) {
        Ok(src) => {
            Some(src)
        }
        Err(e) => {
            android_log(
                ANDROID_LOG_ERROR,
                &format!(
                    "mic_spoofing_create_source: failed to open default source {}: {}",
                    default_wav_path, e
                ),
            );

            if default_wav_path == system_wav_path {
                return None;
            }

            android_log(
                ANDROID_LOG_ERROR,
                "mic_spoofing_create_source: falling back to /system/etc default source",
            );
            match SpoofedAudioSource::new(system_wav_path) {
                Ok(src) => {
                    Some(src)
                }
                Err(fallback_err) => {
                    android_log(
                        ANDROID_LOG_ERROR,
                        &format!(
                            "mic_spoofing_create_source: /system/etc fallback open failed: {}",
                            fallback_err
                        ),
                    );
                    None
                }
            }
        }
    }
}

struct SpoofedAudioSource {
    samples: Vec<f32>,
    source_sample_rate: u32,
    source_channels: u16,
    position: usize,
}

impl SpoofedAudioSource {
    fn new(wav_path: &str) -> Result<Self, String> {
        let wav_file = File::open(wav_path).map_err(|e| format!("Failed to open WAV: {}", e))?;
        Self::from_reader(wav_file)
    }

    fn from_reader<R: Read>(reader: R) -> Result<Self, String> {
        let reader =
            hound::WavReader::new(reader).map_err(|e| format!("Failed to open WAV: {}", e))?;
        let spec = reader.spec();

        if spec.channels == 0 {
            return Err("Invalid WAV: channel count is 0".to_string());
        }
        if spec.sample_rate == 0 {
            return Err("Invalid WAV: sample rate is 0".to_string());
        }

        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Int => {
                if !(1..=32).contains(&spec.bits_per_sample) {
                    return Err(format!(
                        "Unsupported integer PCM bit depth: {}",
                        spec.bits_per_sample
                    ));
                }
                let max = (1u64 << (spec.bits_per_sample - 1)) as f32;
                reader
                    .into_samples::<i32>()
                    .take(MAX_WAV_SAMPLES as usize)
                    .map(|s| s.map_err(|e| format!("Failed to decode WAV sample: {}", e)))
                    .map(|s| s.map(|sample| sample as f32 / max))
                    .collect::<Result<Vec<_>, _>>()?
            }
            hound::SampleFormat::Float => {
                reader
                    .into_samples::<f32>()
                    .take(MAX_WAV_SAMPLES as usize)
                    .map(|s| s.map_err(|e| format!("Failed to decode WAV sample: {}", e)))
                    .collect::<Result<Vec<_>, _>>()?
            }
        };

        Ok(Self {
            samples,
            source_sample_rate: spec.sample_rate,
            source_channels: spec.channels,
            position: 0,
        })
    }

    fn read_f32(
        &mut self,
        output: &mut [f32],
        target_sample_rate: u32,
        target_channels: u16,
    ) -> usize {
        if target_channels == 0 {
            return 0;
        }

        let frames_out = output.len() / target_channels as usize;
        if self.samples.is_empty() || self.source_channels == 0 {
            output.fill(0.0);
            return frames_out;
        }

        let ratio = self.source_sample_rate as f64 / target_sample_rate as f64;
        let src_frames = self.samples.len() / self.source_channels as usize;
        if src_frames == 0 {
            output.fill(0.0);
            return frames_out;
        }

        for frame in 0..frames_out {
            let src_pos = (self.position as f64 + frame as f64 * ratio) as usize;
            let src_frame = src_pos % src_frames;

            for ch in 0..target_channels as usize {
                let src_ch = ch.min(self.source_channels as usize - 1);
                let idx = src_frame * self.source_channels as usize + src_ch;
                output[frame * target_channels as usize + ch] = self.samples[idx];
            }
        }

        self.position = ((self.position as f64 + frames_out as f64 * ratio) as usize) % src_frames;

        frames_out
    }

    fn read_i16(&mut self, output: &mut [i16], target_sr: u32, target_ch: u16) -> usize {
        let mut temp = vec![0.0f32; output.len()];
        let frames = self.read_f32(&mut temp, target_sr, target_ch);
        for (i, s) in temp.iter().enumerate() {
            output[i] = (*s * PCM_I16_MAX).clamp(PCM_I16_MIN, PCM_I16_MAX) as i16;
        }
        frames
    }

    fn read_u8(&mut self, output: &mut [u8], target_sr: u32, target_ch: u16) -> usize {
        let mut temp = vec![0.0f32; output.len()];
        let frames = self.read_f32(&mut temp, target_sr, target_ch);
        for (i, s) in temp.iter().enumerate() {
            // Android 8-bit PCM is unsigned: silence = 128
            output[i] = (*s * PCM_U8_SCALE + PCM_U8_OFFSET).clamp(0.0, 255.0) as u8;
        }
        frames
    }

    fn read_i24_packed(&mut self, output: &mut [u8], target_sr: u32, target_ch: u16) -> usize {
        let sample_count = output.len() / 3;
        let mut temp = vec![0.0f32; sample_count];
        let frames = self.read_f32(&mut temp, target_sr, target_ch);
        for (i, s) in temp.iter().enumerate() {
            let val = (*s * PCM_I24_MAX).clamp(PCM_I24_MIN, PCM_I24_MAX) as i32;
            let bytes = val.to_le_bytes();
            let base = i * 3;
            output[base] = bytes[0];
            output[base + 1] = bytes[1];
            output[base + 2] = bytes[2];
        }
        frames
    }

    fn read_i32(&mut self, output: &mut [i32], target_sr: u32, target_ch: u16) -> usize {
        let mut temp = vec![0.0f32; output.len()];
        let frames = self.read_f32(&mut temp, target_sr, target_ch);
        for (i, s) in temp.iter().enumerate() {
            output[i] = (*s * PCM_I32_MAX).clamp(PCM_I32_MIN, PCM_I32_MAX) as i32;
        }
        frames
    }
}

#[no_mangle]
/// Returns whether mic spoofing is enabled for the given UID
pub extern "C" fn mic_spoofing_is_enabled_for_uid(uid: i32) -> bool {
    match is_enabled_for_uid_inner(uid) {
        Ok(enabled) => enabled,
        Err(e) => {
            android_log(ANDROID_LOG_ERROR, &format!("is_enabled_for_uid: {}", e));
            false
        }
    }
}

#[no_mangle]
/// Creates a spoofed audio source for the provided uid
/// Returns a null pointer if source creation fails
pub extern "C" fn mic_spoofing_create_source(uid: i32) -> *mut std::ffi::c_void {
    let default_wav_path = resolve_default_wav_path();
    match create_source_with_fallback(uid, default_wav_path.as_str(), get_custom_source_fd) {
        Some(src) => Box::into_raw(Box::new(src)) as *mut std::ffi::c_void,
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
/// # Safety
/// `source` must be either null or a pointer previously returned by
/// `mic_spoofing_create_source`, and it must not be used after this call
pub unsafe extern "C" fn mic_spoofing_destroy_source(source: *mut std::ffi::c_void) {
    if !source.is_null() {
        // SAFETY: `source` is non-null and expected to originate from
        // `Box::into_raw(Box<SpoofedAudioSource>)` in `mic_spoofing_create_source`.
        unsafe {
            drop(Box::from_raw(source as *mut SpoofedAudioSource));
        }
    }
}

#[no_mangle]
/// # Safety
/// `source` must be null or a valid pointer returned by
/// `mic_spoofing_create_source`. `buffer` must point to writable memory for
/// at least `frame_count * channel_count * bytes_per_sample(audio_format)` bytes
pub unsafe extern "C" fn mic_spoofing_read_samples(
    source: *mut std::ffi::c_void,
    buffer: *mut u8,
    frame_count: usize,
    sample_rate: u32,
    channel_count: u32,
    audio_format: u32,
) -> usize {
    if buffer.is_null() || frame_count == 0 || sample_rate == 0 || channel_count == 0 {
        return 0;
    }

    if channel_count > u16::MAX as u32 {
        return 0;
    }

    let fill_requested_silence = || {
        // SAFETY: This function validated non-null `buffer`; length arithmetic is
        // validated inside `fill_silence`
        unsafe {
            fill_silence(buffer, frame_count, channel_count, audio_format);
        }
    };

    if source.is_null() {
        fill_requested_silence();
        return frame_count;
    }

    // SAFETY: `source` is non-null and expected to be a valid pointer returned
    // by `mic_spoofing_create_source`
    let src = unsafe { &mut *(source as *mut SpoofedAudioSource) };
    let ch = channel_count as u16;
    let Some(sample_count) = frame_count.checked_mul(channel_count as usize) else {
        return 0;
    };

    if sample_count > MAX_WAV_SAMPLES as usize {
        return 0;
    }

    let frames = match audio_format {
        AUDIO_FORMAT_PCM_16_BIT => {
            if (buffer as usize) % std::mem::align_of::<i16>() != 0 {
                fill_requested_silence();
                return frame_count;
            }
            // SAFETY: alignment is checked above and caller guarantees writable
            // output buffer with sufficient size
            let out = unsafe { std::slice::from_raw_parts_mut(buffer as *mut i16, sample_count) };
            src.read_i16(out, sample_rate, ch)
        }
        AUDIO_FORMAT_PCM_FLOAT => {
            if (buffer as usize) % std::mem::align_of::<f32>() != 0 {
                fill_requested_silence();
                return frame_count;
            }
            // SAFETY: alignment is checked above and caller guarantees writable
            // output buffer with sufficient size
            let out = unsafe { std::slice::from_raw_parts_mut(buffer as *mut f32, sample_count) };
            src.read_f32(out, sample_rate, ch)
        }
        AUDIO_FORMAT_PCM_8_BIT => {
            // SAFETY: caller guarantees writable output buffer with sufficient size.
            let out = unsafe { std::slice::from_raw_parts_mut(buffer, sample_count) };
            src.read_u8(out, sample_rate, ch)
        }
        AUDIO_FORMAT_PCM_32_BIT => {
            if (buffer as usize) % std::mem::align_of::<i32>() != 0 {
                fill_requested_silence();
                return frame_count;
            }
            // SAFETY: alignment is checked above and caller guarantees writable
            // output buffer with sufficient size
            let out = unsafe { std::slice::from_raw_parts_mut(buffer as *mut i32, sample_count) };
            src.read_i32(out, sample_rate, ch)
        }
        AUDIO_FORMAT_PCM_24_BIT_PACKED => {
            let Some(byte_count) = sample_count.checked_mul(3) else {
                return 0;
            };
            // SAFETY: caller guarantees writable output buffer with sufficient size
            let out = unsafe { std::slice::from_raw_parts_mut(buffer, byte_count) };
            src.read_i24_packed(out, sample_rate, ch)
        }
        _ => {
            fill_requested_silence();
            frame_count
        }
    };

    if frames == 0 {
        fill_requested_silence();
        return frame_count;
    }

    frames
}

#[cfg(test)]
mod tests;
