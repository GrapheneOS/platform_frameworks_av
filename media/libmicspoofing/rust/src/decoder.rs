use std::ffi::{c_char, c_void};
use std::fs::File;
use std::io::ErrorKind;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use crate::logging::{android_log, ANDROID_LOG_DEBUG, ANDROID_LOG_ERROR};
use crate::media_ndk::{
    key_channel_count, key_mime, key_pcm_encoding, key_sample_rate, BufferInfo, MediaCodec,
    MediaExtractor, MediaFormat, OutputBufferResult, BUFFER_FLAG_END_OF_STREAM,
    SAMPLE_FLAG_ENCRYPTED,
};

const CODEC_TIMEOUT_US: i64 = 10_000;
const O_CLOEXEC: i32 = 0o2000000;

const PCM_ENCODING_PCM_16BIT: i32 = 2;
const PCM_ENCODING_PCM_8BIT: i32 = 3;
const PCM_ENCODING_PCM_FLOAT: i32 = 4;
const PCM_ENCODING_PCM_24BIT_PACKED: i32 = 21;
const PCM_ENCODING_PCM_32BIT: i32 = 22;

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ObservedDecoderThreadExit {
    Completed,
    ConsumerClosed,
    Fatal(String),
    Panicked,
}

#[cfg(test)]
static OBSERVED_DECODER_THREAD_EXITS: std::sync::Mutex<Vec<ObservedDecoderThreadExit>> =
    std::sync::Mutex::new(Vec::new());

unsafe extern "C" {
    fn dup(fd: i32) -> i32;
    fn pipe2(pipefd: *mut i32, flags: i32) -> i32;
    fn write(fd: i32, buf: *const c_void, count: usize) -> isize;
}

pub(crate) fn start_decoder_thread(source_fd: OwnedFd) -> Result<(OwnedFd, u32, u32), String> {
    let source_len = file_length_bytes(&source_fd)?;
    if source_len == 0 {
        return Err("audio source file is empty".to_string());
    }

    let mut extractor = MediaExtractor::new()?;
    extractor.set_data_source_fd(source_fd.as_raw_fd(), 0, source_len as i64)?;
    if let Some(file_format) = extractor.file_format() {
        android_log(
            ANDROID_LOG_DEBUG,
            &format!(
                "streaming decoder file format: {}",
                file_format.description()
            ),
        );
    }

    let selected_track = select_audio_track(&extractor)?;
    android_log(
        ANDROID_LOG_DEBUG,
        &format!(
            "streaming decoder selected track={} mime={} format={}",
            selected_track.index, selected_track.mime, selected_track.description
        ),
    );

    extractor.select_track(selected_track.index)?;

    let mut codec = MediaCodec::create_decoder_by_type(&selected_track.mime)?;
    codec.configure(&selected_track.format)?;
    codec.start()?;

    let (pipe_read, pipe_write) = create_pipe()?;
    let expected_sample_rate = selected_track.sample_rate;
    let expected_channel_count = selected_track.channel_count;
    if selected_track.mime == "audio/raw" {
        let output_format = parse_output_format(&selected_track.format)?;
        if output_format.sample_rate != expected_sample_rate
            || output_format.channel_count != expected_channel_count
        {
            return Err(format!(
                "raw extractor output format changed to {} Hz / {} channels, expected {} Hz / {} channels",
                output_format.sample_rate,
                output_format.channel_count,
                expected_sample_rate,
                expected_channel_count
            ));
        }

        std::thread::Builder::new()
            .name("mic_spoofing_decoder".to_string())
            .spawn(move || {
                let result = std::panic::catch_unwind(move || {
                    run_raw_extractor_loop(extractor, pipe_write, output_format)
                });

                match result {
                    Ok(Ok(())) => {
                        #[cfg(test)]
                        record_observed_decoder_thread_exit(ObservedDecoderThreadExit::Completed);
                    }
                    Ok(Err(DecoderThreadExit::ConsumerClosed)) => {
                        android_log(ANDROID_LOG_DEBUG, "streaming decoder consumer closed pipe");
                        #[cfg(test)]
                        record_observed_decoder_thread_exit(
                            ObservedDecoderThreadExit::ConsumerClosed,
                        );
                    }
                    Ok(Err(DecoderThreadExit::Fatal(message))) => {
                        android_log(
                            ANDROID_LOG_ERROR,
                            &format!("streaming decoder thread failed: {message}"),
                        );
                        #[cfg(test)]
                        record_observed_decoder_thread_exit(ObservedDecoderThreadExit::Fatal(
                            message,
                        ));
                    }
                    Err(_) => {
                        android_log(ANDROID_LOG_ERROR, "streaming decoder thread panicked");
                        #[cfg(test)]
                        record_observed_decoder_thread_exit(ObservedDecoderThreadExit::Panicked);
                    }
                }
            })
            .map_err(|e| format!("failed to spawn decoder thread: {e}"))?;

        return Ok((pipe_read, expected_sample_rate, expected_channel_count));
    }

    std::thread::Builder::new()
        .name("mic_spoofing_decoder".to_string())
        .spawn(move || {
            let result = std::panic::catch_unwind(move || {
                run_decoder_loop(
                    extractor,
                    codec,
                    pipe_write,
                    expected_sample_rate,
                    expected_channel_count,
                )
            });

            match result {
                Ok(Ok(())) => {
                    #[cfg(test)]
                    record_observed_decoder_thread_exit(ObservedDecoderThreadExit::Completed);
                }
                Ok(Err(DecoderThreadExit::ConsumerClosed)) => {
                    android_log(ANDROID_LOG_DEBUG, "streaming decoder consumer closed pipe");
                    #[cfg(test)]
                    record_observed_decoder_thread_exit(ObservedDecoderThreadExit::ConsumerClosed);
                }
                Ok(Err(DecoderThreadExit::Fatal(message))) => {
                    android_log(
                        ANDROID_LOG_ERROR,
                        &format!("streaming decoder thread failed: {message}"),
                    );
                    #[cfg(test)]
                    record_observed_decoder_thread_exit(ObservedDecoderThreadExit::Fatal(message));
                }
                Err(_) => {
                    android_log(ANDROID_LOG_ERROR, "streaming decoder thread panicked");
                    #[cfg(test)]
                    record_observed_decoder_thread_exit(ObservedDecoderThreadExit::Panicked);
                }
            }
        })
        .map_err(|e| format!("failed to spawn decoder thread: {e}"))?;

    Ok((pipe_read, expected_sample_rate, expected_channel_count))
}

