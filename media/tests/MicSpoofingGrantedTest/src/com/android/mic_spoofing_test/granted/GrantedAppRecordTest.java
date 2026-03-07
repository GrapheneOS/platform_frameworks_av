package com.android.mic_spoofing_test.granted;

import static com.google.common.truth.Truth.assertThat;
import static com.google.common.truth.Truth.assertWithMessage;

import android.Manifest;
import android.app.AppOpsManager;
import android.content.Context;
import android.content.pm.GosPackageState;
import android.content.pm.PackageManager;
import android.content.pm.spoofing.MicSpoofing;
import android.media.AudioFormat;
import android.media.AudioRecord;
import android.media.MediaExtractor;
import android.media.MediaFormat;
import android.media.MediaRecorder;
import android.os.ParcelFileDescriptor;
import android.os.Process;
import android.os.UserHandle;

import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.filters.MediumTest;
import androidx.test.platform.app.InstrumentationRegistry;

import org.junit.After;
import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;

import java.io.File;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.List;

@MediumTest
@RunWith(AndroidJUnit4.class)
public class GrantedAppRecordTest {

    private static final int SAMPLE_RATE = 48000;
    private static final int CHANNEL_CONFIG = AudioFormat.CHANNEL_IN_MONO;
    private static final int ENCODING = AudioFormat.ENCODING_PCM_16BIT;

    // 16-bit mono = 2 bytes
    private static final int FRAME_SIZE = 2;
    private static final int FRAMES_50MS = SAMPLE_RATE / 20;
    private static final int BUFFER_SIZE_BYTES = FRAMES_50MS * FRAME_SIZE;

    private static final int RECORD_DURATION_MS = 2000;

    private Context context;
    private String packageName;
    private int userId;
    private final List<File> outputFiles = new ArrayList<>();

    @Before
    public void setUp() {
        context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        packageName = context.getPackageName();
        userId = UserHandle.myUserId();

        assertWithMessage("RECORD_AUDIO must be granted for this test APK to function."
                + " Verify that AndroidTest.xml does NOT revoke the permission.")
                .that(context.checkSelfPermission(Manifest.permission.RECORD_AUDIO))
                .isEqualTo(PackageManager.PERMISSION_GRANTED);
    }

    @After
    public void tearDown() throws Exception {
        disableMicSpoofing();

        for (File file : outputFiles) {
            file.delete();
        }
        outputFiles.clear();
    }

    private void enableMicSpoofing() throws IOException {
        runShellCommand("pm edit-gos-package-state " + packageName + " " + userId
                + " add-flag MIC_SPOOFING_ENABLED");
        GosPackageState state = GosPackageState.get(packageName, userId);
        MicSpoofing.onGosPackageStateChanged(state);
    }

    private void disableMicSpoofing() throws IOException {
        runShellCommand("pm edit-gos-package-state " + packageName + " " + userId
                + " clear-flag MIC_SPOOFING_ENABLED");
        MicSpoofing.onGosPackageStateChanged(GosPackageState.DEFAULT);
    }

    private static void runShellCommand(String command) throws IOException {
        var parcelFileDescriptor = InstrumentationRegistry.getInstrumentation()
                .getUiAutomation().executeShellCommand(command);
        try (var is = new ParcelFileDescriptor.AutoCloseInputStream(parcelFileDescriptor)) {
            is.readAllBytes();
        }
    }

    private static AudioRecord createDefaultRecord() {
        return createRecord(SAMPLE_RATE, CHANNEL_CONFIG, ENCODING);
    }

    private static AudioRecord createRecord(int sampleRate, int channelConfig, int encoding) {
        var bytesPerSample = getBytesPerSample(encoding);
        var channels = channelConfig == AudioFormat.CHANNEL_IN_STEREO ? 2 : 1;
        var bufferSize = Math.max(sampleRate / 20, 256) * bytesPerSample * channels;
        return new AudioRecord.Builder()
                .setAudioSource(MediaRecorder.AudioSource.MIC)
                .setAudioFormat(new AudioFormat.Builder()
                        .setSampleRate(sampleRate)
                        .setChannelMask(channelConfig)
                        .setEncoding(encoding)
                        .build())
                .setBufferSizeInBytes(bufferSize)
                .build();
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

    private File createOutputFile(String suffix) throws IOException {
        var file = File.createTempFile("granted_record_", suffix, context.getCacheDir());
        outputFiles.add(file);
        return file;
    }

    private File recordWithMediaRecorder(int durationMs) throws Exception {
        var outputFile = createOutputFile(".mp4");

        var recorder = new MediaRecorder(context);
        try {
            recorder.setAudioSource(MediaRecorder.AudioSource.MIC);
            recorder.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4);
            recorder.setAudioEncoder(MediaRecorder.AudioEncoder.AAC);
            recorder.setAudioSamplingRate(44100);
            recorder.setAudioChannels(1);
            recorder.setAudioEncodingBitRate(64000);
            recorder.setOutputFile(outputFile.getAbsolutePath());
            recorder.prepare();
            recorder.start();

            Thread.sleep(durationMs);

            recorder.stop();
        } finally {
            recorder.release();
        }
        return outputFile;
    }

