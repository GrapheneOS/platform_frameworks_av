use super::*;
use std::ffi::c_void;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::time::{SystemTime, UNIX_EPOCH};

const O_CLOEXEC: i32 = 0o2000000;

unsafe extern "C" {
    fn close(fd: i32) -> i32;
    fn dup(fd: i32) -> i32;
    fn pipe2(pipefd: *mut i32, flags: i32) -> i32;
    fn read(fd: i32, buf: *mut c_void, count: usize) -> isize;
    fn write(fd: i32, buf: *const c_void, count: usize) -> isize;
}

#[derive(Clone)]
struct TestFactoryConfig {
    output_samples: Vec<f32>,
    sample_rate: u32,
    channel_count: u32,
    capture_prefix_len: usize,
    fail_on_prefix: Option<Vec<u8>>,
}

static TEST_FACTORY_CONFIG: std::sync::Mutex<Option<TestFactoryConfig>> =
    std::sync::Mutex::new(None);
static TEST_FACTORY_CAPTURED_SOURCE_BYTES: std::sync::Mutex<Vec<u8>> =
    std::sync::Mutex::new(Vec::new());
static TEST_FACTORY_CAPTURED_SOURCE_CALLS: std::sync::Mutex<Vec<Vec<u8>>> =
    std::sync::Mutex::new(Vec::new());

fn test_temp_path(name: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!(
        "{}/micspoofing_test_{}_{}",
        std::env::temp_dir().display(),
        name,
        nanos
    )
}

fn make_foreign_uid() -> i32 {
    current_process_uid().saturating_add(1)
}

fn create_pipe_pair() -> (OwnedFd, OwnedFd) {
    let mut fds = [-1i32; 2];
    // SAFETY: `fds` points to two writable integers for `pipe2`
    let result = unsafe { pipe2(fds.as_mut_ptr(), O_CLOEXEC) };
    assert_eq!(
        result,
        0,
        "pipe2 failed: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: `pipe2` produced owned file descriptors on success
    let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    // SAFETY: `pipe2` produced owned file descriptors on success
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    (read_fd, write_fd)
}

fn write_all_fd(fd: &OwnedFd, bytes: &[u8]) {
    let mut written = 0usize;
    while written < bytes.len() {
        // SAFETY: `fd` is live and `bytes` points to readable memory
        let result = unsafe {
            write(
                fd.as_raw_fd(),
                bytes[written..].as_ptr().cast(),
                bytes.len() - written,
            )
        };
        assert!(
            result >= 0,
            "write failed: {}",
            std::io::Error::last_os_error()
        );
        written += result as usize;
    }
}

fn create_f32_pipe(samples: &[f32]) -> OwnedFd {
    let (read_fd, write_fd) = create_pipe_pair();
    // SAFETY: `samples` is contiguous f32 storage and the byte view covers the same memory
    let bytes = unsafe {
        std::slice::from_raw_parts(
            samples.as_ptr().cast::<u8>(),
            std::mem::size_of_val(samples),
        )
    };
    write_all_fd(&write_fd, bytes);
    drop(write_fd);
    read_fd
}

fn read_f32_from_fd(fd: &OwnedFd, sample_count: usize) -> Vec<f32> {
    let mut bytes = vec![0u8; sample_count * std::mem::size_of::<f32>()];
    let mut read_total = 0usize;
    while read_total < bytes.len() {
        // SAFETY: `fd` is live and `bytes` points to writable memory
        let result = unsafe {
            read(
                fd.as_raw_fd(),
                bytes[read_total..].as_mut_ptr().cast(),
                bytes.len() - read_total,
            )
        };
        assert!(
            result >= 0,
            "read failed: {}",
            std::io::Error::last_os_error()
        );
        if result == 0 {
            break;
        }
        read_total += result as usize;
    }

    bytes[..read_total]
        .chunks_exact(4)
        .map(|chunk| f32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

fn read_f32_from_source(
    source: *mut c_void,
    frame_count: usize,
    sample_rate: u32,
    channel_count: u32,
) -> Vec<f32> {
    let mut output = vec![1.0f32; frame_count * channel_count as usize];
    // SAFETY: `source` is either a valid source pointer or null and `output` is writable storage
    let frames = unsafe {
        mic_spoofing_read_samples(
            source,
            output.as_mut_ptr().cast(),
            frame_count,
            sample_rate,
            channel_count,
            AUDIO_FORMAT_PCM_FLOAT,
        )
    };
    assert_eq!(frames, frame_count);
    output
}

fn clear_pending_source_fd_for_test() {
    mic_spoofing_clear_pending_source_fd();
    PENDING_SOURCE_FD.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "pending source fd should be cleared"
        );
    });
}

