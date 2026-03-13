package com.android.mic_spoofing_test;

import static com.google.common.truth.Truth.assertThat;

import android.Manifest;
import android.app.AppOpsManager;
import android.content.pm.GosPackageState;
import android.content.pm.GosPackageStateFlag;
import android.content.pm.spoofing.MicSpoofing;
import android.ext.DerivedPackageFlag;

import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.filters.SmallTest;

import org.junit.After;
import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;

@SmallTest
@RunWith(AndroidJUnit4.class)
public class MicSpoofingStateTest {

    private static final long MIC_SPOOFING_FLAG = 1L << GosPackageStateFlag.MIC_SPOOFING_ENABLED;
    private static final int DFLAG_RECORD_AUDIO = DerivedPackageFlag.HAS_RECORD_AUDIO_DECLARATION;

    @Before
    public void setUp() {
        resetMicSpoofing();
    }

    @After
    public void tearDown() {
        resetMicSpoofing();
    }

    private void resetMicSpoofing() {
        if (MicSpoofing.isEnabled()) {
            MicSpoofing.onGosPackageStateChanged(GosPackageState.DEFAULT);
        }
    }

    private static GosPackageState createState(long flagStorage1, int derivedFlags) {
        var state = new GosPackageState(flagStorage1, 0L, null, null, null);
        state.derivedFlags = derivedFlags;
        return state;
    }

    private void enableMicSpoofing(int derivedFlags) {
        MicSpoofing.onGosPackageStateChanged(createState(MIC_SPOOFING_FLAG, derivedFlags));
    }

    private void enableMicSpoofingWithRecordAudio() {
        enableMicSpoofing(DFLAG_RECORD_AUDIO);
    }

    @Test
    public void testIsEnabled_defaultState_returnsFalse() {
        assertThat(MicSpoofing.isEnabled()).isFalse();
    }

    @Test
    public void testOnGosPackageStateChanged_enableWithFlag() {
        enableMicSpoofingWithRecordAudio();

        assertThat(MicSpoofing.isEnabled()).isTrue();
    }

    @Test
    public void testOnGosPackageStateChanged_disableByRemovingFlag() {
        enableMicSpoofingWithRecordAudio();
        assertThat(MicSpoofing.isEnabled()).isTrue();

        MicSpoofing.onGosPackageStateChanged(GosPackageState.DEFAULT);

        assertThat(MicSpoofing.isEnabled()).isFalse();
    }

    @Test
    public void testOnGosPackageStateChanged_enableCachesDerivedFlags() {
        enableMicSpoofingWithRecordAudio();

        assertThat(MicSpoofing.shouldSpoofSelfPermissionCheck(
                Manifest.permission.RECORD_AUDIO)).isTrue();
    }

    @Test
    public void testOnGosPackageStateChanged_alreadyEnabled_doesNotUpdateDerivedFlags() {
        enableMicSpoofingWithRecordAudio();
        assertThat(MicSpoofing.shouldSpoofSelfPermissionCheck(
                Manifest.permission.RECORD_AUDIO)).isTrue();

        MicSpoofing.onGosPackageStateChanged(createState(MIC_SPOOFING_FLAG, 0));

        assertThat(MicSpoofing.shouldSpoofSelfPermissionCheck(
                Manifest.permission.RECORD_AUDIO)).isTrue();
    }

    @Test
    public void testOnGosPackageStateChanged_alreadyDisabled_isNoOp() {
        assertThat(MicSpoofing.isEnabled()).isFalse();

        MicSpoofing.onGosPackageStateChanged(GosPackageState.DEFAULT);

        assertThat(MicSpoofing.isEnabled()).isFalse();
    }

    @Test
    public void testOnGosPackageStateChanged_reEnableAfterDisable_updatesDerivedFlags() {
        enableMicSpoofingWithRecordAudio();
        assertThat(MicSpoofing.shouldSpoofSelfPermissionCheck(
                Manifest.permission.RECORD_AUDIO)).isTrue();

        MicSpoofing.onGosPackageStateChanged(GosPackageState.DEFAULT);
        assertThat(MicSpoofing.isEnabled()).isFalse();

        enableMicSpoofing(DerivedPackageFlag.DFLAGS_SET);
        assertThat(MicSpoofing.isEnabled()).isTrue();

        assertThat(MicSpoofing.shouldSpoofSelfPermissionCheck(
                Manifest.permission.RECORD_AUDIO)).isFalse();
    }

    @Test
    public void testOnGosPackageStateChanged_enableWithOtherGosFlags() {
        long flags = MIC_SPOOFING_FLAG | (1L << GosPackageStateFlag.STORAGE_SCOPES_ENABLED);
        MicSpoofing.onGosPackageStateChanged(createState(flags, DFLAG_RECORD_AUDIO));

        assertThat(MicSpoofing.isEnabled()).isTrue();
        assertThat(MicSpoofing.shouldSpoofSelfPermissionCheck(
                Manifest.permission.RECORD_AUDIO)).isTrue();
    }

    @Test
    public void testOnGosPackageStateChanged_disableDoesNotClearDerivedFlags() {
        enableMicSpoofingWithRecordAudio();
        assertThat(MicSpoofing.shouldSpoofSelfPermissionCheck(
                Manifest.permission.RECORD_AUDIO)).isTrue();
        assertThat(MicSpoofing.shouldSpoofSelfAppOpCheck(
                AppOpsManager.OP_RECORD_AUDIO)).isTrue();

        MicSpoofing.onGosPackageStateChanged(GosPackageState.DEFAULT);

        assertThat(MicSpoofing.shouldSpoofSelfPermissionCheck(
                Manifest.permission.RECORD_AUDIO)).isFalse();
        assertThat(MicSpoofing.shouldSpoofSelfAppOpCheck(
                AppOpsManager.OP_RECORD_AUDIO)).isFalse();
    }