#[cfg(test)]
fn record_observed_decoder_thread_exit(exit: ObservedDecoderThreadExit) {
    OBSERVED_DECODER_THREAD_EXITS.lock().unwrap().push(exit);
}

#[cfg(test)]
pub(crate) fn clear_observed_decoder_thread_exits() {
    OBSERVED_DECODER_THREAD_EXITS.lock().unwrap().clear();
}

#[cfg(test)]
pub(crate) fn wait_for_observed_decoder_thread_exit(
    timeout: std::time::Duration,
) -> Option<ObservedDecoderThreadExit> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(exit) = OBSERVED_DECODER_THREAD_EXITS.lock().unwrap().pop() {
            return Some(exit);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

struct SelectedAudioTrack {
    index: usize,
    mime: String,
    sample_rate: u32,
    channel_count: u32,
    description: String,
    format: MediaFormat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OutputAudioFormat {
    sample_rate: u32,
    channel_count: u32,
    pcm_encoding: i32,
}

enum DecoderThreadExit {
    ConsumerClosed,
    Fatal(String),
}

fn file_length_bytes(source_fd: &OwnedFd) -> Result<u64, String> {
    let file = duplicate_as_file(source_fd)?;
    file.metadata()
        .map_err(|e| format!("failed to stat source fd: {e}"))
        .map(|metadata| metadata.len())
}

fn duplicate_as_file(source_fd: &OwnedFd) -> Result<File, String> {
    // SAFETY: `dup` creates a new owned descriptor referring to the same underlying file
    let duplicated = unsafe { dup(source_fd.as_raw_fd()) };
    if duplicated < 0 {
        return Err(format!(
            "dup failed while cloning source fd: {}",
            std::io::Error::last_os_error()
        ));
    }

    // SAFETY: `dup` returned a fresh owned descriptor
    Ok(File::from(unsafe { OwnedFd::from_raw_fd(duplicated) }))
}

fn create_pipe() -> Result<(OwnedFd, OwnedFd), String> {
    let mut fds = [-1i32; 2];
    // SAFETY: `fds` points to two writable integers for `pipe2`
    let result = unsafe { pipe2(fds.as_mut_ptr(), O_CLOEXEC) };
    if result != 0 {
        return Err(format!("pipe2 failed: {}", std::io::Error::last_os_error()));
    }

    // SAFETY: `pipe2` initialized both entries with owned descriptors on success
    let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    // SAFETY: `pipe2` initialized both entries with owned descriptors on success
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    Ok((read_fd, write_fd))
}

fn select_audio_track(extractor: &MediaExtractor) -> Result<SelectedAudioTrack, String> {
    for index in 0..extractor.track_count() {
        let format = extractor.track_format(index)?;
        let Some(mime) = format.get_string(key_mime()) else {
            continue;
        };
        if !mime.starts_with("audio/") {
            continue;
        }

        let sample_rate = parse_positive_u32(&format, key_sample_rate(), "sample rate")?;
        let channel_count = parse_positive_u32(&format, key_channel_count(), "channel count")?;
        return Ok(SelectedAudioTrack {
            index,
            mime,
            sample_rate,
            channel_count,
            description: format.description(),
            format,
        });
    }

    Err("audio source contains no audio track".to_string())
}

fn parse_positive_u32(format: &MediaFormat, key: *const c_char, name: &str) -> Result<u32, String> {
    let value = format
        .get_i32(key)
        .ok_or_else(|| format!("audio track is missing {name}"))?;
    if value <= 0 {
        return Err(format!("audio track has invalid {name}: {value}"));
    }
    Ok(value as u32)
}

fn parse_output_format(format: &MediaFormat) -> Result<OutputAudioFormat, String> {
    let sample_rate = parse_positive_u32(format, key_sample_rate(), "output sample rate")?;
    let channel_count = parse_positive_u32(format, key_channel_count(), "output channel count")?;
    let pcm_encoding = format
        .get_i32(key_pcm_encoding())
        .unwrap_or(PCM_ENCODING_PCM_16BIT);

    match pcm_encoding {
        PCM_ENCODING_PCM_8BIT
        | PCM_ENCODING_PCM_16BIT
        | PCM_ENCODING_PCM_FLOAT
        | PCM_ENCODING_PCM_24BIT_PACKED
        | PCM_ENCODING_PCM_32BIT => {}
        other => {
            return Err(format!("unsupported decoder PCM encoding {other}"));
        }
    }

    Ok(OutputAudioFormat {
        sample_rate,
        channel_count,
        pcm_encoding,
    })
}

fn run_decoder_loop(
    mut extractor: MediaExtractor,
    mut codec: MediaCodec,
    pipe_write: OwnedFd,
    expected_sample_rate: u32,
    expected_channel_count: u32,
) -> Result<(), DecoderThreadExit> {
    let mut input_eos = false;
    let mut output_eos = false;
    let mut saw_output_samples = false;
    let mut output_format: Option<OutputAudioFormat> = None;
    let mut last_queued_sample_time_us: Option<i64> = None;

    loop {
        if !input_eos {
            queue_next_input_sample(
                &mut extractor,
                &mut codec,
                &mut input_eos,
                &mut last_queued_sample_time_us,
            )
            .map_err(DecoderThreadExit::Fatal)?;
        }

        match codec
            .dequeue_output_buffer(CODEC_TIMEOUT_US)
            .map_err(DecoderThreadExit::Fatal)?
        {
            OutputBufferResult::TryAgainLater | OutputBufferResult::OutputBuffersChanged => {}
            OutputBufferResult::OutputFormatChanged => {
                let format = codec.output_format().map_err(DecoderThreadExit::Fatal)?;
                let parsed = parse_output_format(&format).map_err(DecoderThreadExit::Fatal)?;
                if parsed.sample_rate != expected_sample_rate
                    || parsed.channel_count != expected_channel_count
                {
                    return Err(DecoderThreadExit::Fatal(format!(
                        "decoder output format changed to {} Hz / {} channels, expected {} Hz / {} channels",
                        parsed.sample_rate,
                        parsed.channel_count,
                        expected_sample_rate,
                        expected_channel_count
                    )));
                }
                android_log(
                    ANDROID_LOG_DEBUG,
                    &format!("streaming decoder output format: {}", format.description()),
                );
                output_format = Some(parsed);
            }
            OutputBufferResult::Buffer { index, info } => {
                let buffer_result = handle_output_buffer(
                    &mut codec,
                    index,
                    &info,
                    &mut output_format,
                    &pipe_write,
                    expected_sample_rate,
                    expected_channel_count,
                );
                if info.size > 0 {
                    saw_output_samples = true;
                }
                output_eos |= info.flags & BUFFER_FLAG_END_OF_STREAM != 0;

                codec
                    .release_output_buffer(index)
                    .map_err(DecoderThreadExit::Fatal)?;
                match buffer_result {
                    Ok(()) => {}
                    Err(DecoderThreadExit::ConsumerClosed) => {
                        return Err(DecoderThreadExit::ConsumerClosed);
                    }
                    Err(DecoderThreadExit::Fatal(message)) => {
                        return Err(DecoderThreadExit::Fatal(message));
                    }
                }
            }
        }

        if input_eos && output_eos {
            if !saw_output_samples {
                return Err(DecoderThreadExit::Fatal(
                    "decoded source produced no audio samples".to_string(),
                ));
            }

            extractor
                .seek_to_start()
                .map_err(DecoderThreadExit::Fatal)?;
            codec.flush().map_err(DecoderThreadExit::Fatal)?;
            input_eos = false;
            output_eos = false;
            last_queued_sample_time_us = None;
        }
    }
}

fn run_raw_extractor_loop(
    mut extractor: MediaExtractor,
    pipe_write: OwnedFd,
    output_format: OutputAudioFormat,
) -> Result<(), DecoderThreadExit> {
    let mut input_buffer = Vec::new();
    let mut saw_output_samples = false;

    loop {
        let sample_size = extractor.sample_size();
        if sample_size < 0 {
            if !saw_output_samples {
                return Err(DecoderThreadExit::Fatal(
                    "decoded source produced no audio samples".to_string(),
                ));
            }

            extractor
                .seek_to_start()
                .map_err(DecoderThreadExit::Fatal)?;
            continue;
        }

        let sample_flags = extractor.sample_flags();
        if sample_flags & SAMPLE_FLAG_ENCRYPTED != 0 {
            return Err(DecoderThreadExit::Fatal(
                "audio source contains encrypted samples".to_string(),
            ));
        }

        let sample_size = sample_size as usize;
        input_buffer.resize(sample_size, 0);
        let bytes_read = extractor
            .read_sample_data(&mut input_buffer[..sample_size])
            .map_err(DecoderThreadExit::Fatal)?;
        if bytes_read != sample_size {
            return Err(DecoderThreadExit::Fatal(format!(
                "extractor read {bytes_read} bytes but expected {sample_size}"
            )));
        }

        let samples = normalize_pcm_buffer_to_f32(&input_buffer[..bytes_read], output_format.pcm_encoding)
            .map_err(DecoderThreadExit::Fatal)?;
        if !samples.is_empty() {
            saw_output_samples = true;
            write_f32_samples(&pipe_write, &samples)?;
        }

        let _ = extractor.advance();
    }
}

fn queue_next_input_sample(
    extractor: &mut MediaExtractor,
    codec: &mut MediaCodec,
    input_eos: &mut bool,
    last_queued_sample_time_us: &mut Option<i64>,
) -> Result<(), String> {
    let Some(index) = codec.dequeue_input_buffer(CODEC_TIMEOUT_US)? else {
        return Ok(());
    };

    let sample_size = extractor.sample_size();
    if sample_size < 0 {
        codec.queue_input_buffer(index, 0, 0, BUFFER_FLAG_END_OF_STREAM)?;
        *input_eos = true;
        return Ok(());
    }

    let sample_flags = extractor.sample_flags();
    if sample_flags & SAMPLE_FLAG_ENCRYPTED != 0 {
        return Err("audio source contains encrypted samples".to_string());
    }

    let sample_time_us =
        normalize_sample_time_us(extractor.sample_time_us(), *last_queued_sample_time_us);

    let sample_size = sample_size as usize;
    let input_buffer = codec.input_buffer(index)?;
    if sample_size > input_buffer.len() {
        return Err(format!(
            "extractor sample size {sample_size} exceeds codec input buffer {}",
            input_buffer.len()
        ));
    }

    let bytes_read = extractor.read_sample_data(&mut input_buffer[..sample_size])?;
    if bytes_read != sample_size {
        return Err(format!(
            "extractor read {bytes_read} bytes but expected {sample_size}"
        ));
    }

    codec.queue_input_buffer(index, bytes_read, sample_time_us, 0)?;
    *last_queued_sample_time_us = Some(sample_time_us);
    let _ = extractor.advance();
    Ok(())
}

fn normalize_sample_time_us(sample_time_us: i64, last_queued_sample_time_us: Option<i64>) -> i64 {
    let Some(last_queued_sample_time_us) = last_queued_sample_time_us else {
        return sample_time_us.max(0);
    };

    if sample_time_us <= last_queued_sample_time_us {
        last_queued_sample_time_us.saturating_add(1)
    } else {
        sample_time_us
    }
}

fn handle_output_buffer(
    codec: &mut MediaCodec,
    index: usize,
    info: &BufferInfo,
    output_format: &mut Option<OutputAudioFormat>,
    pipe_write: &OwnedFd,
    expected_sample_rate: u32,
    expected_channel_count: u32,
) -> Result<(), DecoderThreadExit> {
    if info.size == 0 {
        return Ok(());
    }

    let format = if let Some(format) = output_format {
        *format
    } else {
        let current_format = codec.output_format().map_err(DecoderThreadExit::Fatal)?;
        let parsed = parse_output_format(&current_format).map_err(DecoderThreadExit::Fatal)?;
        if parsed.sample_rate != expected_sample_rate
            || parsed.channel_count != expected_channel_count
        {
            return Err(DecoderThreadExit::Fatal(format!(
                "decoder output format changed to {} Hz / {} channels, expected {} Hz / {} channels",
                parsed.sample_rate,
                parsed.channel_count,
                expected_sample_rate,
                expected_channel_count
            )));
        }
        *output_format = Some(parsed);
        parsed
    };

    let bytes = codec
        .output_buffer(index, info)
        .map_err(DecoderThreadExit::Fatal)?;

    let samples = normalize_pcm_buffer_to_f32(bytes, format.pcm_encoding)
        .map_err(DecoderThreadExit::Fatal)?;

    if samples.is_empty() {
        return Ok(());
    }

    write_f32_samples(pipe_write, &samples)
}

fn write_f32_samples(pipe_write: &OwnedFd, samples: &[f32]) -> Result<(), DecoderThreadExit> {
    if samples.is_empty() {
        return Ok(());
    }

    // SAFETY: `samples` is a live contiguous f32 slice and the byte view covers the same memory
    let bytes = unsafe {
        std::slice::from_raw_parts(
            samples.as_ptr().cast::<u8>(),
            std::mem::size_of_val(samples),
        )
    };

    let mut written = 0usize;
    while written < bytes.len() {
        // SAFETY: `pipe_write` is a live owned descriptor and `bytes` points to readable memory
        let result = unsafe {
            write(
                pipe_write.as_raw_fd(),
                bytes[written..].as_ptr().cast(),
                bytes.len() - written,
            )
        };
        if result < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == ErrorKind::Interrupted {
                continue;
            }
            if err.raw_os_error() == Some(32) {
                return Err(DecoderThreadExit::ConsumerClosed);
            }
            return Err(DecoderThreadExit::Fatal(format!(
                "pipe write failed: {err}"
            )));
        }

        written += result as usize;
    }

    Ok(())
}

pub(crate) fn normalize_pcm_buffer_to_f32(
    bytes: &[u8],
    pcm_encoding: i32,
) -> Result<Vec<f32>, String> {
    match pcm_encoding {
        PCM_ENCODING_PCM_8BIT => Ok(bytes
            .iter()
            .map(|sample| ((*sample as f32) - 128.0) / 128.0)
            .map(clamp_float_sample)
            .collect()),
        PCM_ENCODING_PCM_16BIT => bytes
            .chunks_exact(2)
            .map(|chunk| i16::from_ne_bytes([chunk[0], chunk[1]]) as f32 / 32768.0)
            .map(clamp_float_sample)
            .collect::<Vec<_>>()
            .pipe_if_remainder(
                bytes.len() % 2 == 0,
                "PCM 16-bit buffer length is not divisible by 2",
            ),
        PCM_ENCODING_PCM_FLOAT => bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .map(clamp_float_sample)
            .collect::<Vec<_>>()
            .pipe_if_remainder(
                bytes.len() % 4 == 0,
                "PCM float buffer length is not divisible by 4",
            ),
        PCM_ENCODING_PCM_24BIT_PACKED => bytes
            .chunks_exact(3)
            .map(|chunk| {
                let value =
                    (chunk[0] as i32) | ((chunk[1] as i32) << 8) | ((chunk[2] as i32) << 16);
                let signed = if value & 0x0080_0000 != 0 {
                    value | !0x00FF_FFFF
                } else {
                    value
                };
                signed as f32 / 8_388_608.0
            })
            .map(clamp_float_sample)
            .collect::<Vec<_>>()
            .pipe_if_remainder(
                bytes.len() % 3 == 0,
                "PCM 24-bit buffer length is not divisible by 3",
            ),
        PCM_ENCODING_PCM_32BIT => bytes
            .chunks_exact(4)
            .map(|chunk| {
                i32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as f32
                    / 2_147_483_648.0
            })
            .map(clamp_float_sample)
            .collect::<Vec<_>>()
            .pipe_if_remainder(
                bytes.len() % 4 == 0,
                "PCM 32-bit buffer length is not divisible by 4",
            ),
        other => Err(format!("unsupported decoder PCM encoding {other}")),
    }
}

fn clamp_float_sample(sample: f32) -> f32 {
    if !sample.is_finite() {
        0.0
    } else {
        sample.clamp(-1.0, 1.0)
    }
}

trait CollectWithRemainderCheck {
    fn pipe_if_remainder(self, is_aligned: bool, message: &str) -> Result<Vec<f32>, String>;
}

impl CollectWithRemainderCheck for Vec<f32> {
    fn pipe_if_remainder(self, is_aligned: bool, message: &str) -> Result<Vec<f32>, String> {
        if is_aligned {
            Ok(self)
        } else {
            Err(message.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_pcm_8bit() {
        let samples = normalize_pcm_buffer_to_f32(&[0, 128, 255], PCM_ENCODING_PCM_8BIT).unwrap();
        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0], -1.0);
        assert_eq!(samples[1], 0.0);
        assert!(samples[2] > 0.99);
    }

    #[test]
    fn test_normalize_pcm_16bit() {
        let bytes = [0x00, 0x80, 0x00, 0x00, 0xff, 0x7f];
        let samples = normalize_pcm_buffer_to_f32(&bytes, PCM_ENCODING_PCM_16BIT).unwrap();
        assert_eq!(samples, vec![-1.0, 0.0, 32767.0 / 32768.0]);
    }

    #[test]
    fn test_normalize_pcm_24bit() {
        let bytes = [0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0xff, 0xff, 0x7f];
        let samples = normalize_pcm_buffer_to_f32(&bytes, PCM_ENCODING_PCM_24BIT_PACKED).unwrap();
        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0], -1.0);
        assert_eq!(samples[1], 0.0);
        assert!(samples[2] > 0.99);
    }

    #[test]
    fn test_normalize_pcm_32bit() {
        let bytes = [
            0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0x7f,
        ];
        let samples = normalize_pcm_buffer_to_f32(&bytes, PCM_ENCODING_PCM_32BIT).unwrap();
        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0], -1.0);
        assert_eq!(samples[1], 0.0);
        assert!(samples[2] > 0.99);
    }

    #[test]
    fn test_normalize_pcm_float_clamps_non_finite() {
        let bytes = [
            0x00, 0x00, 0x80, 0x3f, // 1.0
            0x00, 0x00, 0x80, 0x7f, // inf
        ];
        let samples = normalize_pcm_buffer_to_f32(&bytes, PCM_ENCODING_PCM_FLOAT).unwrap();
        assert_eq!(samples, vec![1.0, 0.0]);
    }

    #[test]
    fn test_normalize_rejects_partial_samples() {
        let err = normalize_pcm_buffer_to_f32(&[0x00], PCM_ENCODING_PCM_16BIT).unwrap_err();
        assert!(err.contains("divisible by 2"));
    }
}