fn dup_fd(fd: i32) -> Option<i32> {
    // SAFETY: `dup` duplicates the descriptor without borrowing Rust references
    let duplicated = unsafe { dup(fd) };
    if duplicated < 0 {
        None
    } else {
        Some(duplicated)
    }
}

fn close_fd(fd: i32) {
    // SAFETY: tests only call this for descriptors they own
    unsafe {
        close(fd);
    }
}

fn approx_eq(left: f32, right: f32) -> bool {
    (left - right).abs() < 0.0001
}

fn assert_approx_slice(left: &[f32], right: &[f32]) {
    assert_eq!(left.len(), right.len(), "slice lengths differ");
    for (index, (lhs, rhs)) in left.iter().zip(right.iter()).enumerate() {
        assert!(
            approx_eq(*lhs, *rhs),
            "mismatch at index {}: {} vs {}",
            index,
            lhs,
            rhs
        );
    }
}

fn configure_test_factory(config: TestFactoryConfig) {
    *TEST_FACTORY_CONFIG.lock().unwrap() = Some(config);
    TEST_FACTORY_CAPTURED_SOURCE_BYTES.lock().unwrap().clear();
    TEST_FACTORY_CAPTURED_SOURCE_CALLS.lock().unwrap().clear();
}

fn take_captured_source_bytes() -> Vec<u8> {
    std::mem::take(&mut *TEST_FACTORY_CAPTURED_SOURCE_BYTES.lock().unwrap())
}

fn take_captured_source_calls() -> Vec<Vec<u8>> {
    std::mem::take(&mut *TEST_FACTORY_CAPTURED_SOURCE_CALLS.lock().unwrap())
}

unsafe extern "C" fn test_decoder_factory(
    source_fd: i32,
    out_sample_rate: *mut u32,
    out_channel_count: *mut u32,
) -> i32 {
    let config = TEST_FACTORY_CONFIG
        .lock()
        .unwrap()
        .clone()
        .expect("test decoder factory must be configured");
    // SAFETY: the caller transfers ownership of `source_fd` to the factory
    let mut source_file = unsafe { std::fs::File::from(OwnedFd::from_raw_fd(source_fd)) };
    let mut captured = vec![0u8; config.capture_prefix_len];
    let bytes_read = source_file.read(&mut captured).unwrap();
    captured.truncate(bytes_read);
    *TEST_FACTORY_CAPTURED_SOURCE_BYTES.lock().unwrap() = captured.clone();
    TEST_FACTORY_CAPTURED_SOURCE_CALLS
        .lock()
        .unwrap()
        .push(captured.clone());

    if config.fail_on_prefix.as_deref() == Some(captured.as_slice()) {
        return -1;
    }

    if !out_sample_rate.is_null() {
        // SAFETY: the pointer is checked above and valid for this call
        unsafe {
            *out_sample_rate = config.sample_rate;
        }
    }
    if !out_channel_count.is_null() {
        // SAFETY: the pointer is checked above and valid for this call
        unsafe {
            *out_channel_count = config.channel_count;
        }
    }

    create_f32_pipe(&config.output_samples).into_raw_fd()
}

