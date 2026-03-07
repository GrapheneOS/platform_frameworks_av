package com.android.mic_spoofing_test;

import static com.google.common.truth.Truth.assertWithMessage;

import android.content.Context;
import android.media.MediaCodec;
import android.media.MediaExtractor;
import android.media.MediaFormat;
import android.media.MediaMetadataRetriever;
import android.media.MediaRecorder;
import android.os.ParcelFileDescriptor;

import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.filters.LargeTest;
import androidx.test.platform.app.InstrumentationRegistry;

import org.junit.After;
import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;

import java.io.File;
import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

@LargeTest
@RunWith(AndroidJUnit4.class)
public class SpoofedMediaRecorderTest extends BaseMicSpoofingTest {

    private static final int RECORD_DURATION_MS = 2000;
    private static final int SAMPLE_RATE = 44100;
    private static final int CHANNELS = 1;
    private static final int BIT_RATE = 64000;

    private Context context;
    private final List<File> outputFiles = new ArrayList<>();

    @Before
    public void setUp() throws Exception {
        context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        initPackageContext();

        enableMicSpoofing();
    }

    @After
    public void tearDown() throws Exception {
        disableMicSpoofing();
        for (var file : outputFiles) {
            file.delete();
        }
        outputFiles.clear();
    }

    private File createOutputFile(String suffix) throws IOException {
        var file = File.createTempFile("mic_spoof_test_", suffix, context.getCacheDir());
        outputFiles.add(file);
        return file;
    }

    private MediaRecorder createRecorder(File outputFile, int audioSource, int outputFormat,
            int audioEncoder, int sampleRate, int channels, int bitRate) {
        var recorder = new MediaRecorder(context);
        recorder.setAudioSource(audioSource);
        recorder.setOutputFormat(outputFormat);
        recorder.setAudioEncoder(audioEncoder);
        recorder.setAudioSamplingRate(sampleRate);
        recorder.setAudioChannels(channels);
        recorder.setAudioEncodingBitRate(bitRate);
        recorder.setOutputFile(outputFile.getAbsolutePath());
        return recorder;
    }

    private MediaRecorder createRecorder(File outputFile, int outputFormat,
            int audioEncoder, int sampleRate, int channels, int bitRate) {
        return createRecorder(outputFile, MediaRecorder.AudioSource.MIC, outputFormat,
                audioEncoder, sampleRate, channels, bitRate);
    }

    private MediaRecorder createDefaultRecorder(File outputFile) {
        return createRecorder(outputFile, MediaRecorder.OutputFormat.THREE_GPP,
                MediaRecorder.AudioEncoder.AAC, SAMPLE_RATE, CHANNELS, BIT_RATE);
    }

    private static void recordForDuration(MediaRecorder recorder, int durationMs) throws Exception {
        recorder.prepare();
        recorder.start();
        Thread.sleep(durationMs);
        recorder.stop();
    }

    private static int findAudioTrack(MediaExtractor extractor) {
        for (var i = 0; i < extractor.getTrackCount(); i++) {
            var format = extractor.getTrackFormat(i);
            var mime = format.getString(MediaFormat.KEY_MIME);
            if (mime != null && mime.startsWith("audio/")) {
                return i;
            }
        }
        return -1;
    }

    private static long getAudioDurationMs(File file) throws IOException {
        try (var retriever = new MediaMetadataRetriever()) {
            retriever.setDataSource(file.getAbsolutePath());
            var durationStr = retriever.extractMetadata(
                    MediaMetadataRetriever.METADATA_KEY_DURATION);
            return durationStr != null ? Long.parseLong(durationStr) : -1;
        }
    }

