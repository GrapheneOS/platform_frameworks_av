package com.android.mic_spoofing_test;

import static com.google.common.truth.Truth.assertThat;
import static com.google.common.truth.Truth.assertWithMessage;

import android.Manifest;
import android.app.ActivityManager;
import android.app.AppOpsManager;
import android.content.Context;
import android.content.Intent;
import android.content.pm.GosPackageState;
import android.content.pm.PackageManager;
import android.content.pm.ServiceInfo;
import android.content.pm.spoofing.MicSpoofing;
import android.media.AudioFormat;
import android.media.AudioRecord;
import android.media.MediaRecorder;
import android.os.ParcelFileDescriptor;
import android.os.Process;
import android.os.UserHandle;

import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.filters.LargeTest;
import androidx.test.platform.app.InstrumentationRegistry;

import org.junit.After;
import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;

import java.io.IOException;
import java.util.concurrent.TimeUnit;

@LargeTest
@RunWith(AndroidJUnit4.class)
public class MicSpoofingPermissionIntegrationTest {

    private Context context;
    private String packageName;
    private int userId;

    @Before
    public void setUp() throws Exception {
        context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        packageName = context.getPackageName();
        userId = UserHandle.myUserId();

        enableMicSpoofing();
    }

    @After
    public void tearDown() throws Exception {
        context.stopService(new Intent(context, MicSpoofingFgsHelper.class));
        context.stopService(new Intent(context, MicSpoofingSystemExemptedFgsHelper.class));
        disableMicSpoofing();
    }

    private void enableMicSpoofing() throws IOException {
        runShellCommand("pm edit-gos-package-state " + packageName + " " + userId
                + " add-flag MIC_SPOOFING_ENABLED");
        var state = GosPackageState.get(packageName, userId);
        MicSpoofing.onGosPackageStateChanged(state);
    }

    private void disableMicSpoofing() throws IOException {
        runShellCommand("pm edit-gos-package-state " + packageName + " " + userId
                + " clear-flag MIC_SPOOFING_ENABLED");
        MicSpoofing.onGosPackageStateChanged(GosPackageState.DEFAULT);
    }

    private static void runShellCommand(String command) throws IOException {
        var parcelFileDescriptor = InstrumentationRegistry
                .getInstrumentation()
                .getUiAutomation()
                .executeShellCommand(command);

        try (var is = new ParcelFileDescriptor.AutoCloseInputStream(parcelFileDescriptor)) {
            is.readAllBytes(); // block until the command completes
        }
    }

    private static boolean hasNonSilentBytes(byte[] data, int length) {
        for (var i = 0; i < length; i++) {
            if (data[i] != 0) {
                return true;
            }
        }
        return false;
    }

    private Throwable startFgsAndWait() throws Exception {
        MicSpoofingFgsHelper.resetLatch();

        var intent = new Intent(context, MicSpoofingFgsHelper.class);
        intent.setAction(MicSpoofingFgsHelper.ACTION_START_MIC_FGS);

        context.startForegroundService(intent);

        var completed = MicSpoofingFgsHelper.latch.await(10, TimeUnit.SECONDS);
        assertWithMessage("FGS helper should respond within timeout")
                .that(completed).isTrue();

        return MicSpoofingFgsHelper.startForegroundError;
    }

    private Throwable startManifestFgsAndWait() throws Exception {
        MicSpoofingFgsHelper.resetLatch();

        var intent = new Intent(context, MicSpoofingFgsHelper.class);
        intent.setAction(MicSpoofingFgsHelper.ACTION_START_MIC_FGS_MANIFEST);

        context.startForegroundService(intent);

        var completed = MicSpoofingFgsHelper.latch.await(10, TimeUnit.SECONDS);
        assertWithMessage("FGS helper should respond within timeout")
                .that(completed).isTrue();

        return MicSpoofingFgsHelper.startForegroundError;
    }

