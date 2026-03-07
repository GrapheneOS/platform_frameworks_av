package com.android.mic_spoofing_test;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.IBinder;

import java.util.concurrent.CountDownLatch;

public class MicSpoofingSystemExemptedFgsHelper extends Service {

    static final String CHANNEL_ID = "mic_spoofing_test_channel";
    static final String ACTION_START_SYSTEM_EXEMPTED_FGS =
            "com.android.mic_spoofing_test.ACTION_START_SYSTEM_EXEMPTED_FGS";

    static volatile Throwable startForegroundError;

    static volatile CountDownLatch latch = new CountDownLatch(1);

    static void resetLatch() {
        startForegroundError = null;
        latch = new CountDownLatch(1);
    }

    @Override
    public void onCreate() {
        super.onCreate();

        var notificationManager = getSystemService(NotificationManager.class);
        if (notificationManager.getNotificationChannel(CHANNEL_ID) == null) {
            notificationManager.createNotificationChannel(new NotificationChannel(
                    CHANNEL_ID, "MicSpoofing Test", NotificationManager.IMPORTANCE_LOW));
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        try {
            var notification = new Notification.Builder(this, CHANNEL_ID)
                    .setSmallIcon(android.R.drawable.ic_media_play)
                    .setContentTitle("MicSpoofing systemExempted FGS Test")
                    .build();

            startForeground(startId, notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_SYSTEM_EXEMPTED);
            startForegroundError = null;
        } catch (Throwable t) {
            startForegroundError = t;
            stopSelf();
        } finally {
            latch.countDown();
        }
        return START_NOT_STICKY;
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