    @Test
    public void testMicSpoofing_notEnabled_byDefault() {
        assertWithMessage("MicSpoofing should not be enabled for an app with real permission")
                .that(MicSpoofing.isEnabled()).isFalse();
    }

    @Test
    public void testMicSpoofing_shouldSpoofSelfPermCheck_returnsFalse() {
        assertWithMessage("shouldSpoofSelfPermissionCheck should return false")
                .that(MicSpoofing.shouldSpoofSelfPermissionCheck(
                        Manifest.permission.RECORD_AUDIO))
                .isFalse();
    }

    @Test
    public void testMicSpoofing_shouldSpoofSelfAppOpCheck_returnsFalse() {
        assertWithMessage("shouldSpoofSelfAppOpCheck should return false")
                .that(MicSpoofing.shouldSpoofSelfAppOpCheck(AppOpsManager.OP_RECORD_AUDIO))
                .isFalse();
    }

    @Test
    public void testCheckSelfPermission_recordAudio_isReallyGranted() {
        assertWithMessage("MicSpoofing should not be active")
                .that(MicSpoofing.isEnabled()).isFalse();

        var result = context.checkSelfPermission(Manifest.permission.RECORD_AUDIO);
        assertWithMessage("RECORD_AUDIO should be genuinely granted (not spoofed)")
                .that(result).isEqualTo(PackageManager.PERMISSION_GRANTED);
    }

    @Test
    public void testAppOpsManager_checkOp_recordAudio_isAllowed() {
        assertWithMessage("MicSpoofing should not be active")
                .that(MicSpoofing.isEnabled()).isFalse();

        var appOps = context.getSystemService(AppOpsManager.class);
        var mode = appOps.checkOpNoThrow(
                AppOpsManager.OPSTR_RECORD_AUDIO, Process.myUid(), packageName);

        assertWithMessage("OP_RECORD_AUDIO should be allowed via real permission")
                .that(mode).isEqualTo(AppOpsManager.MODE_ALLOWED);
    }

    @Test
    public void testPackageManager_checkPermission_recordAudio_isGranted() {
        assertWithMessage("MicSpoofing should not be active")
                .that(MicSpoofing.isEnabled()).isFalse();

        var result = context.getPackageManager()
                .checkPermission(Manifest.permission.RECORD_AUDIO, packageName);

        assertWithMessage("PackageManager.checkPermission should report real GRANTED")
                .that(result).isEqualTo(PackageManager.PERMISSION_GRANTED);
    }

    @Test
    public void testCheckPermission_recordAudio_withPidUid_isGranted() {
        assertWithMessage("MicSpoofing should not be active")
                .that(MicSpoofing.isEnabled()).isFalse();

        var result = context.checkPermission(Manifest.permission.RECORD_AUDIO,
                Process.myPid(), Process.myUid());

        assertWithMessage("checkPermission(RECORD_AUDIO, myPid, myUid) should be real GRANTED")
                .that(result).isEqualTo(PackageManager.PERMISSION_GRANTED);
    }

    @Test
    public void testAppOpsManager_noteOp_recordAudio_isAllowed() {
        assertWithMessage("MicSpoofing should not be active")
                .that(MicSpoofing.isEnabled()).isFalse();

        var appOps = context.getSystemService(AppOpsManager.class);
        var mode = appOps.noteOpNoThrow(
                AppOpsManager.OPSTR_RECORD_AUDIO, Process.myUid(), packageName);

        assertWithMessage("noteOp(OP_RECORD_AUDIO) should be allowed via real permission")
                .that(mode).isEqualTo(AppOpsManager.MODE_ALLOWED);
    }

    @Test
    public void testAppOpsManager_unsafeCheckOpRaw_recordAudio_isAllowed() {
        assertWithMessage("MicSpoofing should not be active")
                .that(MicSpoofing.isEnabled()).isFalse();

        var appOps = context.getSystemService(AppOpsManager.class);
        var mode = appOps.unsafeCheckOpRawNoThrow(
                AppOpsManager.OPSTR_RECORD_AUDIO, Process.myUid(), packageName);

        assertWithMessage("unsafeCheckOpRaw(OP_RECORD_AUDIO) should indicate permission"
                + " granted (MODE_ALLOWED or MODE_FOREGROUND)")
                .that(mode).isAnyOf(AppOpsManager.MODE_ALLOWED,
                        AppOpsManager.MODE_FOREGROUND);
    }