    private Throwable startSystemExemptedFgsAndWait() throws Exception {
        MicSpoofingSystemExemptedFgsHelper.resetLatch();

        var intent = new Intent(context, MicSpoofingSystemExemptedFgsHelper.class);
        intent.setAction(MicSpoofingSystemExemptedFgsHelper.ACTION_START_SYSTEM_EXEMPTED_FGS);

        context.startForegroundService(intent);

        var completed = MicSpoofingSystemExemptedFgsHelper.latch.await(10, TimeUnit.SECONDS);
        assertWithMessage("systemExempted FGS helper should respond within timeout")
                .that(completed).isTrue();
        return MicSpoofingSystemExemptedFgsHelper.startForegroundError;
    }

    @Test
    public void testCheckSelfPermission_recordAudio_returnGranted() {
        var result = context.checkSelfPermission(Manifest.permission.RECORD_AUDIO);

        assertWithMessage("checkSelfPermission(RECORD_AUDIO) should be spoofed to GRANTED")
                .that(result).isEqualTo(PackageManager.PERMISSION_GRANTED);
    }

    @Test
    public void testCheckSelfPermission_otherPermission_notAffected() {
        var result = context.checkSelfPermission(Manifest.permission.CAMERA);

        assertWithMessage("checkSelfPermission(CAMERA) should not be affected by mic spoofing")
                .that(result).isEqualTo(PackageManager.PERMISSION_DENIED);
    }

    @Test
    public void testCheckSelfPermission_recordAudio_withSpoofingDisabled_returnDenied()
            throws Exception {
        disableMicSpoofing();

        var result = context.checkSelfPermission(Manifest.permission.RECORD_AUDIO);

        assertWithMessage("Without spoofing, RECORD_AUDIO should be DENIED")
                .that(result).isEqualTo(PackageManager.PERMISSION_DENIED);
    }

    @Test
    public void testCheckSelfPermission_fineLocation_notAffected() {
        var result = context.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION);