#[test]
fn test_create_silence_source_fills_zeroes() {
    let source = create_silence_source();
    let output = read_f32_from_source(source, 8, 48_000, 1);
    assert!(output.iter().all(|sample| *sample == 0.0));
    // SAFETY: `source` came from `create_silence_source`
    unsafe { mic_spoofing_destroy_source(source) };
}

#[test]
fn test_create_source_from_fd_reads_pipe_samples_then_silence() {
    let pipe_read = create_f32_pipe(&[0.25, -0.5, 0.75]);
    let source = create_source_from_fd(pipe_read, 48_000, 1);
    let output = read_f32_from_source(source, 6, 48_000, 1);
    assert_approx_slice(&output[..3], &[0.25, -0.5, 0.75]);
    assert!(output[3..].iter().all(|sample| *sample == 0.0));
    // SAFETY: `source` came from `create_source_from_fd`
    unsafe { mic_spoofing_destroy_source(source) };
}

#[test]
fn test_adapter_resamples_and_upmixes_from_pipe() {
    let pipe_read = create_f32_pipe(&[0.1, 0.2]);
    let source = create_source_from_fd(pipe_read, 24_000, 1);
    let output = read_f32_from_source(source, 4, 48_000, 2);
    assert_approx_slice(&output, &[0.1, 0.1, 0.1, 0.1, 0.2, 0.2, 0.2, 0.2]);
    // SAFETY: `source` came from `create_source_from_fd`
    unsafe { mic_spoofing_destroy_source(source) };
}

#[test]
fn test_adapter_downsamples_and_uses_silence_after_eof() {
    let pipe_read = create_f32_pipe(&[0.1, 0.2, 0.3, 0.4]);
    let source = create_source_from_fd(pipe_read, 48_000, 1);
    let output = read_f32_from_source(source, 3, 16_000, 1);
    assert_approx_slice(&output, &[0.1, 0.4, 0.0]);
    // SAFETY: `source` came from `create_source_from_fd`
    unsafe { mic_spoofing_destroy_source(source) };
}

#[test]
fn test_read_samples_unknown_format_fills_silence() {
    let pipe_read = create_f32_pipe(&[0.8]);
    let source = create_source_from_fd(pipe_read, 48_000, 1);
    let mut output = vec![0xFFu8; 16 * std::mem::size_of::<i16>()];
    // SAFETY: `source` is valid and `output` is writable
    let frames =
        unsafe { mic_spoofing_read_samples(source, output.as_mut_ptr(), 16, 48_000, 1, 0xFF) };
    assert_eq!(frames, 16);
    assert!(output.iter().all(|byte| *byte == 0));
    // SAFETY: `source` came from `create_source_from_fd`
    unsafe { mic_spoofing_destroy_source(source) };
}

#[test]
fn test_read_samples_alignment_guard_fills_silence() {
    let pipe_read = create_f32_pipe(&[0.5, -0.5]);
    let source = create_source_from_fd(pipe_read, 48_000, 1);
    let mut storage = vec![0xABu8; 16 * std::mem::size_of::<f32>() + 1];
    let requested_bytes = 4 * std::mem::size_of::<f32>();
    // SAFETY: `source` is valid and `storage[1..]` is writable but intentionally misaligned
    let frames = unsafe {
        mic_spoofing_read_samples(
            source,
            storage[1..].as_mut_ptr(),
            4,
            48_000,
            1,
            AUDIO_FORMAT_PCM_FLOAT,
        )
    };
    assert_eq!(frames, 4);
    assert!(storage[1..1 + requested_bytes].iter().all(|byte| *byte == 0));
    assert!(storage[1 + requested_bytes..].iter().all(|byte| *byte == 0xAB));
    // SAFETY: `source` came from `create_source_from_fd`
    unsafe { mic_spoofing_destroy_source(source) };
}