    private static boolean decodedAudioIsNonSilent(File file) throws IOException {
        var extractor = new MediaExtractor();
        try {
            extractor.setDataSource(file.getAbsolutePath());
            var audioTrack = findAudioTrack(extractor);
            if (audioTrack < 0) return false;

            extractor.selectTrack(audioTrack);
            var format = extractor.getTrackFormat(audioTrack);
            var mime = format.getString(MediaFormat.KEY_MIME);

            var decoder = MediaCodec.createDecoderByType(mime);
            try {
                decoder.configure(format, null, null, 0);
                decoder.start();

                var info = new MediaCodec.BufferInfo();
                var inputDone = false;
                var timeoutUs = 10_000L;

                while (true) {
                    if (!inputDone) {
                        var inIdx = decoder.dequeueInputBuffer(timeoutUs);
                        if (inIdx >= 0) {
                            var inBuf = decoder.getInputBuffer(inIdx);
                            var sampleSize = extractor.readSampleData(inBuf, 0);
                            if (sampleSize < 0) {
                                decoder.queueInputBuffer(inIdx, 0, 0, 0,
                                        MediaCodec.BUFFER_FLAG_END_OF_STREAM);
                                inputDone = true;
                            } else {
                                decoder.queueInputBuffer(inIdx, 0, sampleSize,
                                        extractor.getSampleTime(), 0);
                                extractor.advance();
                            }
                        }
                    }

                    var outIdx = decoder.dequeueOutputBuffer(info, timeoutUs);
                    if (outIdx >= 0) {
                        if (info.size > 0) {
                            var outBuf = decoder.getOutputBuffer(outIdx);
                            for (var i = info.offset; i < info.offset + info.size; i++) {
                                if (outBuf.get(i) != 0) {
                                    return true;
                                }
                            }
                        }
                        decoder.releaseOutputBuffer(outIdx, false);
                        if ((info.flags & MediaCodec.BUFFER_FLAG_END_OF_STREAM) != 0) {
                            break;
                        }
                    } else if (outIdx == MediaCodec.INFO_TRY_AGAIN_LATER && inputDone) {
                        break;
                    }
                }
                return false;
            } finally {
                decoder.stop();
                decoder.release();
            }
        } finally {
            extractor.release();
        }
    }

    @Test
    public void testMediaRecorder_spoofed_producesOutputFile() throws Exception {
        var outputFile = createOutputFile(".3gp");
        var recorder = createDefaultRecorder(outputFile);
        try {
            recordForDuration(recorder, RECORD_DURATION_MS);

            assertWithMessage("Output file should exist").that(outputFile.exists()).isTrue();
            assertWithMessage("Output file should not be empty")
                    .that(outputFile.length())
                    .isGreaterThan(0L);
        } finally {
            recorder.release();
        }
    }

    @Test
    public void testMediaRecorder_spoofed_outputContainsAudioTrack() throws Exception {
        var outputFile = createOutputFile(".3gp");
        var recorder = createDefaultRecorder(outputFile);
        try {
            recordForDuration(recorder, RECORD_DURATION_MS);
        } finally {
            recorder.release();
        }

        var extractor = new MediaExtractor();
        try {
            extractor.setDataSource(outputFile.getAbsolutePath());
            var audioTrack = findAudioTrack(extractor);
            assertWithMessage("Output should contain an audio track")
                    .that(audioTrack).isAtLeast(0);

            var format = extractor.getTrackFormat(audioTrack);
            var mime = format.getString(MediaFormat.KEY_MIME);
            assertWithMessage("Audio track MIME type should start with 'audio/'")
                    .that(mime).startsWith("audio/");
        } finally {
            extractor.release();
        }
    }

    @Test
    public void testMediaRecorder_spoofed_outputContainsNonSilentAudio() throws Exception {
        var outputFile = createOutputFile(".3gp");
        var recorder = createDefaultRecorder(outputFile);
        try {
            recordForDuration(recorder, RECORD_DURATION_MS);
        } finally {
            recorder.release();
        }

        assertWithMessage("Decoded spoofed audio should contain non-zero PCM samples")
                .that(decodedAudioIsNonSilent(outputFile)).isTrue();
    }

