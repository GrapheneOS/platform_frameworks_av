package com.android.mic_spoofing_test;

import android.content.pm.GosPackageState;
import android.content.pm.spoofing.MicSpoofing;
import android.ext.micspoofing.MicSpoofingApi;
import android.os.ParcelFileDescriptor;
import android.os.UserHandle;

import androidx.test.platform.app.InstrumentationRegistry;

import java.io.IOException;

abstract class BaseMicSpoofingTest {

    private static final String CUSTOM_SOURCE_FILENAME = "mic_spoofing_test.wav";

    protected String packageName;
    protected int userId;

    protected final void initPackageContext() {
        packageName = InstrumentationRegistry.getInstrumentation()
                .getTargetContext().getPackageName();
        userId = UserHandle.myUserId();
    }

    protected final void enableMicSpoofing() throws IOException {
        var customSourceConfigHex = buildCustomSourceConfigHex();
        runShellCommand("pm edit-gos-package-state " + packageName + " " + userId
                + " set-mic-spoofing-config " + customSourceConfigHex);
        runShellCommand("pm edit-gos-package-state " + packageName + " " + userId
                + " add-flag MIC_SPOOFING_ENABLED");
        var state = GosPackageState.get(packageName, userId);
        MicSpoofing.onGosPackageStateChanged(state);
    }

    protected final void disableMicSpoofing() throws IOException {
        runShellCommand("pm edit-gos-package-state " + packageName + " " + userId
                + " clear-flag MIC_SPOOFING_ENABLED");
        runShellCommand("pm edit-gos-package-state " + packageName + " " + userId
                + " set-mic-spoofing-config null");
        MicSpoofing.onGosPackageStateChanged(GosPackageState.DEFAULT);
    }

    protected static void runShellCommand(String command) throws IOException {
        var parcelFileDescriptor = InstrumentationRegistry.getInstrumentation()
                .getUiAutomation().executeShellCommand(command);
        try (var is = new ParcelFileDescriptor.AutoCloseInputStream(parcelFileDescriptor)) {
            is.readAllBytes();
        }
    }

    private String buildCustomSourceConfigHex() {
        var customSourcePath = "/storage/emulated/" + userId + "/" + CUSTOM_SOURCE_FILENAME;
        return bytesToHex(MicSpoofingApi.buildCustomPathConfig(customSourcePath));
    }

    private static String bytesToHex(byte[] bytes) {
        final var hex = "0123456789abcdef".toCharArray();
        var out = new char[bytes.length * 2];
        for (int i = 0; i < bytes.length; i++) {
            int value = bytes[i] & 0xFF;
            out[i * 2] = hex[value >>> 4];
            out[i * 2 + 1] = hex[value & 0x0F];
        }
        return new String(out);
    }
}