#[test]
fn test_read_samples_excessive_frame_count_returns_zero() {
    let pipe_read = create_f32_pipe(&[0.5]);
    let source = create_source_from_fd(pipe_read, 48_000, 1);
    let mut output = [0xCDu8; 16];
    // SAFETY: `source` is valid and `output` is writable
    let frames = unsafe {
        mic_spoofing_read_samples(
            source,
            output.as_mut_ptr(),
            MAX_READ_SAMPLES as usize + 1,
            48_000,
            1,
            AUDIO_FORMAT_PCM_16_BIT,
        )
    };
    assert_eq!(frames, 0);
    assert!(output.iter().all(|byte| *byte == 0xCD));
    // SAFETY: `source` came from `create_source_from_fd`
    unsafe { mic_spoofing_destroy_source(source) };
}

#[test]
fn test_create_source_foreign_uid_without_pending_is_silence() {
    let source = mic_spoofing_create_source(make_foreign_uid());
    assert!(
        !source.is_null(),
        "foreign uid should fail closed to a silence source"
    );

    let mut output = vec![1i16; 8];
    // SAFETY: `source` is valid and `output` is writable
    let frames = unsafe {
        mic_spoofing_read_samples(
            source,
            output.as_mut_ptr().cast(),
            8,
            48_000,
            1,
            AUDIO_FORMAT_PCM_16_BIT,
        )
    };
    assert_eq!(frames, 8);
    assert!(output.iter().all(|sample| *sample == 0));
    // SAFETY: `source` came from `mic_spoofing_create_source`
    unsafe { mic_spoofing_destroy_source(source) };
}

#[test]
fn test_create_source_current_uid_without_factory_is_silence() {
    let _factory_lock = lock_test_decoder_factory();
    let _factory_guard = set_test_decoder_factory(None);

    let source = mic_spoofing_create_source(current_process_uid());
    assert!(
        !source.is_null(),
        "missing factory should fail closed to a silence source"
    );
    let output = read_f32_from_source(source, 4, 48_000, 1);
    assert!(output.iter().all(|sample| *sample == 0.0));
    // SAFETY: `source` came from `mic_spoofing_create_source`
    unsafe { mic_spoofing_destroy_source(source) };
}

#[test]
fn test_create_source_consumes_pending_fd_before_uid_check() {
    clear_pending_source_fd_for_test();

    let (pipe_read, pipe_write) = create_pipe_pair();
    let raw_fd = pipe_read.into_raw_fd();
    let sample = 0.5f32.to_ne_bytes();
    write_all_fd(&pipe_write, &sample);
    drop(pipe_write);

    mic_spoofing_set_pending_source_fd(raw_fd, 48_000, 1);
    let source = mic_spoofing_create_source(make_foreign_uid());
    PENDING_SOURCE_FD.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "create_source should consume the pending fd"
        );
    });

    let output = read_f32_from_source(source, 2, 48_000, 1);
    assert!(approx_eq(output[0], 0.5));
    assert_eq!(output[1], 0.0);
    // SAFETY: `source` came from `mic_spoofing_create_source`
    unsafe { mic_spoofing_destroy_source(source) };
}

