use super::*;
use std::f32::consts::PI;
use std::io::Cursor;

fn test_wav_path(name: &str) -> String {
    format!("{}/micspoofing_test_{}", std::env::temp_dir().display(), name)
}

fn create_sine_wav_i16_mono(path: &str, sample_rate: u32, num_frames: usize) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for i in 0..num_frames {
        let t = i as f32 / sample_rate as f32;
        let sample = (t * 440.0 * 2.0 * PI).sin();
        writer.write_sample((sample * i16::MAX as f32) as i16).unwrap();
    }
    writer.finalize().unwrap();
}

fn create_sine_wav_f32_stereo(path: &str, sample_rate: u32, num_frames: usize) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for i in 0..num_frames {
        let t = i as f32 / sample_rate as f32;
        let sample = (t * 440.0 * 2.0 * PI).sin();
        writer.write_sample(sample).unwrap();
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();
}

fn create_silent_wav_i16_mono(path: &str, sample_rate: u32, num_frames: usize) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for _ in 0..num_frames {
        writer.write_sample(0i16).unwrap();
    }
    writer.finalize().unwrap();
}

fn make_source_from_samples(samples: Vec<f32>) -> SpoofedAudioSource {
    SpoofedAudioSource {
        samples,
        source_sample_rate: 48000,
        source_channels: 1,
        position: 0,
    }
}