    @Test
    public void testMediaRecorder_spoofed_durationApproximatelyCorrect() throws Exception {
        var recordDurationMs = 3000;
        var outputFile = createOutputFile(".3gp");
        var recorder = createDefaultRecorder(outputFile);
        try {
            recordForDuration(recorder, recordDurationMs);
        } finally {
            recorder.release();
        }

        var durationMs = getAudioDurationMs(outputFile);
        assertWithMessage("Duration should be reported")
                .that(durationMs).isGreaterThan(0L);
        assertWithMessage("Duration should be at least 50% of recorded time")
                .that(durationMs).isAtLeast((long) (recordDurationMs * 0.5));
        assertWithMessage("Duration should not exceed 200% of recorded time")
                .that(durationMs).isAtMost((long) (recordDurationMs * 2.0));
    }

    @Test
    public void testMediaRecorder_spoofed_getMaxAmplitudeReturnsNonZero() throws Exception {
        var outputFile = createOutputFile(".3gp");
        var recorder = createDefaultRecorder(outputFile);
        try {
            recorder.prepare();
            recorder.start();
            // Poll getMaxAmplitude() until a non-zero value is returned.
            // Each call resets the max, so the first call(s) may return 0
            // due to encoding pipeline startup latency
            var maxAmplitude = 0;
            for (var i = 0; i < 30 && maxAmplitude == 0; i++) {
                Thread.sleep(100);
                maxAmplitude = recorder.getMaxAmplitude();
            }
            assertWithMessage("getMaxAmplitude() should return > 0 for spoofed audio")
                    .that(maxAmplitude).isGreaterThan(0);

            recorder.stop();
        } finally {
            recorder.release();
        }
    }

    @Test
    public void testMediaRecorder_spoofed_aacAdtsFormat() throws Exception {
        var outputFile = createOutputFile(".aac");
        var recorder = createRecorder(outputFile,
                MediaRecorder.OutputFormat.AAC_ADTS,
                MediaRecorder.AudioEncoder.AAC,
                SAMPLE_RATE, CHANNELS, BIT_RATE);
        try {
            recordForDuration(recorder, RECORD_DURATION_MS);
        } finally {
            recorder.release();
        }

        assertWithMessage("AAC ADTS file should not be empty")
                .that(outputFile.length()).isGreaterThan(0L);

        var extractor = new MediaExtractor();
        try {
            extractor.setDataSource(outputFile.getAbsolutePath());
            assertWithMessage("AAC ADTS output should contain an audio track")
                    .that(findAudioTrack(extractor)).isAtLeast(0);
        } finally {
            extractor.release();
        }
    }

    @Test
    public void testMediaRecorder_spoofed_amrNbFormat() throws Exception {
        var outputFile = createOutputFile(".amr");
        // AMR-NB requires 8 kHz mono
        var recorder = createRecorder(outputFile,
                MediaRecorder.OutputFormat.AMR_NB,
                MediaRecorder.AudioEncoder.AMR_NB,
                8000, 1, 12200);
        try {
            recordForDuration(recorder, RECORD_DURATION_MS);
        } finally {
            recorder.release();
        }

        assertWithMessage("AMR NB file should not be empty")
                .that(outputFile.length()).isGreaterThan(0L);

        var extractor = new MediaExtractor();
        try {
            extractor.setDataSource(outputFile.getAbsolutePath());
            assertWithMessage("AMR NB output should contain an audio track")
                    .that(findAudioTrack(extractor)).isAtLeast(0);
        } finally {
            extractor.release();
        }
    }

    @Test
    public void testMediaRecorder_spoofed_mpeg4Format() throws Exception {
        var outputFile = createOutputFile(".mp4");
        var recorder = createRecorder(outputFile,
                MediaRecorder.OutputFormat.MPEG_4,
                MediaRecorder.AudioEncoder.AAC,
                SAMPLE_RATE, CHANNELS, BIT_RATE);
        try {
            recordForDuration(recorder, RECORD_DURATION_MS);
        } finally {
            recorder.release();
        }

        assertWithMessage("MPEG-4 file should not be empty")
                .that(outputFile.length()).isGreaterThan(0L);

        var extractor = new MediaExtractor();
        try {
            extractor.setDataSource(outputFile.getAbsolutePath());
            assertWithMessage("MPEG-4 output should contain an audio track")
                    .that(findAudioTrack(extractor)).isAtLeast(0);
        } finally {
            extractor.release();
        }
    }

