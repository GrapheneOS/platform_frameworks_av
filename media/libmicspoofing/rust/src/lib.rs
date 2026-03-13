//! Microphone spoofing runtime support for media capture clients

mod adapter;
mod logging;

use std::cell::RefCell;
use std::ffi::{c_char, c_void, CStr};
use std::fs::File;
use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};
use std::sync::OnceLock;

use adapter::SpoofedAudioSource;
use jni::sys::{jsize, JNI_GetCreatedJavaVMs, JNI_OK};
use jni::{JNIEnv, JavaVM};
use logging::{android_log, ANDROID_LOG_DEBUG, ANDROID_LOG_ERROR};
use packagemanager_aidl::aidl::android::content::pm::IPackageManagerNative::IPackageManagerNative;

pub(crate) const DEFAULT_SPOOFED_AUDIO_PATH: &str = "/system/etc/spoofed_mic_audio_default.wav";
const FIRST_APPLICATION_UID: i32 = 10_000;

const PROP_VALUE_MAX: usize = 92;
const DEBUGGABLE_PROP: &[u8] = b"ro.debuggable\0";
const TEST_DEFAULT_WAV_PATH_PROP: &[u8] = b"debug.mic_spoofing.default_audio_path\0";
const TEST_DEFAULT_WAV_PATH_PREFIX: &str = "/data/local/tmp/";

// Keep a hard per-call bound until the temporary conversion buffers are removed.
pub(crate) const MAX_READ_SAMPLES: u32 = 2_880_000;

// Float-to-PCM conversion constants, matching clamp*_from_float behavior.
pub(crate) const PCM_I16_MAX: f32 = 32_767.0;
pub(crate) const PCM_I16_MIN: f32 = -32_768.0;
pub(crate) const PCM_I24_MAX: f32 = 8_388_607.0;
pub(crate) const PCM_I24_MIN: f32 = -8_388_608.0;
pub(crate) const PCM_I32_MAX: f32 = 2_147_483_647.0;
pub(crate) const PCM_I32_MIN: f32 = -2_147_483_648.0;
pub(crate) const PCM_U8_SCALE: f32 = 127.0;
pub(crate) const PCM_U8_OFFSET: f32 = 128.0;

// Native audio PCM sub-format values
pub(crate) const AUDIO_FORMAT_PCM_16_BIT: u32 = 0x1;
pub(crate) const AUDIO_FORMAT_PCM_8_BIT: u32 = 0x2;
pub(crate) const AUDIO_FORMAT_PCM_32_BIT: u32 = 0x3;
pub(crate) const AUDIO_FORMAT_PCM_FLOAT: u32 = 0x5;
pub(crate) const AUDIO_FORMAT_PCM_24_BIT_PACKED: u32 = 0x6;

pub(crate) type DecoderFactoryFn = unsafe extern "C" fn(
    source_fd: i32,
    out_sample_rate: *mut u32,
    out_channel_count: *mut u32,
) -> i32;

static DECODER_FACTORY: OnceLock<DecoderFactoryFn> = OnceLock::new();