    @Test
    public void testAudioRecord_withPermission_initializesSuccessfully() {
        var record = createDefaultRecord();
        try {
            assertThat(record.getState()).isEqualTo(AudioRecord.STATE_INITIALIZED);
        } finally {
            record.release();
        }
    }

    @Test
    public void testAudioRecord_withPermission_usesRealTrack() {
        assertWithMessage("MicSpoofing should not be enabled")
                .that(MicSpoofing.isEnabled()).isFalse();

        var record = createDefaultRecord();
        try {
            assertWithMessage("AudioRecord should initialize with real track")
                    .that(record.getState())
                    .isEqualTo(AudioRecord.STATE_INITIALIZED);

            record.startRecording();
            assertWithMessage("Recording should start successfully")
                    .that(record.getRecordingState())
                    .isEqualTo(AudioRecord.RECORDSTATE_RECORDING);

            var buffer = new short[FRAMES_50MS];
            var read = record.read(buffer, 0, buffer.length);
            assertWithMessage("read() should return requested frame count")
                    .that(read).isEqualTo(FRAMES_50MS);

            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testAudioRecord_withPermission_startStopLifecycle() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            assertThat(record.getRecordingState())
                    .isEqualTo(AudioRecord.RECORDSTATE_RECORDING);

            var buffer = new short[FRAMES_50MS];
            var read = record.read(buffer, 0, buffer.length);
            assertThat(read).isGreaterThan(0);

            record.stop();
            assertThat(record.getRecordingState())
                    .isEqualTo(AudioRecord.RECORDSTATE_STOPPED);

            record.release();
            assertThat(record.getState())
                    .isEqualTo(AudioRecord.STATE_UNINITIALIZED);
        } finally {
            record.release();
        }
    }