    @Test
    public void testMediaRecorder_spoofed_oggOpusFormat() throws Exception {
        var outputFile = createOutputFile(".ogg");
        // Opus requires 48 kHz
        var recorder = createRecorder(outputFile,
                MediaRecorder.OutputFormat.OGG,
                MediaRecorder.AudioEncoder.OPUS,
                48000, 1, 64000);
        try {
            recordForDuration(recorder, RECORD_DURATION_MS);
        } finally {
            recorder.release();
        }

        assertWithMessage("OGG file should not be empty")
                .that(outputFile.length()).isGreaterThan(0L);

        var extractor = new MediaExtractor();
        try {
            extractor.setDataSource(outputFile.getAbsolutePath());
            assertWithMessage("OGG/Opus output should contain an audio track")
                    .that(findAudioTrack(extractor)).isAtLeast(0);
        } finally {
            extractor.release();
        }
    }

    @Test
    public void testMediaRecorder_spoofed_variousSampleRates() throws Exception {
        int[] sampleRates = {8000, 16000, 44100, 48000};

        for (var sampleRate : sampleRates) {
            var desc = "sampleRate=" + sampleRate;
            var outputFile = createOutputFile("_" + sampleRate + ".3gp");
            var recorder = createRecorder(outputFile,
                    MediaRecorder.OutputFormat.THREE_GPP,
                    MediaRecorder.AudioEncoder.AAC,
                    sampleRate, CHANNELS, BIT_RATE);
            try {
                recordForDuration(recorder, 1000);
            } finally {
                recorder.release();
            }

            assertWithMessage(desc + ": file should not be empty")
                    .that(outputFile.length()).isGreaterThan(0L);

            var extractor = new MediaExtractor();
            try {
                extractor.setDataSource(outputFile.getAbsolutePath());
                assertWithMessage(desc + ": should contain an audio track")
                        .that(findAudioTrack(extractor)).isAtLeast(0);
            } finally {
                extractor.release();
            }
        }
    }

    @Test
    public void testMediaRecorder_spoofed_stereoRecording() throws Exception {
        var outputFile = createOutputFile(".3gp");
        var recorder = createRecorder(outputFile,
                MediaRecorder.OutputFormat.THREE_GPP,
                MediaRecorder.AudioEncoder.AAC,
                SAMPLE_RATE, 2, BIT_RATE);
        try {
            recordForDuration(recorder, RECORD_DURATION_MS);
        } finally {
            recorder.release();
        }

        assertWithMessage("Stereo recording should produce non-empty file")
                .that(outputFile.length()).isGreaterThan(0L);

        var extractor = new MediaExtractor();
        try {
            extractor.setDataSource(outputFile.getAbsolutePath());
            var audioTrack = findAudioTrack(extractor);
            assertWithMessage("Stereo recording should contain an audio track")
                    .that(audioTrack).isAtLeast(0);

            // The encoder may downmix identical L/R channels (from mono WAV upmix) to mono.
            // The key assertion is that requesting 2 channels does not break the recording pipeline
            var format = extractor.getTrackFormat(audioTrack);
            var channelCount = format.getInteger(MediaFormat.KEY_CHANNEL_COUNT);
            assertWithMessage("Channel count should be 1 or 2").that(channelCount).isAnyOf(1, 2);
        } finally {
            extractor.release();
        }
    }