#[test]
fn test_load_valid_wav_16bit_mono() {
    let path = test_wav_path("load_16bit_mono.wav");
    create_sine_wav_i16_mono(&path, 48000, 4800);

    let src = SpoofedAudioSource::new(&path).unwrap();
    assert_eq!(src.source_sample_rate, 48000);
    assert_eq!(src.source_channels, 1);
    assert_eq!(src.samples.len(), 4800);
    assert_eq!(src.position, 0);
    for &s in &src.samples {
        assert!((-1.0..=1.0).contains(&s), "sample {} out of [-1.0, 1.0]", s);
    }
    assert!(src.samples.iter().any(|&s| s.abs() > 0.01));

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_load_valid_wav_float_stereo() {
    let path = test_wav_path("load_float_stereo.wav");
    create_sine_wav_f32_stereo(&path, 44100, 4410);

    let src = SpoofedAudioSource::new(&path).unwrap();
    assert_eq!(src.source_sample_rate, 44100);
    assert_eq!(src.source_channels, 2);
    assert_eq!(src.samples.len(), 4410 * 2);
    assert_eq!(src.position, 0);
    assert!(src.samples.iter().any(|&s| s.abs() > 0.01));

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_from_reader_valid_wav_in_memory() {
    let path = test_wav_path("from_reader_valid.wav");
    create_sine_wav_i16_mono(&path, 48000, 4800);
    let wav_bytes = std::fs::read(&path).unwrap();

    let src = SpoofedAudioSource::from_reader(Cursor::new(wav_bytes)).unwrap();
    assert_eq!(src.source_sample_rate, 48000);
    assert_eq!(src.source_channels, 1);
    assert_eq!(src.samples.len(), 4800);
    assert!(src.samples.iter().any(|&s| s.abs() > 0.01));

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_from_reader_invalid_data_returns_error() {
    let result = SpoofedAudioSource::from_reader(Cursor::new(b"not a wav".to_vec()));
    assert!(result.is_err(), "expected invalid reader data to fail");
}

#[test]
fn test_from_reader_empty_reader_returns_error() {
    let result = SpoofedAudioSource::from_reader(Cursor::new(Vec::<u8>::new()));
    assert!(result.is_err(), "expected empty reader to fail");
}

#[test]
fn test_load_missing_wav_returns_error() {
    let result = SpoofedAudioSource::new("/nonexistent/path/missing.wav");
    match result {
        Err(e) => assert!(e.contains("Failed to open WAV"), "unexpected error: {}", e),
        Ok(_) => panic!("expected error for missing WAV"),
    }
}

#[test]
fn test_load_invalid_wav_returns_error() {
    let path = test_wav_path("not_a_wav.txt");
    std::fs::write(&path, b"This is not a WAV file").unwrap();

    let result = SpoofedAudioSource::new(&path);
    assert!(result.is_err());

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_load_wav_too_large_truncates_silently() {
    let path = test_wav_path("too_large.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec).unwrap();
    // Write MAX_WAV_SAMPLES + 1 samples of silence
    for _ in 0..=MAX_WAV_SAMPLES {
        writer.write_sample(0i16).unwrap();
    }
    writer.finalize().unwrap();

    let src = SpoofedAudioSource::new(&path).unwrap();
    assert_eq!(
        src.samples.len(),
        MAX_WAV_SAMPLES as usize,
        "oversized WAV should be truncated instead of failing"
    );

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_read_f32_empty_samples_fills_silence() {
    let mut src = SpoofedAudioSource {
        samples: vec![],
        source_sample_rate: 48000,
        source_channels: 1,
        position: 0,
    };

    let mut output = vec![1.0f32; 480]; // pre-fill with non-zero
    let frames = src.read_f32(&mut output, 48000, 1);
    assert_eq!(frames, 480);
    for &s in &output {
        assert_eq!(s, 0.0, "empty source should produce silence");
    }
}

#[test]
fn test_read_f32_output_in_range() {
    let path = test_wav_path("f32_range.wav");
    create_sine_wav_i16_mono(&path, 48000, 4800);
    let mut src = SpoofedAudioSource::new(&path).unwrap();

    let mut output = vec![0.0f32; 480];
    let frames = src.read_f32(&mut output, 48000, 1);
    assert_eq!(frames, 480);
    for &s in &output {
        assert!((-1.0..=1.0).contains(&s), "f32 sample {} out of [-1.0, 1.0]", s);
    }
    assert!(output.iter().any(|&s| s.abs() > 0.01));

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_read_i16_output_in_range() {
    let path = test_wav_path("i16_range.wav");
    create_sine_wav_i16_mono(&path, 48000, 4800);
    let mut src = SpoofedAudioSource::new(&path).unwrap();

    let mut output = vec![0i16; 480];
    let frames = src.read_i16(&mut output, 48000, 1);
    assert_eq!(frames, 480);
    assert!(output.iter().any(|&s| s != 0), "i16 output should be non-silent");

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_read_u8_silence_is_128() {
    // Silent source: all samples should map to 128
    let path = test_wav_path("u8_silence.wav");
    create_silent_wav_i16_mono(&path, 48000, 4800);
    let mut src = SpoofedAudioSource::new(&path).unwrap();

    let mut output = vec![0u8; 480];
    src.read_u8(&mut output, 48000, 1);
    for &s in &output {
        assert_eq!(s, 128, "8-bit PCM silence should be 128, got {}", s);
    }

    std::fs::remove_file(&path).ok();

    // Non-silent source: offset is applied correctly (values != 128)
    let path2 = test_wav_path("u8_nonsilent.wav");
    create_sine_wav_i16_mono(&path2, 48000, 4800);
    let mut src2 = SpoofedAudioSource::new(&path2).unwrap();

    let mut output2 = vec![128u8; 480];
    src2.read_u8(&mut output2, 48000, 1);
    assert!(output2.iter().any(|&s| s != 128), "non-silent source should produce non-128 values");

    std::fs::remove_file(&path2).ok();
}

#[test]
fn test_read_i24_packed() {
    let path = test_wav_path("i24_packed.wav");
    create_sine_wav_i16_mono(&path, 48000, 4800);
    let mut src = SpoofedAudioSource::new(&path).unwrap();

    let num_frames = 480;
    let mut output = vec![0u8; num_frames * 3];
    let frames = src.read_i24_packed(&mut output, 48000, 1);
    assert_eq!(frames, num_frames);
    let has_nonzero = (0..num_frames).any(|i| {
        let b = i * 3;
        output[b] != 0 || output[b + 1] != 0 || output[b + 2] != 0
    });
    assert!(has_nonzero, "24-bit packed output should be non-silent");

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_read_i32_output_in_range() {
    let path = test_wav_path("i32_range.wav");
    create_sine_wav_i16_mono(&path, 48000, 4800);
    let mut src = SpoofedAudioSource::new(&path).unwrap();

    let mut output = vec![0i32; 480];
    let frames = src.read_i32(&mut output, 48000, 1);
    assert_eq!(frames, 480);
    assert!(output.iter().any(|&s| s != 0), "i32 output should be non-silent");

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_resample_8k_to_48k() {
    let path = test_wav_path("resample_8k_to_48k.wav");
    create_sine_wav_i16_mono(&path, 8000, 8000);
    let mut src = SpoofedAudioSource::new(&path).unwrap();

    let num_frames = 4800; // 100 ms
    let mut output = vec![0.0f32; num_frames];
    let frames = src.read_f32(&mut output, 48000, 1);
    assert_eq!(frames, num_frames);
    assert!(output.iter().any(|&s| s.abs() > 0.01), "6x upsampled output should be non-silent");

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_resample_44100_to_48k() {
    let path = test_wav_path("resample_48k.wav");
    create_sine_wav_i16_mono(&path, 44100, 44100);
    let mut src = SpoofedAudioSource::new(&path).unwrap();

    let num_frames = 4800;
    let mut output = vec![0.0f32; num_frames];
    let frames = src.read_f32(&mut output, 48000, 1);
    assert_eq!(frames, num_frames);
    assert!(output.iter().any(|&s| s.abs() > 0.01), "resampled output should be non-silent");

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_resample_44100_to_16k() {
    let path = test_wav_path("resample_16k.wav");
    create_sine_wav_i16_mono(&path, 44100, 44100);
    let mut src = SpoofedAudioSource::new(&path).unwrap();

    let num_frames = 1600;
    let mut output = vec![0.0f32; num_frames];
    let frames = src.read_f32(&mut output, 16000, 1);
    assert_eq!(frames, num_frames);
    assert!(output.iter().any(|&s| s.abs() > 0.01));

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_resample_44100_to_8k() {
    let path = test_wav_path("resample_8k.wav");
    create_sine_wav_i16_mono(&path, 44100, 44100);
    let mut src = SpoofedAudioSource::new(&path).unwrap();

    let num_frames = 800;
    let mut output = vec![0.0f32; num_frames];
    let frames = src.read_f32(&mut output, 8000, 1);
    assert_eq!(frames, num_frames);
    assert!(output.iter().any(|&s| s.abs() > 0.01));

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_mono_source_to_stereo_output() {
    let path = test_wav_path("mono_to_stereo.wav");
    create_sine_wav_i16_mono(&path, 48000, 4800);
    let mut src = SpoofedAudioSource::new(&path).unwrap();

    let num_frames = 480;
    let mut output = vec![0.0f32; num_frames * 2];
    let frames = src.read_f32(&mut output, 48000, 2);
    assert_eq!(frames, num_frames);
    for frame in 0..num_frames {
        let l = output[frame * 2];
        let r = output[frame * 2 + 1];
        assert_eq!(l, r, "mono->stereo: L and R should be identical at frame {}", frame);
    }
    assert!(output.iter().any(|&s| s.abs() > 0.01));

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_stereo_source_to_mono_output() {
    let path = test_wav_path("stereo_to_mono.wav");
    create_sine_wav_f32_stereo(&path, 48000, 4800);

    // Read as stereo to get reference left channel
    let mut src_ref = SpoofedAudioSource::new(&path).unwrap();
    let num_frames = 480;
    let mut stereo_out = vec![0.0f32; num_frames * 2];
    src_ref.read_f32(&mut stereo_out, 48000, 2);

    // Read as mono from a fresh source
    let mut src_mono = SpoofedAudioSource::new(&path).unwrap();
    let mut mono_out = vec![0.0f32; num_frames];
    let frames = src_mono.read_f32(&mut mono_out, 48000, 1);
    assert_eq!(frames, num_frames);
    // Mono output should match left channel of stereo output
    for i in 0..num_frames {
        assert_eq!(
            mono_out[i], stereo_out[i * 2],
            "mono should match left channel at frame {}", i
        );
    }

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_looping_wraps_position() {
    let path = test_wav_path("loop_wrap.wav");
    let source_frames = 480;
    create_sine_wav_i16_mono(&path, 48000, source_frames);
    let mut src = SpoofedAudioSource::new(&path).unwrap();

    // Read more frames than the source contains
    let read_frames = source_frames + 100;
    let mut output = vec![0.0f32; read_frames];
    src.read_f32(&mut output, 48000, 1);
    assert_eq!(src.position, 100, "position should wrap to 100 after reading past end");
    assert!(output.iter().any(|&s| s.abs() > 0.01));

    // Verify the wrapped portion matches a fresh read from position 0
    let mut src2 = SpoofedAudioSource::new(&path).unwrap();
    let mut from_start = vec![0.0f32; source_frames];
    src2.read_f32(&mut from_start, 48000, 1);
    assert_eq!(&output[source_frames..], &from_start[..100]);

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_consecutive_reads_are_continuous() {
    let path = test_wav_path("consecutive.wav");
    create_sine_wav_i16_mono(&path, 48000, 4800);
    let mut src = SpoofedAudioSource::new(&path).unwrap();
    assert_eq!(src.position, 0);

    let mut out1 = vec![0.0f32; 100];
    src.read_f32(&mut out1, 48000, 1);
    assert_eq!(src.position, 100);

    let mut out2 = vec![0.0f32; 100];
    src.read_f32(&mut out2, 48000, 1);
    assert_eq!(src.position, 200);

    // Verify against a single contiguous read from a fresh source
    let mut src2 = SpoofedAudioSource::new(&path).unwrap();
    let mut combined = vec![0.0f32; 200];
    src2.read_f32(&mut combined, 48000, 1);
    assert_eq!(&out1[..], &combined[..100]);
    assert_eq!(&out2[..], &combined[100..200]);

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_zero_length_read() {
    let path = test_wav_path("zero_len.wav");
    create_sine_wav_i16_mono(&path, 48000, 4800);
    let mut src = SpoofedAudioSource::new(&path).unwrap();

    let mut output: Vec<f32> = vec![];
    let frames = src.read_f32(&mut output, 48000, 1);
    assert_eq!(frames, 0);
    assert_eq!(src.position, 0);

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_read_single_frame() {
    let path = test_wav_path("single_frame.wav");
    create_sine_wav_i16_mono(&path, 48000, 4800);
    let mut src = SpoofedAudioSource::new(&path).unwrap();

    let mut output = vec![0.0f32; 1];
    let frames = src.read_f32(&mut output, 48000, 1);
    assert_eq!(frames, 1);
    assert_eq!(src.position, 1);

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_read_f32_zero_source_channels_fills_silence() {
    let mut src = SpoofedAudioSource {
        samples: vec![0.25, -0.25],
        source_sample_rate: 48000,
        source_channels: 0,
        position: 0,
    };

    let mut output = vec![1.0f32; 16];
    let frames = src.read_f32(&mut output, 48000, 1);
    assert_eq!(frames, 16);
    assert!(output.iter().all(|&s| s == 0.0), "invalid source metadata should produce silence");
    assert_eq!(src.position, 0);
}

#[test]
fn test_load_truncated_wav_returns_sample_decode_error() {
    let path = test_wav_path("truncated_sample_decode.wav");
    create_sine_wav_i16_mono(&path, 48000, 1024);

    let mut bytes = std::fs::read(&path).unwrap();
    bytes.truncate(bytes.len().saturating_sub(1));
    std::fs::write(&path, bytes).unwrap();

    let result = SpoofedAudioSource::new(&path);
    match result {
        Err(e) => assert!(
            e.contains("Failed to decode WAV sample"),
            "unexpected error: {}", e
        ),
        Ok(_) => panic!("expected sample decode error for truncated WAV"),
    }

    std::fs::remove_file(&path).ok();
}

#[test]
fn test_create_source_custom_fails_falls_back_to_default() {
    let default_path = test_wav_path("fallback_default.wav");
    let custom_path = test_wav_path("fallback_custom_invalid.wav");
    create_sine_wav_i16_mono(&default_path, 48000, 4800);
    std::fs::write(&custom_path, b"not a wav").unwrap();

    let source = create_source_with_fallback(1000, &default_path, |_| {
        let custom_file = std::fs::File::open(&custom_path)
            .map_err(|e| format!("Failed to open custom test file: {}", e))?;
        Ok(Some(custom_file))
    });

    assert!(source.is_some(), "expected fallback to default source");
    let mut src = source.unwrap();
    let mut output = vec![0.0f32; 480];
    let frames = src.read_f32(&mut output, 48000, 1);
    assert_eq!(frames, 480);
    assert!(output.iter().any(|&s| s.abs() > 0.01), "default fallback output should be non-silent");

    std::fs::remove_file(&default_path).ok();
    std::fs::remove_file(&custom_path).ok();
}

#[test]
fn test_create_source_no_custom_uses_default() {
    let default_path = test_wav_path("fallback_no_custom_default.wav");
    create_sine_wav_i16_mono(&default_path, 48000, 4800);

    let source = create_source_with_fallback(1000, &default_path, |_| Ok(None));
    assert!(source.is_some(), "expected default source when custom FD is absent");

    let src = source.unwrap();
    assert_eq!(src.source_sample_rate, 48000);
    assert_eq!(src.source_channels, 1);
    assert!(!src.samples.is_empty(), "default source should contain decoded samples");

    std::fs::remove_file(&default_path).ok();
}

#[test]
fn test_create_source_custom_valid_preferred_over_missing_default() {
    let custom_path = test_wav_path("fallback_custom_valid.wav");
    create_sine_wav_i16_mono(&custom_path, 8000, 800);

    let source = create_source_with_fallback(1000, "/nonexistent/path/default.wav", |_| {
        let custom_file = std::fs::File::open(&custom_path)
            .map_err(|e| format!("Failed to open custom test file: {}", e))?;
        Ok(Some(custom_file))
    });

    assert!(
        source.is_some(),
        "expected valid custom source to be used even when default path is missing"
    );
    let src = source.unwrap();
    assert_eq!(
        src.source_sample_rate, 8000,
        "source should come from custom WAV (8 kHz), not default fallback"
    );
    assert_eq!(src.source_channels, 1);
    assert!(!src.samples.is_empty(), "custom source should contain decoded samples");

    std::fs::remove_file(&custom_path).ok();
}

#[test]
fn test_create_source_both_fail_returns_none_and_null_source_silence() {
    let custom_path = test_wav_path("fallback_both_fail_custom_invalid.wav");
    std::fs::write(&custom_path, b"not a wav").unwrap();

    let _source = create_source_with_fallback(1000, "/nonexistent/path/default.wav", |_| {
        let custom_file = std::fs::File::open(&custom_path)
            .map_err(|e| format!("Failed to open custom test file: {}", e))?;
        Ok(Some(custom_file))
    });

    let mut output = vec![0xFFu8; 16 * std::mem::size_of::<i16>()];
    // SAFETY: null source is explicitly supported and output points to valid writable memory.
    let frames = unsafe {
        mic_spoofing_read_samples(
            std::ptr::null_mut(),
            output.as_mut_ptr(),
            16,
            48000,
            1,
            AUDIO_FORMAT_PCM_16_BIT,
        )
    };
    assert_eq!(frames, 16);
    assert!(output.iter().all(|&b| b == 0), "null source should fill i16 silence");

    std::fs::remove_file(&custom_path).ok();
}

#[test]
fn test_open_default_source_falls_back_to_system_path_when_override_fails() {
    let override_path = test_wav_path("default_override_invalid.wav");
    let system_path = test_wav_path("system_fallback.wav");
    std::fs::write(&override_path, b"not a wav").unwrap();
    create_sine_wav_i16_mono(&system_path, 48000, 4800);

    let source = super::open_default_source_with_system_fallback(&override_path, &system_path);
    assert!(source.is_some(), "expected fallback to system path source");

    let mut src = source.unwrap();
    let mut output = vec![0.0f32; 480];
    let frames = src.read_f32(&mut output, 48000, 1);
    assert_eq!(frames, 480);
    assert!(
        output.iter().any(|&s| s.abs() > 0.01),
        "system fallback output should be non-silent"
    );

    std::fs::remove_file(&override_path).ok();
    std::fs::remove_file(&system_path).ok();
}

#[test]
fn test_open_default_source_both_paths_fail_returns_none() {
    let source = super::open_default_source_with_system_fallback(
        "/nonexistent/path/default.wav",
        "/nonexistent/path/system.wav",
    );
    assert!(source.is_none(), "expected no source when both paths fail");
}

#[test]
fn test_read_samples_null_buffer_returns_zero() {
    // SAFETY: Passing null pointers is intentional here to validate FFI input guards
    let frames = unsafe {
        super::mic_spoofing_read_samples(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            16,
            48000,
            1,
            super::AUDIO_FORMAT_PCM_16_BIT,
        )
    };
    assert_eq!(frames, 0, "null output buffer must be rejected");
}

#[test]
fn test_read_samples_zero_sample_rate_returns_zero() {
    let mut buf = vec![0xABu8; 16 * std::mem::size_of::<i16>()];
    // SAFETY: Null source is explicitly supported and `buf` points to writable memory
    let frames = unsafe {
        super::mic_spoofing_read_samples(
            std::ptr::null_mut(),
            buf.as_mut_ptr(),
            16,
            0,
            1,
            super::AUDIO_FORMAT_PCM_16_BIT,
        )
    };
    assert_eq!(frames, 0, "sample_rate=0 must return 0");
    assert!(buf.iter().all(|&b| b == 0xAB), "buffer should be unchanged on rejected input");
}

#[test]
fn test_read_samples_zero_channel_count_returns_zero() {
    let mut buf = vec![0xABu8; 16 * std::mem::size_of::<i16>()];
    // SAFETY: Null source is explicitly supported and `buf` points to writable memory
    let frames = unsafe {
        super::mic_spoofing_read_samples(
            std::ptr::null_mut(),
            buf.as_mut_ptr(),
            16,
            48000,
            0,
            super::AUDIO_FORMAT_PCM_16_BIT,
        )
    };
    assert_eq!(frames, 0, "channel_count=0 must return 0");
    assert!(buf.iter().all(|&b| b == 0xAB), "buffer should be unchanged on rejected input");
}

#[test]
fn test_read_samples_channel_count_too_large_returns_zero() {
    let mut buf = vec![0xABu8; 16 * std::mem::size_of::<i16>()];
    // SAFETY: Null source is explicitly supported and `buf` points to writable memory
    let frames = unsafe {
        super::mic_spoofing_read_samples(
            std::ptr::null_mut(),
            buf.as_mut_ptr(),
            16,
            48000,
            u16::MAX as u32 + 1,
            super::AUDIO_FORMAT_PCM_16_BIT,
        )
    };
    assert_eq!(frames, 0, "channel_count > u16::MAX must return 0");
    assert!(buf.iter().all(|&b| b == 0xAB), "buffer should be unchanged on rejected input");
}

#[test]
fn test_read_samples_unknown_format_fills_silence_and_returns_frame_count() {
    let path = test_wav_path("ffi_unknown_format.wav");
    create_sine_wav_i16_mono(&path, 48000, 4800);
    let src = SpoofedAudioSource::new(&path).unwrap();
    let source = Box::into_raw(Box::new(src)) as *mut std::ffi::c_void;

    let mut buf = vec![0xFFu8; 16 * std::mem::size_of::<i16>()];
    // SAFETY: `source` was produced by Box::into_raw and `buf` is writable for requested output
    let frames = unsafe {
        super::mic_spoofing_read_samples(
            source,
            buf.as_mut_ptr(),
            16,
            48000,
            1,
            0xFF, // unknown format
        )
    };
    assert_eq!(frames, 16, "unknown formats should still report requested frame count");
    assert!(buf.iter().all(|&b| b == 0), "unknown format should output silence");

    // SAFETY: `source` is still owned by this test and has not been freed yet
    unsafe { super::mic_spoofing_destroy_source(source); }
    std::fs::remove_file(&path).ok();
}

#[test]
fn test_read_samples_excessive_frame_count_returns_zero() {
    let path = test_wav_path("excessive_frames.wav");
    create_sine_wav_i16_mono(&path, 48000, 4800);
    let src = SpoofedAudioSource::new(&path).unwrap();
    let source = Box::into_raw(Box::new(src)) as *mut std::ffi::c_void;

    // frame_count=3_000_000, channel_count=1 -> sample_count=3_000_000 > MAX_WAV_SAMPLES (2_880_000)
    let mut buf = vec![0xFFu8; 4]; // small sentinel-filled buffer (won't be written to)
    // SAFETY: `source` was created via Box::into_raw and `buf` is a valid writable buffer
    let result = unsafe {
        super::mic_spoofing_read_samples(
            source,
            buf.as_mut_ptr(),
            3_000_000,
            48000,
            1,
            super::AUDIO_FORMAT_PCM_16_BIT,
        )
    };
    assert_eq!(result, 0, "excessive frame_count should return 0");
    assert!(buf.iter().all(|&b| b == 0xFF), "buffer should not be modified");

    // SAFETY: `source` is still a valid pointer produced by `Box::into_raw`.
    unsafe { super::mic_spoofing_destroy_source(source); }
    std::fs::remove_file(&path).ok();
}


#[test]
fn test_read_i16_clamps_out_of_range_samples() {
    let mut src = make_source_from_samples(vec![-2.0, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0]);
    let mut output = vec![0i16; 7];

    let frames = src.read_i16(&mut output, 48000, 1);
    assert_eq!(frames, 7);
    assert_eq!(output, vec![-32768, -32767, -16383, 0, 16383, 32767, 32767]);
}

#[test]
fn test_read_u8_clamps_and_applies_unsigned_offset() {
    let mut src = make_source_from_samples(vec![-2.0, -1.0, 0.0, 1.0, 2.0]);
    let mut output = vec![0u8; 5];

    let frames = src.read_u8(&mut output, 48000, 1);
    assert_eq!(frames, 5);
    assert_eq!(output, vec![0, 1, 128, 255, 255]);
}

#[test]
fn test_read_i24_packed_clamps_out_of_range_samples() {
    let mut src = make_source_from_samples(vec![-2.0, -1.0, 0.0, 1.0, 2.0]);
    let mut output = vec![0u8; 5 * 3];

    let frames = src.read_i24_packed(&mut output, 48000, 1);
    assert_eq!(frames, 5);

    let expected = [-8_388_608i32, -8_388_607, 0, 8_388_607, 8_388_607];
    for (i, expected_val) in expected.iter().enumerate() {
        let bytes = expected_val.to_le_bytes();
        let base = i * 3;
        assert_eq!(output[base], bytes[0], "low byte mismatch at sample {}", i);
        assert_eq!(output[base + 1], bytes[1], "mid byte mismatch at sample {}", i);
        assert_eq!(output[base + 2], bytes[2], "high byte mismatch at sample {}", i);
    }
}

#[test]
fn test_read_i32_clamps_out_of_range_samples() {
    let mut src = make_source_from_samples(vec![-2.0, -1.0, 0.0, 1.0, 2.0]);
    let mut output = vec![0i32; 5];

    let frames = src.read_i32(&mut output, 48000, 1);
    assert_eq!(frames, 5);
    // PCM_I32_MAX is stored as f32 and rounds to 2^31, so -1.0 maps to i32::MIN here
    assert_eq!(
        output,
        vec![-2_147_483_648, -2_147_483_648, 0, 2_147_483_647, 2_147_483_647]
    );
}
