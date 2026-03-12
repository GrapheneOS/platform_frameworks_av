use super::*;
use std::ffi::c_void;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

unsafe extern "C" {
    fn poll(fds: *mut PollFd, nfds: usize, timeout: i32) -> i32;
    fn read(fd: i32, buf: *mut c_void, count: usize) -> isize;
}

static DECODER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
const POLLIN: i16 = 0x0001;
const READ_TIMEOUT_MS: i32 = 2_000;

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

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

fn read_f32_from_fd(fd: &OwnedFd, sample_count: usize) -> Vec<f32> {
    let mut bytes = vec![0u8; sample_count * std::mem::size_of::<f32>()];
    let mut read_total = 0usize;
    while read_total < bytes.len() {
        let mut poll_fd = PollFd {
            fd: fd.as_raw_fd(),
            events: POLLIN,
            revents: 0,
        };
        // SAFETY: `poll_fd` points to one initialized pollfd entry for the live descriptor
        let poll_result = unsafe { poll(&mut poll_fd, 1, READ_TIMEOUT_MS) };
        assert!(
            poll_result >= 0,
            "poll failed: {}",
            std::io::Error::last_os_error()
        );
        assert!(
            poll_result != 0,
            "timed out waiting for decoder output after reading {} of {} bytes",
            read_total,
            bytes.len()
        );

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

fn sine_pcm16_samples(frame_count: usize) -> Vec<i16> {
    (0..frame_count)
        .map(|index| {
            let phase = (index as f32 / 48_000.0) * std::f32::consts::PI * 440.0 * 2.0;
            (phase.sin() * i16::MAX as f32) as i16
        })
        .collect()
}

fn normalized_pcm16_samples(samples: &[i16]) -> Vec<f32> {
    samples
        .iter()
        .map(|sample| *sample as f32 / 32768.0)
        .collect()
}

fn write_source_file(path: &str, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
}

fn write_pcm16_mono_wav(path: &str, sample_rate: u32, samples: &[i16]) {
    let data_size = std::mem::size_of_val(samples) as u32;
    let byte_rate = sample_rate * std::mem::size_of::<i16>() as u32;
    let block_align = std::mem::size_of::<i16>() as u16;

    let mut bytes = Vec::with_capacity(44 + data_size as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_size).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_size.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }

    write_source_file(path, &bytes);
}

#[test]
fn test_start_decoder_thread_rejects_zero_length_source() {
    let _test_lock = DECODER_TEST_LOCK.lock().unwrap();
    decoder::clear_observed_decoder_thread_exits();
    let path = test_temp_path("decoder_zero_length.wav");
    std::fs::write(&path, []).unwrap();
    let file = std::fs::File::open(&path).unwrap();
    let result = decoder::start_decoder_thread(file.into());
    assert!(result.is_err());
    std::fs::remove_file(&path).ok();
}

#[test]
fn test_start_decoder_thread_decodes_wav_source() {
    let _test_lock = DECODER_TEST_LOCK.lock().unwrap();
    decoder::clear_observed_decoder_thread_exits();
    let path = test_temp_path("decoder_valid.wav");
    let samples = sine_pcm16_samples(2048);
    write_pcm16_mono_wav(&path, 48_000, &samples);

    let file = std::fs::File::open(&path).unwrap();
    let (pipe_read, sample_rate, channel_count) =
        decoder::start_decoder_thread(file.into()).unwrap();
    assert_eq!(sample_rate, 48_000);
    assert_eq!(channel_count, 1);

    let decoded = read_f32_from_fd(&pipe_read, 256);
    assert_eq!(decoded.len(), 256);
    assert!(decoded.iter().any(|sample| sample.abs() > 0.01));

    drop(pipe_read);
    assert_eq!(
        decoder::wait_for_observed_decoder_thread_exit(Duration::from_secs(2)),
        Some(decoder::ObservedDecoderThreadExit::ConsumerClosed)
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn test_decoder_start_rejects_invalid_inputs() {
    let _test_lock = DECODER_TEST_LOCK.lock().unwrap();
    decoder::clear_observed_decoder_thread_exits();
    let mut sample_rate = 1u32;
    let mut channel_count = 1u32;
    // SAFETY: invalid inputs are intentional here to validate FFI guards
    let result = unsafe { mic_spoofing_decoder_start(-1, &mut sample_rate, &mut channel_count) };
    assert_eq!(result, -1);
    assert_eq!(sample_rate, 1);
    assert_eq!(channel_count, 1);

    let path = test_temp_path("decoder_null_outputs.wav");
    write_pcm16_mono_wav(&path, 48_000, &sine_pcm16_samples(64));
    let file = std::fs::File::open(&path).unwrap();
    let raw_fd = file.into_raw_fd();
    // SAFETY: null out params are intentional to validate FFI guards
    let result =
        unsafe { mic_spoofing_decoder_start(raw_fd, std::ptr::null_mut(), std::ptr::null_mut()) };
    assert_eq!(result, -1);
    std::fs::remove_file(&path).ok();
}

#[test]
fn test_decoder_start_decodes_wav_source() {
    let _test_lock = DECODER_TEST_LOCK.lock().unwrap();
    decoder::clear_observed_decoder_thread_exits();
    let path = test_temp_path("decoder_abi_valid.wav");
    write_pcm16_mono_wav(&path, 16_000, &sine_pcm16_samples(1024));

    let file = std::fs::File::open(&path).unwrap();
    let raw_fd = file.into_raw_fd();
    let mut sample_rate = 0u32;
    let mut channel_count = 0u32;
    // SAFETY: output pointers are valid for the duration of this call
    let pipe_fd =
        unsafe { mic_spoofing_decoder_start(raw_fd, &mut sample_rate, &mut channel_count) };
    assert!(pipe_fd >= 0);
    assert_eq!(sample_rate, 16_000);
    assert_eq!(channel_count, 1);

    // SAFETY: `mic_spoofing_decoder_start` returns a fresh owned fd on success
    let pipe_read = unsafe { OwnedFd::from_raw_fd(pipe_fd) };
    let decoded = read_f32_from_fd(&pipe_read, 128);
    assert!(decoded.iter().any(|sample| sample.abs() > 0.01));

    drop(pipe_read);
    assert_eq!(
        decoder::wait_for_observed_decoder_thread_exit(Duration::from_secs(2)),
        Some(decoder::ObservedDecoderThreadExit::ConsumerClosed)
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn test_start_decoder_thread_loops_short_source() {
    let _test_lock = DECODER_TEST_LOCK.lock().unwrap();
    decoder::clear_observed_decoder_thread_exits();
    let path = test_temp_path("decoder_looping.wav");
    let samples = [0i16, 8_192, -8_192, 16_384, -16_384, 4_096, -4_096, 0];
    write_pcm16_mono_wav(&path, 48_000, &samples);

    let file = std::fs::File::open(&path).unwrap();
    let (pipe_read, sample_rate, channel_count) =
        decoder::start_decoder_thread(file.into()).unwrap();
    assert_eq!(sample_rate, 48_000);
    assert_eq!(channel_count, 1);

    let decoded = read_f32_from_fd(&pipe_read, samples.len() * 3);
    assert_eq!(decoded.len(), samples.len() * 3);

    let expected = normalized_pcm16_samples(&samples);
    for (index, sample) in decoded.iter().enumerate() {
        let expected_sample = expected[index % expected.len()];
        assert!(
            (sample - expected_sample).abs() < 0.0001,
            "looped sample mismatch at index {}: {} vs {}",
            index,
            sample,
            expected_sample
        );
    }

    drop(pipe_read);
    assert_eq!(
        decoder::wait_for_observed_decoder_thread_exit(Duration::from_secs(2)),
        Some(decoder::ObservedDecoderThreadExit::ConsumerClosed)
    );
    std::fs::remove_file(&path).ok();
}