    @Test
    public void testMediaRecorder_spoofed_resetAndRerecord() throws Exception {
        var outputFile1 = createOutputFile("_first.3gp");
        var outputFile2 = createOutputFile("_second.3gp");

        var recorder = createDefaultRecorder(outputFile1);
        try {
            recordForDuration(recorder, 1000);
            assertWithMessage("First output file should not be empty")
                    .that(outputFile1.length()).isGreaterThan(0L);

            recorder.reset();
            recorder.setAudioSource(MediaRecorder.AudioSource.MIC);
            recorder.setOutputFormat(MediaRecorder.OutputFormat.THREE_GPP);
            recorder.setAudioEncoder(MediaRecorder.AudioEncoder.AAC);
            recorder.setAudioSamplingRate(SAMPLE_RATE);
            recorder.setAudioChannels(CHANNELS);
            recorder.setAudioEncodingBitRate(BIT_RATE);
            recorder.setOutputFile(outputFile2.getAbsolutePath());

            recordForDuration(recorder, 1000);
            assertWithMessage("Second output file should not be empty")
                    .that(outputFile2.length()).isGreaterThan(0L);
        } finally {
            recorder.release();
        }

        for (var file : new File[]{outputFile1, outputFile2}) {
            var extractor = new MediaExtractor();
            try {
                extractor.setDataSource(file.getAbsolutePath());
                assertWithMessage(file.getName() + " should contain an audio track")
                        .that(findAudioTrack(extractor)).isAtLeast(0);
            } finally {
                extractor.release();
            }
        }
    }

    @Test
    public void testMediaRecorder_spoofed_pauseAndResume() throws Exception {
        var outputFile = createOutputFile(".3gp");
        var recorder = createDefaultRecorder(outputFile);
        try {
            recorder.prepare();
            recorder.start();
            Thread.sleep(500);

            recorder.pause();
            Thread.sleep(500); // paused — no audio should be encoded

            recorder.resume();
            Thread.sleep(500);
            recorder.stop();
        } finally {
            recorder.release();
        }

        assertWithMessage("Pause/resume recording should produce non-empty file")
                .that(outputFile.length()).isGreaterThan(0L);

        // Duration should reflect only active recording time (~1s, not 1.5s)
        var durationMs = getAudioDurationMs(outputFile);
        assertWithMessage("Duration should be reported").that(durationMs).isGreaterThan(0L);
        assertWithMessage("Duration should exclude paused time (≤ 1.5s)")
                .that(durationMs).isAtMost(1500L);
    }

    @Test
    public void testMediaRecorder_spoofed_releaseWithoutExplicitStop() throws Exception {
        var outputFile = createOutputFile(".3gp");
        var recorder = createDefaultRecorder(outputFile);

        recorder.prepare();
        recorder.start();
        Thread.sleep(500);
        recorder.release();

        assertWithMessage("Output file should exist after release-without-stop")
                .that(outputFile.exists()).isTrue();
    }

    @Test
    public void testMediaRecorder_spoofed_shortRecording() throws Exception {
        var outputFile = createOutputFile(".3gp");
        var recorder = createDefaultRecorder(outputFile);
        try {
            recordForDuration(recorder, 500);
        } finally {
            recorder.release();
        }

        assertWithMessage("Short recording should produce non-empty file")
                .that(outputFile.length()).isGreaterThan(0L);

        var extractor = new MediaExtractor();
        try {
            extractor.setDataSource(outputFile.getAbsolutePath());
            assertWithMessage("Short recording should contain an audio track")
                    .that(findAudioTrack(extractor)).isAtLeast(0);
        } finally {
            extractor.release();
        }
    }

    @Test
    public void testMediaRecorder_spoofed_maxDurationCallback() throws Exception {
        var outputFile = createOutputFile(".3gp");
        var recorder = createDefaultRecorder(outputFile);

        var latch = new CountDownLatch(1);
        var receivedWhat = new int[1];
        recorder.setOnInfoListener((mr, what, extra) -> {
            if (what == MediaRecorder.MEDIA_RECORDER_INFO_MAX_DURATION_REACHED) {
                receivedWhat[0] = what;
                latch.countDown();
            }
        });
        recorder.setMaxDuration(1000); // 1 second

        try {
            recorder.prepare();
            recorder.start();

            var fired = latch.await(5, TimeUnit.SECONDS);
            assertWithMessage("Max duration callback should fire within 5 seconds")
                    .that(fired).isTrue();
            assertWithMessage("Callback should report MAX_DURATION_REACHED")
                    .that(receivedWhat[0])
                    .isEqualTo(MediaRecorder.MEDIA_RECORDER_INFO_MAX_DURATION_REACHED);

            recorder.reset();
        } finally {
            recorder.release();
        }

        assertWithMessage("Max-duration file should not be empty")
                .that(outputFile.length()).isGreaterThan(0L);
    }

