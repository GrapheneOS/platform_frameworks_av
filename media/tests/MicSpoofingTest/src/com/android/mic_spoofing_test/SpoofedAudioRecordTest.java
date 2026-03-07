package com.android.mic_spoofing_test;

import static com.google.common.truth.Truth.assertThat;
import static com.google.common.truth.Truth.assertWithMessage;

import android.media.AudioDeviceInfo;
import android.media.AudioFormat;
import android.media.AudioManager;
import android.media.AudioRecord;
import android.media.AudioTimestamp;
import android.media.MediaRecorder;

import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.filters.MediumTest;
import androidx.test.platform.app.InstrumentationRegistry;

import org.junit.After;
import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;

import java.nio.ByteBuffer;


@MediumTest
@RunWith(AndroidJUnit4.class)
public class SpoofedAudioRecordTest extends BaseMicSpoofingTest {

    private static final int SAMPLE_RATE = 48000;
    private static final int CHANNEL_CONFIG = AudioFormat.CHANNEL_IN_MONO;
    private static final int ENCODING = AudioFormat.ENCODING_PCM_16BIT;

    // 16-bit mono = 2 bytes
    private static final int FRAME_SIZE = 2;
    private static final int FRAMES_50MS = SAMPLE_RATE / 20;
    private static final int BUFFER_SIZE_BYTES = FRAMES_50MS * FRAME_SIZE;

    @Before
    public void setUp() throws Exception {
        initPackageContext();

        enableMicSpoofing();
    }

    @After
    public void tearDown() throws Exception {
        disableMicSpoofing();
    }

    private static AudioRecord createDefaultRecord() {
        return createRecord(SAMPLE_RATE, CHANNEL_CONFIG, ENCODING);
    }

    private static AudioRecord createRecord(int sampleRate, int channelConfig, int encoding) {
        return createRecord(MediaRecorder.AudioSource.MIC, sampleRate, channelConfig, encoding);
    }

    private static AudioRecord createRecord(
            int audioSource,
            int sampleRate,
            int channelConfig,
            int encoding
    ) {
        var bytesPerSample = getBytesPerSample(encoding);
        var channels = channelConfig == AudioFormat.CHANNEL_IN_STEREO ? 2 : 1;
        var bufferSize = Math.max(sampleRate / 20, 256) * bytesPerSample * channels;
        return new AudioRecord.Builder()
                .setAudioSource(audioSource)
                .setAudioFormat(new AudioFormat.Builder()
                        .setSampleRate(sampleRate)
                        .setChannelMask(channelConfig)
                        .setEncoding(encoding)
                        .build())
                .setBufferSizeInBytes(bufferSize)
                .build();
    }

    private static AudioRecord createImplicitDefaultRecord() {
        var bytesPerSample = getBytesPerSample(ENCODING);
        var bufferSize = Math.max(SAMPLE_RATE / 20, 256) * bytesPerSample;
        return new AudioRecord.Builder()
                .setAudioFormat(new AudioFormat.Builder()
                        .setSampleRate(SAMPLE_RATE)
                        .setChannelMask(CHANNEL_CONFIG)
                        .setEncoding(ENCODING)
                        .build())
                .setBufferSizeInBytes(bufferSize)
                .build();
    }

    private static AudioDeviceInfo getAnyInputDevice() {
        var audioManager = InstrumentationRegistry.getInstrumentation()
                .getTargetContext()
                .getSystemService(AudioManager.class);
        assertThat(audioManager).isNotNull();

        var inputDevices = audioManager.getDevices(AudioManager.GET_DEVICES_INPUTS);
        assertWithMessage("Test requires at least one input device to route to")
                .that(inputDevices.length)
                .isGreaterThan(0);
        return inputDevices[0];
    }