#[test]
fn test_set_pending_source_fd_stores_metadata_and_owns_fd() {
    clear_pending_source_fd_for_test();

    let path = test_temp_path("pending_source_fd_store.tmp");
    let file = std::fs::File::create(&path).unwrap();
    let raw_fd = file.into_raw_fd();

    mic_spoofing_set_pending_source_fd(raw_fd, 48_000, 2);

    PENDING_SOURCE_FD.with(|cell| {
        let pending = cell.borrow();
        let pending = pending
            .as_ref()
            .expect("pending source fd should be present");
        assert_eq!(pending.sample_rate, 48_000);
        assert_eq!(pending.channel_count, 2);
        assert!(
            dup_fd(raw_fd).is_some(),
            "fd should remain open while owned by thread-local storage"
        );
    });

    if let Some(duplicated) = dup_fd(raw_fd) {
        close_fd(duplicated);
    }
    clear_pending_source_fd_for_test();
    assert!(
        dup_fd(raw_fd).is_none(),
        "fd should be closed after clearing the thread-local"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn test_set_pending_source_fd_replaces_existing_fd_and_closes_old_one() {
    clear_pending_source_fd_for_test();

    let first_raw_fd = std::fs::File::create(test_temp_path("pending_first.tmp"))
        .unwrap()
        .into_raw_fd();
    let second_raw_fd = std::fs::File::create(test_temp_path("pending_second.tmp"))
        .unwrap()
        .into_raw_fd();

    mic_spoofing_set_pending_source_fd(first_raw_fd, 8_000, 1);
    assert!(dup_fd(first_raw_fd).is_some());

    mic_spoofing_set_pending_source_fd(second_raw_fd, 16_000, 2);
    assert!(
        dup_fd(first_raw_fd).is_none(),
        "replacing should close the old fd"
    );
    PENDING_SOURCE_FD.with(|cell| {
        let pending = cell.borrow();
        let pending = pending
            .as_ref()
            .expect("replacement should leave pending state");
        assert_eq!(pending.sample_rate, 16_000);
        assert_eq!(pending.channel_count, 2);
    });

    clear_pending_source_fd_for_test();
    assert!(
        dup_fd(second_raw_fd).is_none(),
        "clearing should close the replacement fd"
    );
}

#[test]
fn test_clear_pending_source_fd_is_idempotent() {
    clear_pending_source_fd_for_test();
    mic_spoofing_clear_pending_source_fd();
    PENDING_SOURCE_FD.with(|cell| {
        assert!(cell.borrow().is_none());
    });
}

#[test]
fn test_set_pending_source_fd_with_invalid_input_clears_state() {
    clear_pending_source_fd_for_test();
    mic_spoofing_set_pending_source_fd(-1, 48_000, 1);
    PENDING_SOURCE_FD.with(|cell| {
        assert!(cell.borrow().is_none(), "invalid fd should not be stored");
    });

    let temp_fd = std::fs::File::create(test_temp_path("pending_invalid_metadata.tmp"))
        .unwrap()
        .into_raw_fd();
    mic_spoofing_set_pending_source_fd(temp_fd, 0, 1);
    PENDING_SOURCE_FD.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "invalid metadata should clear pending state"
        );
    });
    assert!(
        dup_fd(temp_fd).is_none(),
        "invalid metadata should drop ownership of the fd"
    );
}

#[test]
fn test_start_streaming_decoder_rejects_null_output_pointers() {
    let _factory_lock = lock_test_decoder_factory();
    let _factory_guard = set_test_decoder_factory(None);

    // SAFETY: passing null pointers is intentional here to validate FFI guards
    let result = unsafe {
        mic_spoofing_start_streaming_decoder(1234, std::ptr::null_mut(), std::ptr::null_mut())
    };
    assert_eq!(result, -1);
}

#[test]
fn test_start_streaming_decoder_without_factory_returns_failure() {
    let _factory_lock = lock_test_decoder_factory();
    let _factory_guard = set_test_decoder_factory(None);

    let mut sample_rate = 77u32;
    let mut channel_count = 88u32;
    // SAFETY: output pointers are valid for the duration of this call
    let result = unsafe {
        mic_spoofing_start_streaming_decoder(
            current_process_uid(),
            &mut sample_rate,
            &mut channel_count,
        )
    };
    assert_eq!(result, -1);
    assert_eq!(sample_rate, 0);
    assert_eq!(channel_count, 0);
}