    @Test
    public void testMediaRecorder_spoofed_fileDescriptorOutput() throws Exception {
        var outputFile = createOutputFile(".3gp");

        var parcelFileDescriptor = ParcelFileDescriptor.open(outputFile,
                ParcelFileDescriptor.MODE_CREATE | ParcelFileDescriptor.MODE_READ_WRITE);

        assertWithMessage("ParcelFileDescrtiptor should not be null")
                .that(parcelFileDescriptor).isNotNull();

        var recorder = new MediaRecorder(context);
        try {
            recorder.setAudioSource(MediaRecorder.AudioSource.MIC);
            recorder.setOutputFormat(MediaRecorder.OutputFormat.THREE_GPP);
            recorder.setAudioEncoder(MediaRecorder.AudioEncoder.AAC);
            recorder.setAudioSamplingRate(SAMPLE_RATE);
            recorder.setAudioChannels(CHANNELS);
            recorder.setAudioEncodingBitRate(BIT_RATE);
            recorder.setOutputFile(parcelFileDescriptor.getFileDescriptor());

            recordForDuration(recorder, RECORD_DURATION_MS);
        } finally {
            recorder.release();
            parcelFileDescriptor.close();
        }

        assertWithMessage("FD-based output file should not be empty")
                .that(outputFile.length()).isGreaterThan(0L);

        var extractor = new MediaExtractor();
        try {
            extractor.setDataSource(outputFile.getAbsolutePath());
            assertWithMessage("FD-based output should contain an audio track")
                    .that(findAudioTrack(extractor)).isAtLeast(0);
        } finally {
            extractor.release();
        }
    }

    @Test
    public void testMediaRecorder_spoofed_concurrentInstances() throws Exception {
        var outputFile1 = createOutputFile("_concurrent1.3gp");
        var outputFile2 = createOutputFile("_concurrent2.amr");

        var recorder1 = createDefaultRecorder(outputFile1);

        // Second recorder uses a different format to exercise two distinct
        // encoder pipelines in the media server simultaneously
        var recorder2 = createRecorder(outputFile2,
                MediaRecorder.OutputFormat.AMR_NB,
                MediaRecorder.AudioEncoder.AMR_NB,
                8000, 1, 12200);
        try {
            recorder1.prepare();
            recorder2.prepare();
            recorder1.start();
            recorder2.start();
            Thread.sleep(RECORD_DURATION_MS);
            recorder1.stop();
            recorder2.stop();
        } finally {
            recorder1.release();
            recorder2.release();
        }

        assertWithMessage("First concurrent file should not be empty")
                .that(outputFile1.length()).isGreaterThan(0L);
        assertWithMessage("Second concurrent file should not be empty")
                .that(outputFile2.length()).isGreaterThan(0L);

        for (var file : new File[]{outputFile1, outputFile2}) {
            var extractor = new MediaExtractor();
            try {
                extractor.setDataSource(file.getAbsolutePath());
                assertWithMessage(file.getName() + " should contain an audio track")
                        .that(findAudioTrack(extractor)).isAtLeast(0);
            } finally {
                extractor.release();
            }
        }
    }