    private static int getBytesPerSample(int encoding) {
        return switch (encoding) {
            case AudioFormat.ENCODING_PCM_8BIT -> 1;
            case AudioFormat.ENCODING_PCM_16BIT -> 2;
            case AudioFormat.ENCODING_PCM_24BIT_PACKED -> 3;
            case AudioFormat.ENCODING_PCM_FLOAT, AudioFormat.ENCODING_PCM_32BIT -> 4;
            default -> throw new IllegalArgumentException("Unknown encoding: " + encoding);
        };
    }

    private static boolean hasNonSilentBytes(byte[] data, int length) {
        return hasNonSilentBytes(data, 0, length);
    }

    private static boolean hasNonSilentBytes(byte[] data, int offset, int length) {
        for (var i = offset; i < offset + length; i++) {
            if (data[i] != 0) return true;
        }
        return false;
    }

    private static boolean hasNonSilentShorts(short[] data, int length) {
        for (var i = 0; i < length; i++) {
            if (data[i] != 0) return true;
        }
        return false;
    }

    private static boolean hasNonSilentFloats(float[] data, int length) {
        for (var i = 0; i < length; i++) {
            if (data[i] != 0.0f) return true;
        }
        return false;
    }

    @Test
    public void testConstructor_spoofedTrack_initializesSuccessfully() {
        var record = createDefaultRecord();
        try {
            assertThat(record.getState()).isEqualTo(AudioRecord.STATE_INITIALIZED);
        } finally {
            record.release();
        }
    }

    @Test
    public void testConstructor_defaultSource_spoofedTrack_initializesSuccessfully() {
        var record = createImplicitDefaultRecord();
        try {
            assertThat(record.getState()).isEqualTo(AudioRecord.STATE_INITIALIZED);
        } finally {
            record.release();
        }
    }

    @Test
    public void testConstructor_spoofedTrack_variousFormats() {
        int[] sampleRates = {8000, 16000, 44100, 48000};
        int[] encodings = {
                AudioFormat.ENCODING_PCM_8BIT,
                AudioFormat.ENCODING_PCM_16BIT,
                AudioFormat.ENCODING_PCM_24BIT_PACKED,
                AudioFormat.ENCODING_PCM_32BIT,
                AudioFormat.ENCODING_PCM_FLOAT,
        };
        int[] channelConfigs = {
                AudioFormat.CHANNEL_IN_MONO,
                AudioFormat.CHANNEL_IN_STEREO,
        };

        for (var sampleRate : sampleRates) {
            for (var encoding : encodings) {
                for (var channelConfig : channelConfigs) {
                    var desc = "sampleRate=" + sampleRate
                            + " encoding=" + encoding
                            + " channels="
                            + (channelConfig == AudioFormat.CHANNEL_IN_STEREO
                            ? "stereo" : "mono");
                    var record = createRecord(sampleRate, channelConfig, encoding);
                    try {
                        assertWithMessage(desc)
                                .that(record.getState())
                                .isEqualTo(AudioRecord.STATE_INITIALIZED);
                    } finally {
                        record.release();
                    }
                }
            }
        }
    }

    @Test
    public void testBuilder_camcorderSource_notSpoofed_throws() {
        UnsupportedOperationException thrown = null;
        try {
            createRecord(MediaRecorder.AudioSource.CAMCORDER, SAMPLE_RATE, CHANNEL_CONFIG,
                    ENCODING);
        } catch (UnsupportedOperationException e) {
            thrown = e;
        }

        assertWithMessage("Builder should reject CAMCORDER when mic spoofing is enabled")
                .that(thrown).isNotNull();
    }

