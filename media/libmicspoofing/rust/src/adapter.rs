use std::ffi::c_void;
use std::io::ErrorKind;
use std::os::fd::{AsRawFd, OwnedFd};

use crate::{
    PCM_I16_MAX, PCM_I16_MIN, PCM_I24_MAX, PCM_I24_MIN, PCM_I32_MAX, PCM_I32_MIN, PCM_U8_OFFSET,
    PCM_U8_SCALE,
};

const PIPE_READ_CHUNK_SAMPLES: usize = 4096;

unsafe extern "C" {
    fn read(fd: i32, buf: *mut c_void, count: usize) -> isize;
}

pub(crate) struct SpoofedAudioSource {
    pipe_read: Option<OwnedFd>,
    source_sample_rate: u32,
    source_channels: u32,
    read_buf: Vec<f32>,
    read_buf_pos: usize,
    read_buf_len: usize,
    frac_position: f64,
    partial_sample_bytes: [u8; 4],
    partial_sample_len: usize,
    pipe_eof: bool,
}

impl SpoofedAudioSource {
    pub(crate) fn from_pipe(
        pipe_read: OwnedFd,
        source_sample_rate: u32,
        source_channels: u32,
    ) -> Self {
        Self {
            pipe_read: Some(pipe_read),
            source_sample_rate,
            source_channels,
            read_buf: vec![0.0; PIPE_READ_CHUNK_SAMPLES.max(source_channels as usize)],
            read_buf_pos: 0,
            read_buf_len: 0,
            frac_position: 0.0,
            partial_sample_bytes: [0; 4],
            partial_sample_len: 0,
            pipe_eof: false,
        }
    }

    pub(crate) fn silence() -> Self {
        Self {
            pipe_read: None,
            source_sample_rate: 0,
            source_channels: 0,
            read_buf: Vec::new(),
            read_buf_pos: 0,
            read_buf_len: 0,
            frac_position: 0.0,
            partial_sample_bytes: [0; 4],
            partial_sample_len: 0,
            pipe_eof: true,
        }
    }

    fn available_samples(&self) -> usize {
        self.read_buf_len.saturating_sub(self.read_buf_pos)
    }

    fn available_complete_frames(&self) -> usize {
        if self.source_channels == 0 {
            0
        } else {
            self.available_samples() / self.source_channels as usize
        }
    }

    fn push_sample(&mut self, sample: f32) {
        if self.read_buf_len == self.read_buf.len() {
            let next_len = self.read_buf.len().max(PIPE_READ_CHUNK_SAMPLES) * 2;
            self.read_buf.resize(next_len, 0.0);
        }
        self.read_buf[self.read_buf_len] = sample;
        self.read_buf_len += 1;
    }

    fn compact_read_buffer(&mut self) {
        if self.read_buf_pos == 0 {
            return;
        }

        let remaining = self.read_buf_len.saturating_sub(self.read_buf_pos);
        if remaining > 0 {
            self.read_buf.copy_within(self.read_buf_pos..self.read_buf_len, 0);
        }
        self.read_buf_pos = 0;
        self.read_buf_len = remaining;
    }

    fn append_pipe_bytes(&mut self, bytes: &[u8]) {
        let mut offset = 0usize;

        if self.partial_sample_len > 0 {
            let needed = 4 - self.partial_sample_len;
            let take = needed.min(bytes.len());
            self.partial_sample_bytes[self.partial_sample_len..self.partial_sample_len + take]
                .copy_from_slice(&bytes[..take]);
            self.partial_sample_len += take;
            offset += take;

            if self.partial_sample_len == 4 {
                self.push_sample(f32::from_ne_bytes(self.partial_sample_bytes));
                self.partial_sample_len = 0;
            }
        }

        for chunk in bytes[offset..].chunks_exact(4) {
            self.push_sample(f32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
            offset += 4;
        }

        let remainder = &bytes[offset..];
        if !remainder.is_empty() {
            self.partial_sample_bytes[..remainder.len()].copy_from_slice(remainder);
            self.partial_sample_len = remainder.len();
        }
    }

    fn read_more_from_pipe(&mut self) -> bool {
        if self.pipe_eof {
            return false;
        }

        let Some(pipe_read_fd) = self.pipe_read.as_ref().map(AsRawFd::as_raw_fd) else {
            self.pipe_eof = true;
            return false;
        };

        self.compact_read_buffer();

        let mut temp = [0u8; PIPE_READ_CHUNK_SAMPLES * std::mem::size_of::<f32>()];
        loop {
            // SAFETY: `pipe_read` is a live owned descriptor and `temp` is writable storage
            let read_bytes = unsafe { read(pipe_read_fd, temp.as_mut_ptr().cast(), temp.len()) };
            if read_bytes < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == ErrorKind::Interrupted {
                    continue;
                }

                self.pipe_eof = true;
                return false;
            }

            if read_bytes == 0 {
                self.pipe_eof = true;
                return false;
            }

            self.append_pipe_bytes(&temp[..read_bytes as usize]);
            return true;
        }
    }