        assertWithMessage("ACCESS_FINE_LOCATION should not be affected by mic spoofing")
                .that(result).isEqualTo(PackageManager.PERMISSION_DENIED);
    }

    @Test
    public void testCheckPermission_recordAudio_withPidUid_returnGranted() {
        var result = context.checkPermission(Manifest.permission.RECORD_AUDIO, Process.myPid(),
                Process.myUid());

        assertWithMessage("checkPermission(RECORD_AUDIO, myPid, myUid) should be spoofed")
                .that(result).isEqualTo(PackageManager.PERMISSION_GRANTED);
    }

    @Test
    public void testPackageManager_checkPermission_recordAudio_returnGranted() {
        var result = context.getPackageManager().checkPermission(Manifest.permission.RECORD_AUDIO,
                packageName);

        assertWithMessage("PackageManager.checkPermission(RECORD_AUDIO) should be spoofed")
                .that(result).isEqualTo(PackageManager.PERMISSION_GRANTED);
    }

    @Test
    public void testPackageManager_checkPermission_camera_notAffected() {
        var result = context.getPackageManager().checkPermission(Manifest.permission.CAMERA,
                packageName);

        assertWithMessage("PackageManager.checkPermission(CAMERA) should not be spoofed")
                .that(result).isEqualTo(PackageManager.PERMISSION_DENIED);
    }

    @Test
    public void testAppOpsManager_checkOp_recordAudio_returnAllowed() {
        var appOpsManager = context.getSystemService(AppOpsManager.class);
        var mode = appOpsManager.checkOpNoThrow(AppOpsManager.OPSTR_RECORD_AUDIO, Process.myUid(),
                packageName);

        assertWithMessage("OP_RECORD_AUDIO should be spoofed to MODE_ALLOWED")
                .that(mode).isEqualTo(AppOpsManager.MODE_ALLOWED);
    }

    @Test
    public void testAppOpsManager_checkOp_camera_notAffected() {
        var appOpsManager = context.getSystemService(AppOpsManager.class);
        var mode = appOpsManager.checkOpNoThrow(AppOpsManager.OPSTR_CAMERA, Process.myUid(),
                packageName);

        assertWithMessage("OP_CAMERA should not be affected by mic spoofing")
                .that(mode).isNotEqualTo(AppOpsManager.MODE_ALLOWED);
    }

    @Test
    public void testAppOpsManager_noteOp_recordAudio_returnAllowed() {
        var appOpsManager = context.getSystemService(AppOpsManager.class);
        var mode = appOpsManager.noteOpNoThrow(AppOpsManager.OPSTR_RECORD_AUDIO, Process.myUid(),
                packageName);

        assertWithMessage("noteOp(OP_RECORD_AUDIO) should be spoofed to MODE_ALLOWED")
                .that(mode).isEqualTo(AppOpsManager.MODE_ALLOWED);
    }

    @Test
    public void testAppOpsManager_unsafeCheckOpRaw_recordAudio_returnAllowed() {
        var appOpsManager = context.getSystemService(AppOpsManager.class);
        var mode = appOpsManager.unsafeCheckOpRawNoThrow(AppOpsManager.OPSTR_RECORD_AUDIO,
                Process.myUid(),
                packageName);

        assertWithMessage("unsafeCheckOpRaw(OP_RECORD_AUDIO) should be spoofed to MODE_ALLOWED")
                .that(mode).isEqualTo(AppOpsManager.MODE_ALLOWED);
    }

    @Test
    public void testAppOpsManager_checkOp_recordAudio_withSpoofingDisabled_notAllowed()
            throws Exception {
        disableMicSpoofing();

        var appOpsManager = context.getSystemService(AppOpsManager.class);
        var mode = appOpsManager.checkOpNoThrow(AppOpsManager.OPSTR_RECORD_AUDIO, Process.myUid(),
                packageName);

        assertWithMessage("Without spoofing, OP_RECORD_AUDIO should not be MODE_ALLOWED")
                .that(mode).isNotEqualTo(AppOpsManager.MODE_ALLOWED);
    }

    @Test
    public void testAppOpsManager_noteOp_recordAudio_withSpoofingDisabled_notAllowed()
            throws Exception {
        disableMicSpoofing();

        var appOps = context.getSystemService(AppOpsManager.class);
        var mode = appOps.noteOpNoThrow(AppOpsManager.OPSTR_RECORD_AUDIO, Process.myUid(),
                packageName);

        assertWithMessage("Without spoofing, noteOp(OP_RECORD_AUDIO) should not be MODE_ALLOWED")
                .that(mode).isNotEqualTo(AppOpsManager.MODE_ALLOWED);
    }

    @Test
    public void testFgsTypeMicrophone_spoofed_startsSuccessfully() throws Exception {
        var error = startFgsAndWait();

        assertWithMessage("startForeground(FOREGROUND_SERVICE_TYPE_MICROPHONE) should succeed"
                + " with mic spoofing enabled")
                .that(error).isNull();
    }

    @Test
    public void testFgsTypeMicrophone_spoofed_serviceRunsInForeground() throws Exception {
        var error = startFgsAndWait();
        assertWithMessage("startForeground should succeed").that(error).isNull();

        Thread.sleep(500);

        var activityManager = context.getSystemService(ActivityManager.class);
        var services = activityManager.getRunningServices(100);

        var foundForeground = false;
        for (var info : services) {
            if (MicSpoofingFgsHelper.class.getName().equals(info.service.getClassName())) {
                assertWithMessage("Service should be in foreground")
                        .that(info.foreground).isTrue();
                foundForeground = true;
                break;
            }
        }
        assertWithMessage("MicSpoofingFgsHelper should be in the running services list")
                .that(foundForeground).isTrue();
    }

    @Test
    public void testFgsTypeMicrophone_spoofed_effectiveTypeIsSystemExempted() throws Exception {
        var error = startFgsAndWait();

        assertWithMessage("startForeground should succeed").that(error).isNull();
        assertWithMessage("Mic spoofing FGS should run as SYSTEM_EXEMPTED internally")
                .that(MicSpoofingFgsHelper.lastForegroundServiceType)
                .isEqualTo(ServiceInfo.FOREGROUND_SERVICE_TYPE_SYSTEM_EXEMPTED);
    }

    @Test
    public void testFgsTypeManifest_spoofed_effectiveTypeIsSystemExempted() throws Exception {
        var error = startManifestFgsAndWait();

        assertWithMessage("startForeground(manifest) should succeed").that(error).isNull();
        assertWithMessage("Manifest microphone FGS should run as SYSTEM_EXEMPTED internally")
                .that(MicSpoofingFgsHelper.lastForegroundServiceType)
                .isEqualTo(ServiceInfo.FOREGROUND_SERVICE_TYPE_SYSTEM_EXEMPTED);
    }

    @Test
    public void testFgsTypeMicrophone_notSpoofed_throwsSecurityException() throws Exception {
        disableMicSpoofing();

        MicSpoofingFgsHelper.resetLatch();

        var intent = new Intent(context, MicSpoofingFgsHelper.class);
        intent.setAction(MicSpoofingFgsHelper.ACTION_START_MIC_FGS);

        context.startForegroundService(intent);

        var completed = MicSpoofingFgsHelper.latch.await(10, TimeUnit.SECONDS);
        assertWithMessage("FGS helper should respond within timeout")
                .that(completed).isTrue();

        var error = MicSpoofingFgsHelper.startForegroundError;
        assertWithMessage("startForeground() should throw without mic spoofing")
                .that(error).isNotNull();
        assertWithMessage("Error should be SecurityException")
                .that(error).isInstanceOf(SecurityException.class);
    }

    @Test
    public void testFgsTypeMicrophone_notSpoofed_normalPermInsufficientAlone() throws Exception {
        var result = context.checkSelfPermission(Manifest.permission.FOREGROUND_SERVICE_MICROPHONE);
        assertWithMessage("FOREGROUND_SERVICE_MICROPHONE should be auto-granted (normal perm)")
                .that(result).isEqualTo(PackageManager.PERMISSION_GRANTED);

        result = context.checkSelfPermission(Manifest.permission.FOREGROUND_SERVICE);
        assertWithMessage("FOREGROUND_SERVICE should be auto-granted (normal perm)")
                .that(result).isEqualTo(PackageManager.PERMISSION_GRANTED);

        disableMicSpoofing();

        MicSpoofingFgsHelper.resetLatch();

        var intent = new Intent(context, MicSpoofingFgsHelper.class);
        intent.setAction(MicSpoofingFgsHelper.ACTION_START_MIC_FGS);

        context.startForegroundService(intent);

        var completed = MicSpoofingFgsHelper.latch.await(10, TimeUnit.SECONDS);
        assertWithMessage("FGS helper should respond within timeout")
                .that(completed).isTrue();

        var error = MicSpoofingFgsHelper.startForegroundError;
        assertWithMessage("FOREGROUND_SERVICE_MICROPHONE alone is not sufficient — "
                + "RECORD_AUDIO or mic spoofing is also required")
                .that(error).isNotNull();
        assertWithMessage("Error should be SecurityException")
                .that(error).isInstanceOf(SecurityException.class);
    }

    @Test
    public void testFgsTypeSystemExempted_directRequestSucceeds() throws Exception {
        var error = startSystemExemptedFgsAndWait();

        assertWithMessage("Direct SYSTEM_EXEMPTED request should succeed")
                .that(error).isNull();
    }

    @Test
    public void testFgsTypeMicrophone_spoofed_canRecordAudio() throws Exception {
        var fgsError = startFgsAndWait();
        assertWithMessage("FGS should start successfully").that(fgsError).isNull();

        var permResult = context.checkSelfPermission(Manifest.permission.RECORD_AUDIO);
        assertWithMessage("RECORD_AUDIO should be spoofed to GRANTED during FGS")
                .that(permResult).isEqualTo(PackageManager.PERMISSION_GRANTED);

        var appOpsManager = context.getSystemService(AppOpsManager.class);
        var opResult = appOpsManager.checkOpNoThrow(AppOpsManager.OPSTR_RECORD_AUDIO,
                Process.myUid(), packageName);
        assertWithMessage("OP_RECORD_AUDIO should be MODE_ALLOWED during FGS")
                .that(opResult).isEqualTo(AppOpsManager.MODE_ALLOWED);
    }

    @Test
    public void testFgsTypeMicrophone_spoofed_audioRecordStillWorksAfterRemap()
            throws Exception {
        var fgsError = startFgsAndWait();
        assertWithMessage("FGS should start successfully").that(fgsError).isNull();

        var minBufferSize = AudioRecord.getMinBufferSize(
                48_000, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT);
        assertWithMessage("AudioRecord min buffer size should be valid")
                .that(minBufferSize).isGreaterThan(0);

        var record = new AudioRecord.Builder()
                .setAudioSource(MediaRecorder.AudioSource.MIC)
                .setAudioFormat(new AudioFormat.Builder()
                        .setSampleRate(48_000)
                        .setChannelMask(AudioFormat.CHANNEL_IN_MONO)
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .build())
                .setBufferSizeInBytes(Math.max(minBufferSize, 4_096))
                .build();

        try {
            assertWithMessage("AudioRecord should initialize after the FGS remap")
                    .that(record.getState()).isEqualTo(AudioRecord.STATE_INITIALIZED);

            record.startRecording();
            assertWithMessage("AudioRecord should enter RECORDSTATE_RECORDING")
                    .that(record.getRecordingState())
                    .isEqualTo(AudioRecord.RECORDSTATE_RECORDING);

            var buffer = new byte[2_048];
            var bytesRead = record.read(buffer, 0, buffer.length);
            assertWithMessage("AudioRecord should return captured frames")
                    .that(bytesRead).isGreaterThan(0);
            assertWithMessage("Spoofed AudioRecord should still return non-silent audio")
                    .that(hasNonSilentBytes(buffer, bytesRead)).isTrue();

            record.stop();
        } finally {
            record.release();
        }
    }

    @Test
    public void testSpoofingToggle_enableDisableEnable_permissionsTrackState() throws Exception {
        assertThat(context.checkSelfPermission(Manifest.permission.RECORD_AUDIO))
                .isEqualTo(PackageManager.PERMISSION_GRANTED);

        disableMicSpoofing();
        assertThat(context.checkSelfPermission(Manifest.permission.RECORD_AUDIO))
                .isEqualTo(PackageManager.PERMISSION_DENIED);

        enableMicSpoofing();
        assertThat(context.checkSelfPermission(Manifest.permission.RECORD_AUDIO))
                .isEqualTo(PackageManager.PERMISSION_GRANTED);
    }

    @Test
    public void testSpoofingToggle_appOpsTrackState() throws Exception {
        AppOpsManager appOpsManager = context.getSystemService(AppOpsManager.class);

        assertThat(appOpsManager.checkOpNoThrow(
                AppOpsManager.OPSTR_RECORD_AUDIO, Process.myUid(), packageName))
                .isEqualTo(AppOpsManager.MODE_ALLOWED);

        disableMicSpoofing();
        assertThat(appOpsManager.checkOpNoThrow(
                AppOpsManager.OPSTR_RECORD_AUDIO, Process.myUid(), packageName))
                .isNotEqualTo(AppOpsManager.MODE_ALLOWED);

        enableMicSpoofing();
        assertThat(appOpsManager.checkOpNoThrow(
                AppOpsManager.OPSTR_RECORD_AUDIO, Process.myUid(), packageName))
                .isEqualTo(AppOpsManager.MODE_ALLOWED);
    }

    @Test
    public void testCheckSelfPermission_onlyRecordAudioSpoofed() {
        String[] unaffectedPermissions = {
                Manifest.permission.CAMERA,
                Manifest.permission.ACCESS_FINE_LOCATION,
                Manifest.permission.ACCESS_COARSE_LOCATION,
                Manifest.permission.READ_CONTACTS,
                Manifest.permission.READ_PHONE_STATE,
                Manifest.permission.BODY_SENSORS,
        };

        for (var perm : unaffectedPermissions) {
            var result = context.checkSelfPermission(perm);
            assertWithMessage("checkSelfPermission(" + perm + ") should not be spoofed")
                    .that(result).isEqualTo(PackageManager.PERMISSION_DENIED);
        }
    }

    @Test
    public void testAppOpsManager_onlyRecordAudioSpoofed() {
        var appOpsManager = context.getSystemService(AppOpsManager.class);

        String[] unaffectedOps = {
                AppOpsManager.OPSTR_CAMERA,
                AppOpsManager.OPSTR_FINE_LOCATION,
                AppOpsManager.OPSTR_COARSE_LOCATION,
                AppOpsManager.OPSTR_READ_CONTACTS,
                AppOpsManager.OPSTR_BODY_SENSORS,
        };

        for (var op : unaffectedOps) {
            var mode = appOpsManager.checkOpNoThrow(op, Process.myUid(), packageName);
            assertWithMessage("checkOp(" + op + ") should not be spoofed by mic spoofing")
                    .that(mode).isNotEqualTo(AppOpsManager.MODE_ALLOWED);
        }
    }

    @Test
    public void testFgsTypeMicrophone_disableSpoofingWhileRunning_serviceStaysAlive()
            throws Exception {
        var error = startFgsAndWait();
        assertWithMessage("FGS should start successfully").that(error).isNull();

        disableMicSpoofing();
        Thread.sleep(500);

        var activityManager = context.getSystemService(ActivityManager.class);
        var services = activityManager.getRunningServices(100);
        var stillRunning = false;
        for (var info : services) {
            if (MicSpoofingFgsHelper.class.getName().equals(info.service.getClassName())) {
                stillRunning = true;
                break;
            }
        }
        assertWithMessage("FGS should still be running after disabling mic spoofing")
                .that(stillRunning).isTrue();

        var result = context.checkSelfPermission(Manifest.permission.RECORD_AUDIO);
        assertWithMessage("RECORD_AUDIO should be DENIED after disabling spoofing")
                .that(result).isEqualTo(PackageManager.PERMISSION_DENIED);
    }

    @Test
    public void testFgsTypeMicrophone_restartAfterToggle() throws Exception {
        var error = startFgsAndWait();
        assertWithMessage("First FGS start should succeed").that(error).isNull();

        context.stopService(new Intent(context, MicSpoofingFgsHelper.class));
        Thread.sleep(500);

        disableMicSpoofing();
        MicSpoofingFgsHelper.resetLatch();

        var intent = new Intent(context, MicSpoofingFgsHelper.class);
        intent.setAction(MicSpoofingFgsHelper.ACTION_START_MIC_FGS);

        context.startForegroundService(intent);

        var completed = MicSpoofingFgsHelper.latch.await(10, TimeUnit.SECONDS);
        assertWithMessage("FGS helper should respond within timeout").that(completed).isTrue();
        error = MicSpoofingFgsHelper.startForegroundError;
        assertWithMessage("FGS should fail without mic spoofing").that(error).isNotNull();
        assertThat(error).isInstanceOf(SecurityException.class);

        context.stopService(new Intent(context, MicSpoofingFgsHelper.class));
        Thread.sleep(500);
        enableMicSpoofing();
        error = startFgsAndWait();
        assertWithMessage("FGS should succeed after re-enabling mic spoofing")
                .that(error).isNull();
    }

    private void markRecordAudioAsUserSetDenied() throws IOException {
        runShellCommand("pm clear-permission-flags --user " + userId + " " + packageName + " "
                + Manifest.permission.RECORD_AUDIO + " user-fixed");
        runShellCommand("pm set-permission-flags --user " + userId + " " + packageName + " "
                + Manifest.permission.RECORD_AUDIO + " user-set");
    }

    @Test
    public void testShouldShowRequestPermissionRationale_recordAudio_spoofedReturnsFalse()
            throws Exception {
        disableMicSpoofing();
        assertWithMessage("RECORD_AUDIO should be denied in test setup")
                .that(context.checkSelfPermission(Manifest.permission.RECORD_AUDIO))
                .isEqualTo(PackageManager.PERMISSION_DENIED);

        markRecordAudioAsUserSetDenied();

        var rationaleWithoutSpoofing = context
                .getPackageManager()
                .shouldShowRequestPermissionRationale(Manifest.permission.RECORD_AUDIO);
        assertWithMessage("Without spoofing, denied USER_SET mic permission should show rationale")
                .that(rationaleWithoutSpoofing).isTrue();

        enableMicSpoofing();
        var rationaleWithSpoofing = context
                .getPackageManager()
                .shouldShowRequestPermissionRationale(Manifest.permission.RECORD_AUDIO);
        assertWithMessage("With mic spoofing enabled, rationale should be hidden")
                .that(rationaleWithSpoofing).isFalse();
    }
}