    @Test
    public void testMediaRecorder_spoofed_voiceRecognitionSource() throws Exception {
        var outputFile = createOutputFile(".3gp");
        var recorder = createRecorder(outputFile, MediaRecorder.AudioSource.VOICE_RECOGNITION,
                MediaRecorder.OutputFormat.THREE_GPP, MediaRecorder.AudioEncoder.AAC,
                SAMPLE_RATE, CHANNELS, BIT_RATE);
        try {
            recordForDuration(recorder, RECORD_DURATION_MS);
        } finally {
            recorder.release();
        }

        assertWithMessage("VOICE_RECOGNITION source should produce non-empty file")
                .that(outputFile.length()).isGreaterThan(0L);

        var extractor = new MediaExtractor();
        try {
            extractor.setDataSource(outputFile.getAbsolutePath());
            assertWithMessage("VOICE_RECOGNITION output should contain an audio track")
                    .that(findAudioTrack(extractor)).isAtLeast(0);
        } finally {
            extractor.release();
        }
    }

    @Test
    public void testMediaRecorder_spoofed_defaultSource() throws Exception {
        var outputFile = createOutputFile(".3gp");
        var recorder = createRecorder(outputFile, MediaRecorder.AudioSource.DEFAULT,
                MediaRecorder.OutputFormat.THREE_GPP, MediaRecorder.AudioEncoder.AAC,
                SAMPLE_RATE, CHANNELS, BIT_RATE);
        try {
            recordForDuration(recorder, RECORD_DURATION_MS);
        } finally {
            recorder.release();
        }

        assertWithMessage("DEFAULT source should produce non-empty file")
                .that(outputFile.length()).isGreaterThan(0L);
    }

    @Test
    public void testMediaRecorder_spoofed_camcorderSourceFails() {
        var recorder = new MediaRecorder(context);
        try {
            RuntimeException thrown = null;
            try {
                recorder.setAudioSource(MediaRecorder.AudioSource.CAMCORDER);
            } catch (RuntimeException e) {
                thrown = e;
            }

            assertWithMessage("setAudioSource(CAMCORDER) should fail with mic spoofing enabled")
                    .that(thrown).isNotNull();
        } finally {
            recorder.release();
        }
    }

    @Test
    public void testMediaRecorder_spoofed_maxFileSizeCallback() throws Exception {
        var outputFile = createOutputFile(".3gp");
        var recorder = createDefaultRecorder(outputFile);

        var latch = new CountDownLatch(1);
        var receivedWhat = new int[1];
        recorder.setOnInfoListener((mr, what, extra) -> {
            if (what == MediaRecorder.MEDIA_RECORDER_INFO_MAX_FILESIZE_REACHED) {
                receivedWhat[0] = what;
                latch.countDown();
            }
        });
        // Small file size limit to trigger the callback quickly
        recorder.setMaxFileSize(5000);

        try {
            recorder.prepare();
            recorder.start();

            var fired = latch.await(10, TimeUnit.SECONDS);
            assertWithMessage("Max file size callback should fire within 10 seconds")
                    .that(fired).isTrue();
            assertWithMessage("Callback should report MAX_FILESIZE_REACHED")
                    .that(receivedWhat[0])
                    .isEqualTo(MediaRecorder.MEDIA_RECORDER_INFO_MAX_FILESIZE_REACHED);

            recorder.reset();
        } finally {
            recorder.release();
        }

        assertWithMessage("Max-filesize file should not be empty")
                .that(outputFile.length()).isGreaterThan(0L);
    }

    @Test
    public void testMediaRecorder_withoutSpoofing_setAudioSourceFails() throws Exception {
        disableMicSpoofing();

        var recorder = new MediaRecorder(context);
        try {
            RuntimeException thrown = null;
            try {
                recorder.setAudioSource(MediaRecorder.AudioSource.MIC);
            } catch (RuntimeException e) {
                thrown = e;
            }
            assertWithMessage("setAudioSource(MIC) should fail without RECORD_AUDIO "
                    + "permission and without mic spoofing — if this fails, the test "
                    + "app has real permission and no other test in this class is valid")
                    .that(thrown).isNotNull();
        } finally {
            recorder.release();
            enableMicSpoofing();
        }
    }
}