unsafe extern "C" {
    fn __system_property_get(name: *const c_char, value: *mut c_char) -> i32;
    fn getuid() -> u32;
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

fn resolve_default_audio_path() -> String {
    let is_debuggable = read_system_property(DEBUGGABLE_PROP).as_deref() == Some("1");
    if !is_debuggable {
        return DEFAULT_SPOOFED_AUDIO_PATH.to_string();
    }

    let Some(path) = read_system_property(TEST_DEFAULT_WAV_PATH_PROP) else {
        return DEFAULT_SPOOFED_AUDIO_PATH.to_string();
    };

    let trimmed = path.trim();
    if trimmed.is_empty() {
        return DEFAULT_SPOOFED_AUDIO_PATH.to_string();
    }

    if !trimmed.starts_with(TEST_DEFAULT_WAV_PATH_PREFIX) {
        android_log(
            ANDROID_LOG_ERROR,
            "Ignoring debug.mic_spoofing.default_audio_path outside /data/local/tmp/",
        );
        return DEFAULT_SPOOFED_AUDIO_PATH.to_string();
    }

    trimmed.to_owned()
}

pub(crate) fn current_process_uid() -> i32 {
    // SAFETY: `getuid` is process-local and takes no arguments
    unsafe { getuid() as i32 }
}

pub(crate) fn bytes_per_sample(audio_format: u32) -> usize {
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
    let Some(byte_count) = frame_count
        .checked_mul(channel_count as usize)
        .and_then(|sample_count| sample_count.checked_mul(bps))
    else {
        return;
    };
    let silence = if audio_format == AUDIO_FORMAT_PCM_8_BIT {
        128u8
    } else {
        0u8
    };
    // SAFETY: caller guarantees `buffer` points to writable storage for `byte_count` bytes
    unsafe {
        std::slice::from_raw_parts_mut(buffer, byte_count).fill(silence);
    }
}

fn is_enabled_for_uid_inner(uid: i32) -> Result<bool, String> {
    let pm: binder::Strong<dyn IPackageManagerNative> = binder::check_interface("package_native")
        .map_err(|e| format!("cannot find package_native service: {e:?}"))?;

    pm.isMicSpoofingEnabledForUid(uid)
        .map_err(|e| format!("binder call failed: {e}"))
}

fn get_custom_source_fd(uid: i32) -> Result<Option<File>, String> {
    let current_uid = current_process_uid();
    if uid != current_uid {
        android_log(
            ANDROID_LOG_DEBUG,
            &format!(
                "mic_spoofing: refusing custom source lookup for uid={} from uid={}",
                uid,
                current_uid
            ),
        );
        return Ok(None);
    }

    if uid < FIRST_APPLICATION_UID {
        android_log(
            ANDROID_LOG_DEBUG,
            &format!(
                "mic_spoofing: skipping custom source lookup for non-app uid={}",
                uid
            ),
        );
        return Ok(None);
    }

    let mut vm_ptr = std::ptr::null_mut();
    let mut vm_count: jsize = 0;
    // SAFETY: `vm_ptr` and `vm_count` are writable out-pointers and the capacity is 1 entry
    let status = unsafe { JNI_GetCreatedJavaVMs(&mut vm_ptr, 1, &mut vm_count) };
    if status != JNI_OK || vm_count == 0 || vm_ptr.is_null() {
        return Err(format!(
            "failed to get Java VM instance: status={} vm_count={}",
            status, vm_count
        ));
    }

    // SAFETY: `vm_ptr` was returned by `JNI_GetCreatedJavaVMs` for this process and is non-null
    let vm = unsafe { JavaVM::from_raw(vm_ptr) }
        .map_err(|e| format!("failed to wrap Java VM: {e}"))?;
    let mut env = vm
        .attach_current_thread()
        .map_err(|e| format!("failed to attach current thread to Java VM: {e}"))?;

    let parcel_file_descriptor = match env.call_static_method(
        "android/ext/micspoofing/MicSpoofingApi",
        "openCustomSourceFdForSelf",
        "()Landroid/os/ParcelFileDescriptor;",
        &[],
    ) {
        Ok(value) => value
            .l()
            .map_err(|e| format!("openCustomSourceFdForSelf returned invalid type: {e}"))?,
        Err(e) => {
            clear_pending_jni_exception(
                &mut env,
                "openCustomSourceFdForSelf JNI call failed",
            );
            return Err(format!("openCustomSourceFdForSelf JNI call failed: {e}"));
        }
    };

    if parcel_file_descriptor.is_null() {
        return Ok(None);
    }

    let detached_fd = match env.call_method(parcel_file_descriptor, "detachFd", "()I", &[]) {
        Ok(value) => value
            .i()
            .map_err(|e| format!("ParcelFileDescriptor.detachFd returned invalid type: {e}"))?,
        Err(e) => {
            clear_pending_jni_exception(
                &mut env,
                "ParcelFileDescriptor.detachFd JNI call failed",
            );
            return Err(format!("ParcelFileDescriptor.detachFd failed: {e}"));
        }
    };

    if detached_fd < 0 {
        return Err(format!(
            "ParcelFileDescriptor.detachFd returned invalid fd: {}",
            detached_fd
        ));
    }

    // SAFETY: `detachFd` transfers ownership of a live fd to the caller exactly once here
    Ok(Some(unsafe { File::from(OwnedFd::from_raw_fd(detached_fd)) }))
}

fn clear_pending_jni_exception(env: &mut JNIEnv, context: &str) {
    match env.exception_check() {
        Ok(true) => {
            let _ = env.exception_clear();
            android_log(
                ANDROID_LOG_ERROR,
                &format!("mic_spoofing: cleared pending JNI exception after {context}"),
            );
        }
        Ok(false) => {}
        Err(e) => {
            android_log(
                ANDROID_LOG_ERROR,
                &format!("mic_spoofing: failed to check pending JNI exception after {context}: {e}"),
            );
        }
    }
}

struct PendingSourceFd {
    fd: OwnedFd,
    sample_rate: u32,
    channel_count: u32,
}

thread_local! {
    static PENDING_SOURCE_FD: RefCell<Option<PendingSourceFd>> = const { RefCell::new(None) };
}

fn clear_pending_source_fd_inner() {
    PENDING_SOURCE_FD.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

fn registered_decoder_factory() -> Option<DecoderFactoryFn> {
    #[cfg(test)]
    {
        if let Some(factory) = test_decoder_factory_override() {
            return Some(factory);
        }
    }

    DECODER_FACTORY.get().copied()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SourceKind {
    Custom,
    Default,
}

struct OpenedSourceFile {
    file: File,
    kind: SourceKind,
}

fn open_source_file_with_fallback<F>(
    uid: i32,
    default_audio_path: &str,
    get_custom_source_fd_fn: F,
) -> Result<OpenedSourceFile, String>
where
    F: FnOnce(i32) -> Result<Option<File>, String>,
{
    match get_custom_source_fd_fn(uid) {
        Ok(Some(custom_fd)) => {
            return Ok(OpenedSourceFile {
                file: custom_fd,
                kind: SourceKind::Custom,
            });
        }
        Ok(None) => {
            android_log(
                ANDROID_LOG_DEBUG,
                &format!("mic_spoofing: no custom source fd for uid={uid}"),
            );
        }
        Err(e) => {
            android_log(
                ANDROID_LOG_ERROR,
                &format!("mic_spoofing: custom source fd lookup failed: {e}"),
            );
        }
    }

    open_default_source_file_with_system_fallback(default_audio_path, DEFAULT_SPOOFED_AUDIO_PATH)
        .map(|file| OpenedSourceFile {
            file,
            kind: SourceKind::Default,
        })
}

fn open_default_source_file_with_system_fallback(
    default_audio_path: &str,
    system_audio_path: &str,
) -> Result<File, String> {
    match File::open(default_audio_path) {
        Ok(file) => Ok(file),
        Err(err) => {
            android_log(
                ANDROID_LOG_ERROR,
                &format!(
                    "mic_spoofing: failed to open default audio source {}: {}",
                    default_audio_path, err
                ),
            );
            if default_audio_path == system_audio_path {
                return Err(format!("failed to open default audio source: {err}"));
            }

            android_log(
                ANDROID_LOG_ERROR,
                "mic_spoofing: falling back to built-in default audio source",
            );
            File::open(system_audio_path).map_err(|fallback_err| {
                format!("failed to open fallback audio source: {fallback_err}")
            })
        }
    }
}

fn call_decoder_factory(
    factory: DecoderFactoryFn,
    source_file: File,
) -> Result<(OwnedFd, u32, u32), String> {
    let mut sample_rate = 0u32;
    let mut channel_count = 0u32;
    let source_fd = source_file.into_raw_fd();

    // SAFETY: `source_fd` ownership is transferred to the registered decoder factory
    let pipe_fd = unsafe { factory(source_fd, &mut sample_rate, &mut channel_count) };
    if pipe_fd < 0 {
        return Err("decoder factory failed".to_string());
    }

    // SAFETY: a non-negative return value is an owned pipe read-end
    let pipe_read = unsafe { OwnedFd::from_raw_fd(pipe_fd) };
    if sample_rate == 0 || channel_count == 0 {
        return Err("decoder factory returned invalid source metadata".to_string());
    }

    Ok((pipe_read, sample_rate, channel_count))
}

fn start_streaming_decoder_for_uid(uid: i32) -> Result<(OwnedFd, u32, u32), String> {
    android_log(
        ANDROID_LOG_DEBUG,
        &format!("starting streaming decoder for uid={}", uid),
    );

    let Some(factory) = registered_decoder_factory() else {
        return Err("no decoder factory registered".to_string());
    };

    let default_audio_path = resolve_default_audio_path();
    start_streaming_decoder_with_factory_and_fallback(
        factory,
        uid,
        &default_audio_path,
        get_custom_source_fd,
    )
}

pub(crate) fn start_streaming_decoder_with_factory_and_fallback<F>(
    factory: DecoderFactoryFn,
    uid: i32,
    default_audio_path: &str,
    get_custom_source_fd_fn: F,
) -> Result<(OwnedFd, u32, u32), String>
where
    F: FnOnce(i32) -> Result<Option<File>, String>,
{
    let source = open_source_file_with_fallback(uid, default_audio_path, get_custom_source_fd_fn)?;
    match call_decoder_factory(factory, source.file) {
        Ok(result) => Ok(result),
        Err(custom_decode_err) if source.kind == SourceKind::Custom => {
            android_log(
                ANDROID_LOG_ERROR,
                &format!(
                    "mic_spoofing: custom source decode failed, falling back to default: {custom_decode_err}"
                ),
            );

            let default_source = open_default_source_file_with_system_fallback(
                default_audio_path,
                DEFAULT_SPOOFED_AUDIO_PATH,
            )?;
            call_decoder_factory(factory, default_source).map_err(|default_decode_err| {
                format!(
                    "custom source decode failed: {custom_decode_err}; default source decode failed: {default_decode_err}"
                )
            })
        }
        Err(err) => Err(err),
    }
}

pub(crate) fn create_source_from_fd(
    pipe_read: OwnedFd,
    sample_rate: u32,
    channel_count: u32,
) -> *mut c_void {
    if sample_rate == 0 || channel_count == 0 {
        android_log(
            ANDROID_LOG_ERROR,
            &format!(
                "mic_spoofing_create_source: invalid pipe metadata sr={} ch={}",
                sample_rate, channel_count
            ),
        );
        return create_silence_source();
    }

    Box::into_raw(Box::new(SpoofedAudioSource::from_pipe(
        pipe_read,
        sample_rate,
        channel_count,
    ))) as *mut c_void
}

pub(crate) fn create_silence_source() -> *mut c_void {
    Box::into_raw(Box::new(SpoofedAudioSource::silence())) as *mut c_void
}

#[no_mangle]
/// Returns whether microphone spoofing is enabled for the provided app UID
pub extern "C" fn mic_spoofing_is_enabled_for_uid(uid: i32) -> bool {
    match is_enabled_for_uid_inner(uid) {
        Ok(enabled) => enabled,
        Err(e) => {
            android_log(ANDROID_LOG_ERROR, &format!("is_enabled_for_uid: {e}"));
            false
        }
    }
}

#[no_mangle]
/// Registers the decoder factory used to build streaming decoders for spoofed sources
pub extern "C" fn mic_spoofing_set_decoder_factory(factory: Option<DecoderFactoryFn>) {
    let Some(factory) = factory else {
        android_log(
            ANDROID_LOG_ERROR,
            "set_decoder_factory: ignoring null factory",
        );
        return;
    };

    let _ = DECODER_FACTORY.set(factory);
}

#[no_mangle]
/// Starts a streaming decoder for the provided app UID and returns the readable pipe fd
/// # Safety
/// `out_sample_rate` and `out_channel_count` must be valid writable pointers
pub unsafe extern "C" fn mic_spoofing_start_streaming_decoder(
    uid: i32,
    out_sample_rate: *mut u32,
    out_channel_count: *mut u32,
) -> i32 {
    if out_sample_rate.is_null() || out_channel_count.is_null() {
        android_log(
            ANDROID_LOG_ERROR,
            &format!("start_streaming_decoder: missing output storage for uid={uid}"),
        );
        return -1;
    }

    // SAFETY: the pointers are validated above and point to caller-owned writable storage
    unsafe {
        *out_sample_rate = 0;
        *out_channel_count = 0;
    }

    match start_streaming_decoder_for_uid(uid) {
        Ok((pipe_read, sample_rate, channel_count)) => {
            // SAFETY: pointers are still valid for this call
            unsafe {
                *out_sample_rate = sample_rate;
                *out_channel_count = channel_count;
            }
            pipe_read.into_raw_fd()
        }
        Err(e) => {
            android_log(
                ANDROID_LOG_ERROR,
                &format!("start_streaming_decoder: failed for uid={uid}: {e}"),
            );
            -1
        }
    }
}

#[no_mangle]
/// Stores a pending decoded source fd to be consumed by the next source creation call
pub extern "C" fn mic_spoofing_set_pending_source_fd(
    fd: i32,
    source_sample_rate: u32,
    source_channel_count: u32,
) {
    let owned_fd = if fd >= 0 {
        // SAFETY: the caller transfers ownership of a valid file descriptor
        Some(unsafe { OwnedFd::from_raw_fd(fd) })
    } else {
        None
    };

    if owned_fd.is_none() || source_sample_rate == 0 || source_channel_count == 0 {
        android_log(
            ANDROID_LOG_ERROR,
            &format!(
                "set_pending_source_fd: rejecting fd={} sr={} ch={}",
                fd, source_sample_rate, source_channel_count
            ),
        );
        clear_pending_source_fd_inner();
        return;
    }

    let owned_fd = owned_fd.unwrap();
    PENDING_SOURCE_FD.with(|cell| {
        *cell.borrow_mut() = Some(PendingSourceFd {
            fd: owned_fd,
            sample_rate: source_sample_rate,
            channel_count: source_channel_count,
        });
    });
}

#[no_mangle]
/// Clears any pending decoded source fd staged for source creation
pub extern "C" fn mic_spoofing_clear_pending_source_fd() {
    clear_pending_source_fd_inner();
}

#[no_mangle]
/// Creates a spoofed audio source for the provided app UID
pub extern "C" fn mic_spoofing_create_source(uid: i32) -> *mut c_void {
    let pending = PENDING_SOURCE_FD.with(|cell| cell.borrow_mut().take());
    if let Some(pending) = pending {
        return create_source_from_fd(pending.fd, pending.sample_rate, pending.channel_count);
    }

    if current_process_uid() != uid {
        android_log(
            ANDROID_LOG_DEBUG,
            &format!(
                "mic_spoofing_create_source: uid {} does not match current process uid {}, failing closed to silence",
                uid,
                current_process_uid()
            ),
        );
        return create_silence_source();
    }

    match start_streaming_decoder_for_uid(uid) {
        Ok((pipe_read, sample_rate, channel_count)) => {
            create_source_from_fd(pipe_read, sample_rate, channel_count)
        }
        Err(e) => {
            android_log(
                ANDROID_LOG_ERROR,
                &format!("mic_spoofing_create_source: local decoder start failed: {e}"),
            );
            create_silence_source()
        }
    }
}

#[no_mangle]
/// # Safety
/// `source` must be either null or a pointer previously returned by
/// `mic_spoofing_create_source`, and it must not be used after this call
pub unsafe extern "C" fn mic_spoofing_destroy_source(source: *mut c_void) {
    if !source.is_null() {
        // SAFETY: `source` originates from `Box::into_raw(Box<SpoofedAudioSource>)`
        unsafe {
            drop(Box::from_raw(source as *mut SpoofedAudioSource));
        }
    }
}

#[no_mangle]
/// # Safety
/// `source` must be null or a valid pointer returned by `mic_spoofing_create_source`.
/// `buffer` must point to writable memory for at least
/// `frame_count * channel_count * bytes_per_sample(audio_format)` bytes
pub unsafe extern "C" fn mic_spoofing_read_samples(
    source: *mut c_void,
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
        // SAFETY: this function validated non-null `buffer`; length arithmetic is validated
        // inside `fill_silence`
        unsafe {
            fill_silence(buffer, frame_count, channel_count, audio_format);
        }
    };

    if source.is_null() {
        fill_requested_silence();
        return frame_count;
    }

    let Some(sample_count) = frame_count.checked_mul(channel_count as usize) else {
        return 0;
    };
    if sample_count > MAX_READ_SAMPLES as usize {
        return 0;
    }

    let channel_count = channel_count as u16;

    // SAFETY: `source` is non-null and expected to be a valid pointer returned by create_source
    let source = unsafe { &mut *(source as *mut SpoofedAudioSource) };
    let frames = match audio_format {
        AUDIO_FORMAT_PCM_16_BIT => {
            if (buffer as usize) % std::mem::align_of::<i16>() != 0 {
                fill_requested_silence();
                return frame_count;
            }
            // SAFETY: alignment is checked above and caller guarantees writable output memory
            let out = unsafe { std::slice::from_raw_parts_mut(buffer as *mut i16, sample_count) };
            source.read_i16(out, sample_rate, channel_count)
        }
        AUDIO_FORMAT_PCM_FLOAT => {
            if (buffer as usize) % std::mem::align_of::<f32>() != 0 {
                fill_requested_silence();
                return frame_count;
            }
            // SAFETY: alignment is checked above and caller guarantees writable output memory
            let out = unsafe { std::slice::from_raw_parts_mut(buffer as *mut f32, sample_count) };
            source.read_f32(out, sample_rate, channel_count)
        }
        AUDIO_FORMAT_PCM_8_BIT => {
            // SAFETY: caller guarantees writable output memory with `sample_count` bytes
            let out = unsafe { std::slice::from_raw_parts_mut(buffer, sample_count) };
            source.read_u8(out, sample_rate, channel_count)
        }
        AUDIO_FORMAT_PCM_32_BIT => {
            if (buffer as usize) % std::mem::align_of::<i32>() != 0 {
                fill_requested_silence();
                return frame_count;
            }
            // SAFETY: alignment is checked above and caller guarantees writable output memory
            let out = unsafe { std::slice::from_raw_parts_mut(buffer as *mut i32, sample_count) };
            source.read_i32(out, sample_rate, channel_count)
        }
        AUDIO_FORMAT_PCM_24_BIT_PACKED => {
            let Some(byte_count) = sample_count.checked_mul(3) else {
                return 0;
            };
            // SAFETY: caller guarantees writable output memory with `byte_count` bytes
            let out = unsafe { std::slice::from_raw_parts_mut(buffer, byte_count) };
            source.read_i24_packed(out, sample_rate, channel_count)
        }
        _ => {
            fill_requested_silence();
            return frame_count;
        }
    };

    if frames == 0 {
        fill_requested_silence();
        return frame_count;
    }

    frames
}

#[cfg(test)]
fn test_decoder_factory_override() -> Option<DecoderFactoryFn> {
    *TEST_DECODER_FACTORY.lock().unwrap()
}

#[cfg(test)]
static TEST_DECODER_FACTORY: std::sync::Mutex<Option<DecoderFactoryFn>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
static TEST_DECODER_FACTORY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
fn lock_test_decoder_factory() -> std::sync::MutexGuard<'static, ()> {
    TEST_DECODER_FACTORY_LOCK.lock().unwrap()
}

#[cfg(test)]
struct TestDecoderFactoryGuard(Option<DecoderFactoryFn>);

#[cfg(test)]
impl Drop for TestDecoderFactoryGuard {
    fn drop(&mut self) {
        *TEST_DECODER_FACTORY.lock().unwrap() = self.0;
    }
}

#[cfg(test)]
fn set_test_decoder_factory(factory: Option<DecoderFactoryFn>) -> TestDecoderFactoryGuard {
    let mut slot = TEST_DECODER_FACTORY.lock().unwrap();
    let previous = *slot;
    *slot = factory;
    TestDecoderFactoryGuard(previous)
}

#[cfg(test)]
mod tests;