    @Test
    public void testAudioRecord_withPermission_readBytesReturnsData() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            var buffer = new byte[BUFFER_SIZE_BYTES];
            var read = record.read(buffer, 0, buffer.length);
            assertWithMessage("byte[] read should return requested size")
                    .that(read).isEqualTo(buffer.length);
            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testAudioRecord_withPermission_readShortsReturnsData() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            var buffer = new short[FRAMES_50MS];
            var read = record.read(buffer, 0, buffer.length);
            assertWithMessage("short[] read should return requested frame count")
                    .that(read).isEqualTo(FRAMES_50MS);
            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testAudioRecord_withPermission_readFloatsReturnsData() {
        var record = createRecord(SAMPLE_RATE, CHANNEL_CONFIG,
                AudioFormat.ENCODING_PCM_FLOAT);
        try {
            record.startRecording();
            var buffer = new float[FRAMES_50MS];
            var read = record.read(buffer, 0, buffer.length, AudioRecord.READ_BLOCKING);
            assertWithMessage("float[] read should return requested frame count")
                    .that(read).isEqualTo(FRAMES_50MS);
            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testAudioRecord_withPermission_readByteBufferReturnsData() {
        var record = createDefaultRecord();
        try {
            record.startRecording();
            var buffer = ByteBuffer.allocateDirect(BUFFER_SIZE_BYTES);
            var read = record.read(buffer, BUFFER_SIZE_BYTES);
            assertWithMessage("ByteBuffer read should return requested size")
                    .that(read).isEqualTo(BUFFER_SIZE_BYTES);
            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testAudioRecord_withPermission_variousFormatsInitialize() {
        int[] sampleRates = {8000, 16000, 44100, 48000};
        int[] encodings = {
                AudioFormat.ENCODING_PCM_8BIT,
                AudioFormat.ENCODING_PCM_16BIT,
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
    public void testAudioRecord_withPermission_cumulativeReadsReturnConsistentCount() {
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
    public void testAudioRecord_withPermissionAndSpoofingFlag_initializesSuccessfully()
            throws Exception {
        enableMicSpoofing();

        assertWithMessage("MicSpoofing.isEnabled() should be true after setting flag")
                .that(MicSpoofing.isEnabled()).isTrue();

        var record = createDefaultRecord();
        try {
            assertWithMessage("AudioRecord should still initialize with real permission"
                    + " even when spoofing flag is set")
                    .that(record.getState())
                    .isEqualTo(AudioRecord.STATE_INITIALIZED);
        } finally {
            record.release();
        }
    }

    @Test
    public void testAudioRecord_withPermissionAndSpoofingFlag_readReturnsData()
            throws Exception {
        enableMicSpoofing();

        var record = createDefaultRecord();
        try {
            record.startRecording();

            var buffer = new short[FRAMES_50MS];
            var read = record.read(buffer, 0, buffer.length);
            assertWithMessage("read() should succeed with real permission + spoofing flag")
                    .that(read).isEqualTo(FRAMES_50MS);

            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testCheckSelfPermission_withSpoofingFlag_stillGranted() throws Exception {
        enableMicSpoofing();

        var result = context.checkSelfPermission(Manifest.permission.RECORD_AUDIO);
        assertWithMessage("RECORD_AUDIO should remain GRANTED with both real permission"
                + " and spoofing flag set")
                .that(result).isEqualTo(PackageManager.PERMISSION_GRANTED);
    }

    @Test
    public void testAppOpsManager_withSpoofingFlag_stillAllowed() throws Exception {
        enableMicSpoofing();

        var appOps = context.getSystemService(AppOpsManager.class);
        var mode = appOps.checkOpNoThrow(
                AppOpsManager.OPSTR_RECORD_AUDIO, Process.myUid(), packageName);

        assertWithMessage("OP_RECORD_AUDIO should remain MODE_ALLOWED with both real"
                + " permission and spoofing flag set")
                .that(mode).isEqualTo(AppOpsManager.MODE_ALLOWED);
    }

    @Test
    public void testAppOpsManager_noteOp_withSpoofingFlag_stillAllowed() throws Exception {
        enableMicSpoofing();

        var appOps = context.getSystemService(AppOpsManager.class);
        var mode = appOps.noteOpNoThrow(
                AppOpsManager.OPSTR_RECORD_AUDIO, Process.myUid(), packageName);

        assertWithMessage("noteOp(OP_RECORD_AUDIO) should remain MODE_ALLOWED with both"
                + " real permission and spoofing flag set")
                .that(mode).isEqualTo(AppOpsManager.MODE_ALLOWED);
    }

    @Test
    public void testAudioRecord_afterSpoofingFlagCleared_stillWorks() throws Exception {
        enableMicSpoofing();
        assertThat(MicSpoofing.isEnabled()).isTrue();

        disableMicSpoofing();
        assertThat(MicSpoofing.isEnabled()).isFalse();

        var record = createDefaultRecord();
        try {
            assertThat(record.getState()).isEqualTo(AudioRecord.STATE_INITIALIZED);

            record.startRecording();
            var buffer = new short[FRAMES_50MS];
            var read = record.read(buffer, 0, buffer.length);
            assertWithMessage("read() should succeed after spoofing flag toggle")
                    .that(read).isEqualTo(FRAMES_50MS);

            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testMediaRecorder_withPermission_producesOutputFile() throws Exception {
        assertWithMessage("MicSpoofing should not be active")
                .that(MicSpoofing.isEnabled()).isFalse();

        var outputFile = recordWithMediaRecorder(RECORD_DURATION_MS);

        assertWithMessage("Output file should exist")
                .that(outputFile.exists()).isTrue();
        assertWithMessage("Output file should be non-empty")
                .that(outputFile.length()).isGreaterThan(0);
    }

    @Test
    public void testMediaRecorder_withPermissionAndSpoofingFlag_producesOutputFile()
            throws Exception {
        enableMicSpoofing();

        var outputFile = recordWithMediaRecorder(RECORD_DURATION_MS);

        assertWithMessage("MediaRecorder should produce output with real permission"
                + " even when spoofing flag is set")
                .that(outputFile.exists()).isTrue();
        assertWithMessage("Output file should be non-empty")
                .that(outputFile.length()).isGreaterThan(0);
    }

    @Test
    public void testMediaRecorder_withPermission_outputContainsAudioTrack() throws Exception {
        assertWithMessage("MicSpoofing should not be active")
                .that(MicSpoofing.isEnabled()).isFalse();

        var outputFile = recordWithMediaRecorder(RECORD_DURATION_MS);

        var extractor = new MediaExtractor();
        try {
            extractor.setDataSource(outputFile.getAbsolutePath());

            var foundAudio = false;
            for (var i = 0; i < extractor.getTrackCount(); i++) {
                var format = extractor.getTrackFormat(i);
                var mime = format.getString(MediaFormat.KEY_MIME);
                if (mime != null && mime.startsWith("audio/")) {
                    foundAudio = true;
                    break;
                }
            }
            assertWithMessage("Output should contain an audio track")
                    .that(foundAudio).isTrue();
        } finally {
            extractor.release();
        }
    }
}