    @Test
    public void testStartRecording_spoofedTrack_succeeds() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            assertThat(record.getRecordingState())
                    .isEqualTo(AudioRecord.RECORDSTATE_RECORDING);
            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testStop_spoofedTrack_succeeds() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            record.stop();
            assertThat(record.getRecordingState())
                    .isEqualTo(AudioRecord.RECORDSTATE_STOPPED);
        } finally {
            record.release();
        }
    }

    @Test
    public void testDoubleStart_spoofedTrack_noError() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            assertThat(record.getRecordingState())
                    .isEqualTo(AudioRecord.RECORDSTATE_RECORDING);

            // Second start while already recording: native sees mActive == true and returns early
            record.startRecording();
            assertThat(record.getRecordingState())
                    .isEqualTo(AudioRecord.RECORDSTATE_RECORDING);

            // Reads should still work normally after double start
            var buffer = new short[FRAMES_50MS];
            var read = record.read(buffer, 0, buffer.length);
            assertThat(read).isEqualTo(FRAMES_50MS);

            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testDoubleStop_spoofedTrack_noError() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            record.stop();
            assertThat(record.getRecordingState()).isEqualTo(AudioRecord.RECORDSTATE_STOPPED);

            // Second stop: native sees !mActive and returns immediately
            record.stop();
            assertThat(record.getRecordingState()).isEqualTo(AudioRecord.RECORDSTATE_STOPPED);
        } finally {
            record.release();
        }
    }

    @Test
    public void testRelease_spoofedTrack_noLeak() {
        var record = createDefaultRecord();
        record.startRecording();
        record.stop();
        record.release();
        assertThat(record.getState()).isEqualTo(AudioRecord.STATE_UNINITIALIZED);
    }

    @Test
    public void testReadBytes_spoofedTrack_returnsRequestedSize() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            var buffer = new byte[BUFFER_SIZE_BYTES];
            var read = record.read(buffer, 0, buffer.length);
            assertThat(read).isEqualTo(buffer.length);
            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testReadBytes_spoofedTrack_producesNonSilentData() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            var buffer = new byte[BUFFER_SIZE_BYTES];
            var read = record.read(buffer, 0, buffer.length);
            assertThat(read).isGreaterThan(0);
            assertWithMessage("Spoofed audio data should not be all-zero")
                    .that(hasNonSilentBytes(buffer, read)).isTrue();
            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testReadBytes_spoofedTrack_withOffset() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            var offset = 100;
            var buffer = new byte[offset + BUFFER_SIZE_BYTES];
            var read = record.read(buffer, offset, BUFFER_SIZE_BYTES);
            assertThat(read).isEqualTo(BUFFER_SIZE_BYTES);

            assertWithMessage("Bytes before offset should be untouched")
                    .that(hasNonSilentBytes(buffer, 0, offset)).isFalse();
            assertWithMessage("Data at offset should be non-silent")
                    .that(hasNonSilentBytes(buffer, offset, read)).isTrue();

            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testReadShorts_spoofedTrack_returnsFrameCount() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            var buffer = new short[FRAMES_50MS];
            var read = record.read(buffer, 0, buffer.length);
            assertThat(read).isEqualTo(buffer.length);
            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testReadShorts_spoofedTrack_producesNonSilentData() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            var buffer = new short[FRAMES_50MS];
            var read = record.read(buffer, 0, buffer.length);
            assertThat(read).isGreaterThan(0);
            assertWithMessage("Spoofed short samples should not be all-zero")
                    .that(hasNonSilentShorts(buffer, read)).isTrue();
            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testReadFloats_spoofedTrack_returnsFrameCount() {
        var record = createRecord(SAMPLE_RATE, CHANNEL_CONFIG, AudioFormat.ENCODING_PCM_FLOAT);

        try {
            record.startRecording();
            var buffer = new float[FRAMES_50MS];
            var read = record.read(buffer, 0, buffer.length, AudioRecord.READ_BLOCKING);
            assertThat(read).isEqualTo(buffer.length);
            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testReadFloats_spoofedTrack_producesNonSilentData() {
        var record = createRecord(SAMPLE_RATE, CHANNEL_CONFIG, AudioFormat.ENCODING_PCM_FLOAT);
        try {
            record.startRecording();
            var buffer = new float[FRAMES_50MS];
            var read = record.read(buffer, 0, buffer.length, AudioRecord.READ_BLOCKING);
            assertThat(read).isGreaterThan(0);
            assertWithMessage("Spoofed float samples should not be all-zero")
                    .that(hasNonSilentFloats(buffer, read)).isTrue();

            for (var i = 0; i < read; i++) {
                assertWithMessage("Float sample[" + i + "] out of range")
                        .that(buffer[i]).isAtLeast(-1.0f);
                assertWithMessage("Float sample[" + i + "] out of range")
                        .that(buffer[i]).isAtMost(1.0f);
            }
            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testReadByteBuffer_spoofedTrack_returnsRequestedSize() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            var buffer = ByteBuffer.allocateDirect(BUFFER_SIZE_BYTES);
            var read = record.read(buffer, BUFFER_SIZE_BYTES);
            assertThat(read).isEqualTo(BUFFER_SIZE_BYTES);
            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testReadNonBlocking_spoofedTrack_pacesCorrectly() throws Exception {
        var record = createDefaultRecord();
        try {
            record.startRecording();

            var buffer = new short[FRAMES_50MS];

            // Non-blocking read immediately after start: elapsed time since
            // mSpoofedStartNs is near-zero, so few or no frames are available
            var readImmediate = record.read(buffer, 0, buffer.length,
                    AudioRecord.READ_NON_BLOCKING);
            // Allow a tiny number of frames from scheduling jitter (~5ms max)
            assertWithMessage("Non-blocking read immediately after start")
                    .that(readImmediate).isAtMost(SAMPLE_RATE / 200 + 1);

            // After 200ms, ~9600 frames should be available
            Thread.sleep(200);

            var readAfterWait = record.read(buffer, 0, buffer.length,
                    AudioRecord.READ_NON_BLOCKING);
            assertWithMessage("Non-blocking read after 200ms wait")
                    .that(readAfterWait).isGreaterThan(0);

            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testReadTiming_spoofedTrack_simulatesRealTime() {
        var record = createDefaultRecord();
        try {
            record.startRecording();

            var framesToRead = SAMPLE_RATE;
            var chunkFrames = FRAMES_50MS;
            var buffer = new short[chunkFrames];

            var startNs = System.nanoTime();
            var totalRead = 0;
            while (totalRead < framesToRead) {
                var toRead = Math.min(chunkFrames, framesToRead - totalRead);
                var read = record.read(buffer, 0, toRead);
                assertThat(read).isGreaterThan(0);
                totalRead += read;
            }
            var elapsedNs = System.nanoTime() - startNs;

            // ~1 second of blocking reads should take ~1 second wall time (20% tolerance)
            var expectedNs = 1_000_000_000L;
            assertWithMessage("1s of blocking reads should take ~1s wall time")
                    .that(elapsedNs).isAtLeast((long) (expectedNs * 0.8));
            assertWithMessage("1s of blocking reads should take ~1s wall time")
                    .that(elapsedNs).isAtMost((long) (expectedNs * 1.2));

            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testCumulativeReads_spoofedTrack_returnConsistentFrameCount() {
        var record = createDefaultRecord();
        try {
            record.startRecording();

            var buffer = new short[FRAMES_50MS];
            var totalFrames = 0;
            var numReads = 5;

            for (var i = 0; i < numReads; i++) {
                var read = record.read(buffer, 0, buffer.length);
                assertWithMessage("Read #" + i)
                        .that(read).isEqualTo(FRAMES_50MS);
                totalFrames += read;
            }

            assertThat(totalFrames).isEqualTo(FRAMES_50MS * numReads);

            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testMultipleReads_spoofedTrack_positionAdvances() {
        // Verify that the internal position advances for spoofed tracks by
        // performing multiple reads and checking that each returns the expected
        // frame count and that consecutive reads produce different audio data
        // (the audio source advances through the WAV file, not returning the
        // same samples repeatedly)
        var record = createDefaultRecord();
        try {
            record.startRecording();

            var reads = new short[4][FRAMES_50MS];
            for (var i = 0; i < reads.length; i++) {
                var read = record.read(reads[i], 0, reads[i].length);
                assertWithMessage("Read %s should return full frame count", i)
                        .that(read).isEqualTo(FRAMES_50MS);
            }

            // Verify at least one pair of consecutive reads differs, proving
            // the audio source position advances between reads
            var anyDiffer = false;
            for (var i = 0; i < reads.length - 1; i++) {
                if (!java.util.Arrays.equals(reads[i], reads[i + 1])) {
                    anyDiffer = true;
                    break;
                }
            }
            assertWithMessage("Consecutive reads should return different data, "
                    + "proving position advances through the spoofed audio source")
                    .that(anyDiffer).isTrue();

            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testGetTimestamp_spoofedTrack_returnsError() {
        var record = createDefaultRecord();
        try {
            record.startRecording();

            var timestamp = new AudioTimestamp();
            var result = record.getTimestamp(timestamp, AudioTimestamp.TIMEBASE_MONOTONIC);
            // Spoofed tracks return INVALID_OPERATION for timestamps since there is no real
            // hardware timeline
            assertThat(result).isEqualTo(AudioRecord.ERROR_INVALID_OPERATION);

            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testMultipleSpoofedRecords_concurrent() throws Exception {
        var record1 = createDefaultRecord();
        var record2 = createRecord(SAMPLE_RATE, AudioFormat.CHANNEL_IN_STEREO,
                AudioFormat.ENCODING_PCM_16BIT);
        try {
            record1.startRecording();
            record2.startRecording();

            var buffer1 = new short[FRAMES_50MS]; // mono
            var buffer2 = new short[FRAMES_50MS * 2]; // stereo: 2 shorts/frame
            var read1 = new int[1];
            var read2 = new int[1];

            var t1 = new Thread(() -> read1[0] = record1.read(buffer1, 0, buffer1.length));
            var t2 = new Thread(() -> read2[0] = record2.read(buffer2, 0, buffer2.length));

            t1.start();
            t2.start();
            t1.join(5000);
            t2.join(5000);

            assertWithMessage("Mono record").that(read1[0]).isEqualTo(buffer1.length);
            assertWithMessage("Stereo record").that(read2[0]).isEqualTo(buffer2.length);
            assertWithMessage("Mono data should be non-silent")
                    .that(hasNonSilentShorts(buffer1, read1[0])).isTrue();
            assertWithMessage("Stereo data should be non-silent")
                    .that(hasNonSilentShorts(buffer2, read2[0])).isTrue();

            record1.stop();
            record2.stop();
        } finally {
            record1.release();
            record2.release();
        }
    }

    @Test
    public void testGetActiveMicrophones_spoofedTrack_returnsAtMostRoutedDevice() throws Exception {
        var record = createDefaultRecord();
        try {
            record.startRecording();

            var mics = record.getActiveMicrophones();
            // The native layer returns an empty list for spoofed tracks
            assertThat(mics).isEmpty();

            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testSetPreferredMicrophoneDirection_spoofedTrack_succeeds() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            var result = record.setPreferredMicrophoneDirection(
                    AudioRecord.MIC_DIRECTION_AWAY_FROM_USER);
            assertThat(result).isTrue();
            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testSetPreferredMicrophoneFieldDimension_spoofedTrack_succeeds() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            var result = record.setPreferredMicrophoneFieldDimension(1.0f);
            assertThat(result).isTrue();
            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testSetPreferredDevice_spoofedTrack_doesNotCrash() {
        var record = createDefaultRecord();
        try {
            var preferredDevice = getAnyInputDevice();

            assertThat(record.setPreferredDevice(preferredDevice)).isTrue();
        } finally {
            record.release();
        }
    }

    @Test
    public void testSetPreferredDevice_activeSpoofedTrack_doesNotCrash() {
        var record = createDefaultRecord();
        try {
            var preferredDevice = getAnyInputDevice();

            record.startRecording();
            assertThat(record.getRecordingState()).isEqualTo(AudioRecord.RECORDSTATE_RECORDING);

            assertThat(record.setPreferredDevice(preferredDevice)).isTrue();

            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testReadBeforeStart_spoofedTrack_returnsZero() {
        var record = createDefaultRecord();
        try {
            var buffer = new byte[BUFFER_SIZE_BYTES];
            var read = record.read(buffer, 0, buffer.length);
            assertThat(read).isEqualTo(0);
        } finally {
            record.release();
        }
    }

    @Test
    public void testReadAfterStop_spoofedTrack_doesNotBlockAndReturnsNonNegative() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            record.stop();

            var buffer = new byte[BUFFER_SIZE_BYTES];
            var read = record.read(buffer, 0, buffer.length, AudioRecord.READ_NON_BLOCKING);
            assertWithMessage("read() after stop() should return 0 for spoofed tracks")
                    .that(read).isEqualTo(0);
            assertWithMessage("read() after stop() should not exceed request")
                    .that(read).isAtMost(buffer.length);
        } finally {
            record.release();
        }
    }

    @Test
    public void testReadSubFrameSize_spoofedTrack_returnsZero() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            var buffer = new byte[BUFFER_SIZE_BYTES];
            var read = record.read(buffer, 0, 1);
            assertThat(read).isEqualTo(0);
            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testReadZeroBytes_spoofedTrack_returnsZero() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            var buffer = new byte[BUFFER_SIZE_BYTES];
            var read = record.read(buffer, 0, 0);
            assertThat(read).isEqualTo(0);
            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testStartStopRestart_spoofedTrack_producesFreshData() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            var buffer1 = new short[FRAMES_50MS];
            var read1 = record.read(buffer1, 0, buffer1.length);
            assertThat(read1).isEqualTo(FRAMES_50MS);
            record.stop();

            record.startRecording();
            assertThat(record.getRecordingState()).isEqualTo(AudioRecord.RECORDSTATE_RECORDING);
            var buffer2 = new short[FRAMES_50MS];
            var read2 = record.read(buffer2, 0, buffer2.length);
            assertThat(read2).isEqualTo(FRAMES_50MS);
            assertWithMessage("Data after restart should be non-silent")
                    .that(hasNonSilentShorts(buffer2, read2)).isTrue();
            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testSmallBuffer_setPositionNotificationPeriod_doesNotCrash() {
        var smallBufferBytes = Math.max(
                AudioRecord.getMinBufferSize(SAMPLE_RATE, CHANNEL_CONFIG, ENCODING),
                FRAME_SIZE * 64);
        var record = new AudioRecord.Builder()
                .setAudioSource(MediaRecorder.AudioSource.MIC)
                .setAudioFormat(new AudioFormat.Builder()
                        .setSampleRate(SAMPLE_RATE)
                        .setChannelMask(CHANNEL_CONFIG)
                        .setEncoding(ENCODING)
                        .build())
                .setBufferSizeInBytes(smallBufferBytes)
                .build();
        try {
            assertThat(record.getState()).isEqualTo(AudioRecord.STATE_INITIALIZED);

            record.setPositionNotificationPeriod(FRAMES_50MS);

            record.startRecording();
            var buffer = new short[FRAMES_50MS];
            var read = record.read(buffer, 0, buffer.length);
            assertThat(read).isGreaterThan(0);

            var hasNonZero = false;
            for (var i = 0; i < read; i++) {
                if (buffer[i] != 0) {
                    hasNonZero = true;
                    break;
                }
            }
            assertWithMessage("Spoofed audio should contain non-silent data")
                    .that(hasNonZero).isTrue();

            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testConstructor_nonPcmFormat_notSpoofed() {
        var record = new AudioRecord(
                MediaRecorder.AudioSource.MIC,
                SAMPLE_RATE,
                CHANNEL_CONFIG,
                AudioFormat.ENCODING_E_AC3_JOC,
                BUFFER_SIZE_BYTES
        );

        try {
            assertThat(record.getState()).isEqualTo(AudioRecord.STATE_UNINITIALIZED);
        } finally {
            record.release();
        }
    }
}