#[test]
fn test_start_streaming_decoder_with_factory_and_fallback_prefers_custom_source() {
    let _factory_lock = lock_test_decoder_factory();
    let custom_path = test_temp_path("fallback_custom.bin");
    let custom_bytes = [0x10u8, 0x20, 0x30, 0x40];
    std::fs::write(&custom_path, custom_bytes).unwrap();

    configure_test_factory(TestFactoryConfig {
        output_samples: vec![0.25, -0.5],
        sample_rate: 8_000,
        channel_count: 2,
        capture_prefix_len: custom_bytes.len(),
        fail_on_prefix: None,
    });

    let (pipe_read, sample_rate, channel_count) =
        start_streaming_decoder_with_factory_and_fallback(
            test_decoder_factory,
            current_process_uid(),
            "/nonexistent/default.wav",
            |_| Ok(Some(std::fs::File::open(&custom_path).unwrap())),
        )
        .unwrap();

    assert_eq!(sample_rate, 8_000);
    assert_eq!(channel_count, 2);
    assert_eq!(take_captured_source_bytes(), custom_bytes);
    assert_approx_slice(&read_f32_from_fd(&pipe_read, 2), &[0.25, -0.5]);

    drop(pipe_read);
    std::fs::remove_file(&custom_path).ok();
}

#[test]
fn test_start_streaming_decoder_with_factory_and_fallback_uses_default_on_custom_failure() {
    let _factory_lock = lock_test_decoder_factory();
    let default_path = test_temp_path("fallback_default.bin");
    let default_bytes = [0xAAu8, 0xBB, 0xCC, 0xDD, 0xEE];
    std::fs::write(&default_path, default_bytes).unwrap();

    configure_test_factory(TestFactoryConfig {
        output_samples: vec![0.125],
        sample_rate: 16_000,
        channel_count: 1,
        capture_prefix_len: default_bytes.len(),
        fail_on_prefix: None,
    });

    let (pipe_read, sample_rate, channel_count) =
        start_streaming_decoder_with_factory_and_fallback(
            test_decoder_factory,
            current_process_uid(),
            &default_path,
            |_| Err("custom source lookup failed".to_string()),
        )
        .unwrap();

    assert_eq!(sample_rate, 16_000);
    assert_eq!(channel_count, 1);
    assert_eq!(take_captured_source_bytes(), default_bytes);
    assert_approx_slice(&read_f32_from_fd(&pipe_read, 1), &[0.125]);

    drop(pipe_read);
    std::fs::remove_file(&default_path).ok();
}

#[test]
fn test_start_streaming_decoder_with_factory_and_fallback_retries_default_after_custom_decode_failure(
) {
    let _factory_lock = lock_test_decoder_factory();
    let custom_path = test_temp_path("fallback_custom_decode_failure.bin");
    let default_path = test_temp_path("fallback_custom_decode_default.bin");
    let custom_bytes = [0xDEu8, 0xAD, 0xBE, 0xEF];
    let default_bytes = [0x11u8, 0x22, 0x33, 0x44];
    std::fs::write(&custom_path, custom_bytes).unwrap();
    std::fs::write(&default_path, default_bytes).unwrap();

    configure_test_factory(TestFactoryConfig {
        output_samples: vec![-0.25, 0.5],
        sample_rate: 22_050,
        channel_count: 1,
        capture_prefix_len: custom_bytes.len().max(default_bytes.len()),
        fail_on_prefix: Some(custom_bytes.to_vec()),
    });

    let (pipe_read, sample_rate, channel_count) =
        start_streaming_decoder_with_factory_and_fallback(
            test_decoder_factory,
            current_process_uid(),
            &default_path,
            |_| Ok(Some(std::fs::File::open(&custom_path).unwrap())),
        )
        .unwrap();

    assert_eq!(sample_rate, 22_050);
    assert_eq!(channel_count, 1);
    assert_eq!(
        take_captured_source_calls(),
        vec![custom_bytes.to_vec(), default_bytes.to_vec()]
    );
    assert_approx_slice(&read_f32_from_fd(&pipe_read, 2), &[-0.25, 0.5]);

    drop(pipe_read);
    std::fs::remove_file(&custom_path).ok();
    std::fs::remove_file(&default_path).ok();
}