    fn ensure_frame_available(&mut self) -> bool {
        while self.available_complete_frames() == 0 {
            if !self.read_more_from_pipe() {
                return false;
            }
        }

        true
    }

    fn discard_source_frames(&mut self, mut frame_count: usize) -> bool {
        if frame_count == 0 {
            return true;
        }

        let channels = self.source_channels as usize;
        while frame_count > 0 {
            let available_frames = self.available_complete_frames();
            if available_frames == 0 {
                if !self.read_more_from_pipe() {
                    return false;
                }
                continue;
            }

            let discard_now = frame_count.min(available_frames);
            self.read_buf_pos += discard_now * channels;
            frame_count -= discard_now;

            if self.read_buf_pos == self.read_buf_len {
                self.read_buf_pos = 0;
                self.read_buf_len = 0;
            } else if self.read_buf_pos >= PIPE_READ_CHUNK_SAMPLES {
                self.compact_read_buffer();
            }
        }

        true
    }

    pub(crate) fn read_f32(
        &mut self,
        output: &mut [f32],
        target_sample_rate: u32,
        target_channels: u16,
    ) -> usize {
        if target_channels == 0 {
            return 0;
        }

        let frames_out = output.len() / target_channels as usize;
        if frames_out == 0 {
            return 0;
        }

        output[..frames_out * target_channels as usize].fill(0.0);
        if self.pipe_read.is_none() || self.source_sample_rate == 0 || self.source_channels == 0 {
            return frames_out;
        }

        let ratio = self.source_sample_rate as f64 / target_sample_rate as f64;
        if !ratio.is_finite() || ratio <= 0.0 {
            return frames_out;
        }

        let source_channels = self.source_channels as usize;
        let target_channels = target_channels as usize;
        for frame in 0..frames_out {
            if !self.ensure_frame_available() {
                return frames_out;
            }

            let base = self.read_buf_pos;
            for channel in 0..target_channels {
                let source_channel = channel.min(source_channels - 1);
                output[frame * target_channels + channel] = self.read_buf[base + source_channel];
            }

            self.frac_position += ratio;
            let discard_frames = self.frac_position.floor() as usize;
            self.frac_position -= discard_frames as f64;
            if discard_frames > 0 && !self.discard_source_frames(discard_frames) {
                return frames_out;
            }
        }

        frames_out
    }

    pub(crate) fn read_i16(
        &mut self,
        output: &mut [i16],
        target_sample_rate: u32,
        target_channels: u16,
    ) -> usize {
        let mut temp = vec![0.0f32; output.len()];
        let frames = self.read_f32(&mut temp, target_sample_rate, target_channels);
        for (dst, src) in output.iter_mut().zip(temp.into_iter()) {
            *dst = (src * PCM_I16_MAX).clamp(PCM_I16_MIN, PCM_I16_MAX) as i16;
        }
        frames
    }

    pub(crate) fn read_u8(
        &mut self,
        output: &mut [u8],
        target_sample_rate: u32,
        target_channels: u16,
    ) -> usize {
        let mut temp = vec![0.0f32; output.len()];
        let frames = self.read_f32(&mut temp, target_sample_rate, target_channels);
        for (dst, src) in output.iter_mut().zip(temp.into_iter()) {
            *dst = (src * PCM_U8_SCALE + PCM_U8_OFFSET).clamp(0.0, 255.0) as u8;
        }
        frames
    }

    pub(crate) fn read_i24_packed(
        &mut self,
        output: &mut [u8],
        target_sample_rate: u32,
        target_channels: u16,
    ) -> usize {
        let sample_count = output.len() / 3;
        let mut temp = vec![0.0f32; sample_count];
        let frames = self.read_f32(&mut temp, target_sample_rate, target_channels);
        for (index, sample) in temp.into_iter().enumerate() {
            let clamped = (sample * PCM_I24_MAX).clamp(PCM_I24_MIN, PCM_I24_MAX) as i32;
            let bytes = clamped.to_le_bytes();
            let base = index * 3;
            output[base] = bytes[0];
            output[base + 1] = bytes[1];
            output[base + 2] = bytes[2];
        }
        frames
    }

    pub(crate) fn read_i32(
        &mut self,
        output: &mut [i32],
        target_sample_rate: u32,
        target_channels: u16,
    ) -> usize {
        let mut temp = vec![0.0f32; output.len()];
        let frames = self.read_f32(&mut temp, target_sample_rate, target_channels);
        for (dst, src) in output.iter_mut().zip(temp.into_iter()) {
            *dst = (src * PCM_I32_MAX).clamp(PCM_I32_MIN, PCM_I32_MAX) as i32;
        }
        frames
    }
}