    @Test
    public void testSpoofSelfPermCheck_recordAudio_enabled_returnsTrue() {
        enableMicSpoofingWithRecordAudio();

        assertThat(MicSpoofing.shouldSpoofSelfPermissionCheck(
                Manifest.permission.RECORD_AUDIO)).isTrue();
    }

    @Test
    public void testSpoofSelfPermCheck_recordAudio_disabled_returnsFalse() {
        assertThat(MicSpoofing.shouldSpoofSelfPermissionCheck(
                Manifest.permission.RECORD_AUDIO)).isFalse();
    }

    @Test
    public void testSpoofSelfPermCheck_recordAudio_noDerivedFlag_returnsFalse() {
        enableMicSpoofing(DerivedPackageFlag.DFLAGS_SET);

        assertThat(MicSpoofing.shouldSpoofSelfPermissionCheck(
                Manifest.permission.RECORD_AUDIO)).isFalse();
    }

    @Test
    public void testSpoofSelfPermCheck_camera_enabled_returnsFalse() {
        enableMicSpoofingWithRecordAudio();

        assertThat(MicSpoofing.shouldSpoofSelfPermissionCheck(
                Manifest.permission.CAMERA)).isFalse();
    }

    @Test
    public void testSpoofSelfPermCheck_internet_enabled_returnsFalse() {
        enableMicSpoofingWithRecordAudio();

        assertThat(MicSpoofing.shouldSpoofSelfPermissionCheck(
                Manifest.permission.INTERNET)).isFalse();
    }

    @Test
    public void testSpoofSelfPermCheck_fineLocation_enabled_returnsFalse() {
        enableMicSpoofingWithRecordAudio();

        assertThat(MicSpoofing.shouldSpoofSelfPermissionCheck(
                Manifest.permission.ACCESS_FINE_LOCATION)).isFalse();
    }

    @Test
    public void testSpoofSelfPermCheck_readExternalStorage_enabled_returnsFalse() {
        enableMicSpoofingWithRecordAudio();

        assertThat(MicSpoofing.shouldSpoofSelfPermissionCheck(
                Manifest.permission.READ_EXTERNAL_STORAGE)).isFalse();
    }

    @Test
    public void testSpoofSelfPermCheck_multipleDerivedFlags_returnsTrue() {
        int derivedFlags = DFLAG_RECORD_AUDIO
                | DerivedPackageFlag.HAS_READ_CONTACTS_DECLARATION
                | DerivedPackageFlag.DFLAGS_SET;
        enableMicSpoofing(derivedFlags);

        assertThat(MicSpoofing.shouldSpoofSelfPermissionCheck(
                Manifest.permission.RECORD_AUDIO)).isTrue();
    }

    @Test
    public void testSpoofSelfAppOpCheck_recordAudio_enabled_returnsTrue() {
        enableMicSpoofingWithRecordAudio();

        assertThat(MicSpoofing.shouldSpoofSelfAppOpCheck(
                AppOpsManager.OP_RECORD_AUDIO)).isTrue();
    }

    @Test
    public void testSpoofSelfAppOpCheck_recordAudio_disabled_returnsFalse() {
        assertThat(MicSpoofing.shouldSpoofSelfAppOpCheck(
                AppOpsManager.OP_RECORD_AUDIO)).isFalse();
    }

    @Test
    public void testSpoofSelfAppOpCheck_recordAudio_noDerivedFlag_returnsFalse() {
        enableMicSpoofing(DerivedPackageFlag.DFLAGS_SET);

        assertThat(MicSpoofing.shouldSpoofSelfAppOpCheck(
                AppOpsManager.OP_RECORD_AUDIO)).isFalse();
    }

    @Test
    public void testSpoofSelfAppOpCheck_camera_enabled_returnsFalse() {
        enableMicSpoofingWithRecordAudio();

        assertThat(MicSpoofing.shouldSpoofSelfAppOpCheck(
                AppOpsManager.OP_CAMERA)).isFalse();
    }

    @Test
    public void testSpoofSelfAppOpCheck_fineLocation_enabled_returnsFalse() {
        enableMicSpoofingWithRecordAudio();

        assertThat(MicSpoofing.shouldSpoofSelfAppOpCheck(
                AppOpsManager.OP_FINE_LOCATION)).isFalse();
    }

    @Test
    public void testGetSpoofablePermissionDflag_recordAudio_returnsExpected() {
        assertThat(MicSpoofing.getSpoofablePermissionDflag(Manifest.permission.RECORD_AUDIO))
                .isEqualTo(DerivedPackageFlag.HAS_RECORD_AUDIO_DECLARATION);
    }

    @Test
    public void testGetSpoofablePermissionDflag_camera_returnsZero() {
        assertThat(MicSpoofing.getSpoofablePermissionDflag(Manifest.permission.CAMERA))
                .isEqualTo(0);
    }

    @Test
    public void testGetSpoofablePermissionDflag_internet_returnsZero() {
        assertThat(MicSpoofing.getSpoofablePermissionDflag(Manifest.permission.INTERNET))
                .isEqualTo(0);
    }

    @Test
    public void testGetSpoofablePermissionDflag_readContacts_returnsZero() {
        assertThat(MicSpoofing.getSpoofablePermissionDflag(Manifest.permission.READ_CONTACTS))
                .isEqualTo(0);
    }
}
