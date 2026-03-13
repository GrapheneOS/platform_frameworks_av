#include <gtest/gtest.h>
#include <aaudio/AAudio.h>
#include <mic_spoofing.h>
#include <mic_spoofing_decoder.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <mutex>
#include <unistd.h>
#include <vector>

namespace {

constexpr int64_t kNanosPerSecond = 1000000000LL;
constexpr int64_t kNanosPerMillisecond = 1000000LL;
constexpr int64_t kReadTimeoutNanos = 2 * kNanosPerSecond;

constexpr int32_t kDefaultSampleRate = 48000;
constexpr int32_t kReadFrameCount = 4800;

void ensureDecoderFactoryRegistered() {
    static const bool registered = []() {
        mic_spoofing_set_decoder_factory(&mic_spoofing_decoder_start);
        return true;
    }();
}

struct CallbackData {
    std::mutex mutex;
    std::condition_variable cv;
    bool finished = false;
    std::atomic<int32_t> callbackCount{0};
    bool hasNonSilent = false;

    void signalFinished() {
        {
            std::lock_guard<std::mutex> lock(mutex);
            finished = true;
        }
        cv.notify_one();
    }

    void waitForFinished(int timeoutMs = 5000) {
        std::unique_lock<std::mutex> lock(mutex);
        cv.wait_for(lock, std::chrono::milliseconds(timeoutMs),
                [this] { return finished; });
    }
};

aaudio_data_callback_result_t nonSilentCheckCallback(
        AAudioStream *stream,
        void *userData,
        void *audioData,
        int32_t numFrames
) {
    auto *data = static_cast<CallbackData *>(userData);
    int32_t count = data->callbackCount.fetch_add(1);

    if (!data->hasNonSilent) {
        int32_t channels = AAudioStream_getChannelCount(stream);
        aaudio_format_t format = AAudioStream_getFormat(stream);
        int32_t numSamples = numFrames * channels;

        if (format == AAUDIO_FORMAT_PCM_I16) {
            const auto *samples = static_cast<const int16_t *>(audioData);
            for (int32_t i = 0; i < numSamples; i++) {
                if (samples[i] != 0) { data->hasNonSilent = true; break; }
            }
        } else if (format == AAUDIO_FORMAT_PCM_FLOAT) {
            const auto *samples = static_cast<const float *>(audioData);
            for (int32_t i = 0; i < numSamples; i++) {
                if (samples[i] != 0.0f) { data->hasNonSilent = true; break; }
            }
        } else if (format == AAUDIO_FORMAT_PCM_I32) {
            const auto *samples = static_cast<const int32_t *>(audioData);
            for (int32_t i = 0; i < numSamples; i++) {
                if (samples[i] != 0) { data->hasNonSilent = true; break; }
            }
        } else if (format == AAUDIO_FORMAT_PCM_I24_PACKED) {
            const auto *bytes = static_cast<const uint8_t *>(audioData);
            for (int32_t i = 0; i < numSamples * 3; i++) {
                if (bytes[i] != 0) { data->hasNonSilent = true; break; }
            }
        }
    }

    if (count >= 10 || data->hasNonSilent) {
        data->signalFinished();
        return AAUDIO_CALLBACK_RESULT_STOP;
    }
    return AAUDIO_CALLBACK_RESULT_CONTINUE;
}

aaudio_data_callback_result_t stopImmediatelyCallback(
        AAudioStream * ,
        void *userData,
        void *,
        int32_t
) {
    auto *data = static_cast<CallbackData *>(userData);
    data->callbackCount.fetch_add(1);
    data->signalFinished();
    return AAUDIO_CALLBACK_RESULT_STOP;
}

static aaudio_result_t openSpoofedInputStream(
        AAudioStream **outStream,
        int32_t sampleRate = kDefaultSampleRate,
        int32_t channelCount = 1,
        aaudio_format_t format = AAUDIO_FORMAT_PCM_I16,
        aaudio_performance_mode_t perfMode = AAUDIO_PERFORMANCE_MODE_NONE,
        aaudio_sharing_mode_t sharingMode = AAUDIO_SHARING_MODE_SHARED,
        AAudioStream_dataCallback callback = nullptr,
        void *userData = nullptr,
        aaudio_input_preset_t preset = AAUDIO_INPUT_PRESET_GENERIC
) {
    AAudioStreamBuilder *builder = nullptr;
    aaudio_result_t result = AAudio_createStreamBuilder(&builder);
    if (result != AAUDIO_OK) return result;

    AAudioStreamBuilder_setDirection(builder, AAUDIO_DIRECTION_INPUT);
    AAudioStreamBuilder_setSampleRate(builder, sampleRate);
    AAudioStreamBuilder_setChannelCount(builder, channelCount);
    AAudioStreamBuilder_setFormat(builder, format);
    AAudioStreamBuilder_setPerformanceMode(builder, perfMode);
    AAudioStreamBuilder_setSharingMode(builder, sharingMode);
    AAudioStreamBuilder_setInputPreset(builder, preset);

    if (callback) {
        AAudioStreamBuilder_setDataCallback(builder, callback, userData);
    }

    result = AAudioStreamBuilder_openStream(builder, outStream);
    AAudioStreamBuilder_delete(builder);
    return result;
}

static bool readAndCheckNonSilent(AAudioStream *stream, int32_t framesToRead = kReadFrameCount) {
    aaudio_format_t format = AAudioStream_getFormat(stream);
    int32_t channels = AAudioStream_getChannelCount(stream);

    if (format == AAUDIO_FORMAT_PCM_I16) {
        std::vector<int16_t> buf(framesToRead * channels, 0);
        int32_t framesRead = AAudioStream_read(
                stream, buf.data(), framesToRead, kReadTimeoutNanos);
        if (framesRead <= 0) return false;
        return std::any_of(buf.begin(), buf.begin() + framesRead * channels,
                [](int16_t s) { return s != 0; });
    }
    if (format == AAUDIO_FORMAT_PCM_FLOAT) {
        std::vector<float> buf(framesToRead * channels, 0.0f);
        int32_t framesRead = AAudioStream_read(
                stream, buf.data(), framesToRead, kReadTimeoutNanos);
        if (framesRead <= 0) return false;
        return std::any_of(buf.begin(), buf.begin() + framesRead * channels,
                [](float s) { return s != 0.0f; });
    }
    if (format == AAUDIO_FORMAT_PCM_I32) {
        std::vector<int32_t> buf(framesToRead * channels, 0);
        int32_t framesRead = AAudioStream_read(
                stream, buf.data(), framesToRead, kReadTimeoutNanos);
        if (framesRead <= 0) return false;
        return std::any_of(buf.begin(), buf.begin() + framesRead * channels,
                [](int32_t s) { return s != 0; });
    }
    if (format == AAUDIO_FORMAT_PCM_I24_PACKED) {
        std::vector<uint8_t> buf(framesToRead * channels * 3, 0);
        int32_t framesRead = AAudioStream_read(
                stream, buf.data(), framesToRead, kReadTimeoutNanos);
        if (framesRead <= 0) return false;
        return std::any_of(buf.begin(), buf.begin() + framesRead * channels * 3,
                [](uint8_t s) { return s != 0; });
    }
    return false;
}

class AAudioSpoofedTest : public ::testing::Test {
protected:
    void SetUp() override {
        ensureDecoderFactoryRegistered();
        ASSERT_EQ(0, system("pm edit-gos-package-state com.android.shell 0"
                " add-flag MIC_SPOOFING_ENABLED"))
                << "Failed to enable mic spoofing for com.android.shell";
        ASSERT_TRUE(mic_spoofing_is_enabled_for_uid(getuid()))
                << "mic_spoofing_is_enabled_for_uid(" << getuid()
                << ") returned false after setting flag";
    }

    void TearDown() override {
        system("pm edit-gos-package-state com.android.shell 0"
                " clear-flag MIC_SPOOFING_ENABLED");
    }
};

TEST_F(AAudioSpoofedTest, OpenInputStream_I16_ReadsNonSilentData) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream,
            kDefaultSampleRate, 1, AAUDIO_FORMAT_PCM_I16));

    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    EXPECT_TRUE(readAndCheckNonSilent(stream))
            << "I16 input should contain non-silent spoofed data";
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, OpenInputStream_Float_ReadsNonSilentData) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream,
            kDefaultSampleRate, 1, AAUDIO_FORMAT_PCM_FLOAT));

    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));

    // Read and verify non-silent + valid float range
    int32_t channels = AAudioStream_getChannelCount(stream);
    std::vector<float> buf(kReadFrameCount * channels, 0.0f);
    int32_t framesRead = AAudioStream_read(
            stream, buf.data(), kReadFrameCount, kReadTimeoutNanos);
    ASSERT_GT(framesRead, 0) << "Should read at least some frames";

    bool hasNonSilent = false;
    for (int32_t i = 0; i < framesRead * channels; i++) {
        EXPECT_GE(buf[i], -1.0f) << "Float sample out of range at index " << i;
        EXPECT_LE(buf[i], 1.0f) << "Float sample out of range at index " << i;
        if (buf[i] != 0.0f) hasNonSilent = true;
    }
    EXPECT_TRUE(hasNonSilent) << "Float input should contain non-silent spoofed data";

    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));
    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, OpenInputStream_I24Packed_ReadsNonSilentData) {
    AAudioStream *stream = nullptr;
    aaudio_result_t result = openSpoofedInputStream(&stream,
            kDefaultSampleRate, 1, AAUDIO_FORMAT_PCM_I24_PACKED);
    if (result != AAUDIO_OK) {
        GTEST_SKIP() << "I24_PACKED not supported on this device (result=" << result << ")";
    }

    EXPECT_EQ(AAUDIO_FORMAT_PCM_I24_PACKED, AAudioStream_getFormat(stream));
    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    EXPECT_TRUE(readAndCheckNonSilent(stream))
            << "I24_PACKED input should contain non-silent spoofed data";
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, OpenInputStream_I32_ReadsNonSilentData) {
    AAudioStream *stream = nullptr;
    aaudio_result_t result = openSpoofedInputStream(&stream,
            kDefaultSampleRate, 1, AAUDIO_FORMAT_PCM_I32);
    if (result != AAUDIO_OK) {
        GTEST_SKIP() << "I32 not supported on this device (result=" << result << ")";
    }

    EXPECT_EQ(AAUDIO_FORMAT_PCM_I32, AAudioStream_getFormat(stream));
    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    EXPECT_TRUE(readAndCheckNonSilent(stream))
            << "I32 input should contain non-silent spoofed data";
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, OpenInputStream_FormatUnspecified_NegotiatesSuccessfully) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream,
            kDefaultSampleRate, 1, AAUDIO_FORMAT_UNSPECIFIED));

    aaudio_format_t actual = AAudioStream_getFormat(stream);
    EXPECT_NE(AAUDIO_FORMAT_UNSPECIFIED, actual)
            << "Format should be negotiated to a concrete value";

    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    EXPECT_TRUE(readAndCheckNonSilent(stream))
            << "Stream with negotiated format should produce spoofed data";
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, Callback_ReceivesNonSilentFrames) {
    CallbackData cbData;
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream,
            kDefaultSampleRate, 1, AAUDIO_FORMAT_PCM_I16,
            AAUDIO_PERFORMANCE_MODE_NONE, AAUDIO_SHARING_MODE_SHARED,
            nonSilentCheckCallback, &cbData));

    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    cbData.waitForFinished();
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    EXPECT_TRUE(cbData.hasNonSilent) << "Callback should have received non-silent spoofed data";
    EXPECT_GT(cbData.callbackCount.load(), 0) << "Callback should have been invoked at least once";

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, Callback_StopResult_StopsAfterFirstCall) {
    CallbackData cbData;
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream,
            kDefaultSampleRate, 1, AAUDIO_FORMAT_PCM_I16,
            AAUDIO_PERFORMANCE_MODE_NONE, AAUDIO_SHARING_MODE_SHARED,
            stopImmediatelyCallback, &cbData));

    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    cbData.waitForFinished();
    sleep(1); // Give time for any spurious extra callbacks
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    EXPECT_EQ(1, cbData.callbackCount.load())
            << "CALLBACK_RESULT_STOP should halt callbacks after first invocation";

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, Callback_Float_ReceivesNonSilentData) {
    CallbackData cbData;
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream,
            kDefaultSampleRate, 1, AAUDIO_FORMAT_PCM_FLOAT,
            AAUDIO_PERFORMANCE_MODE_NONE, AAUDIO_SHARING_MODE_SHARED,
            nonSilentCheckCallback, &cbData));

    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    cbData.waitForFinished();
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    EXPECT_TRUE(cbData.hasNonSilent)
            << "Float callback should have received non-silent spoofed data";

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, LowLatency_OpensAndReadsSuccessfully) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream,
            kDefaultSampleRate, 1, AAUDIO_FORMAT_PCM_I16,
            AAUDIO_PERFORMANCE_MODE_LOW_LATENCY));

    // With mic spoofing, MMAP is disabled and the stream falls back to legacy
    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    EXPECT_TRUE(readAndCheckNonSilent(stream))
            << "LOW_LATENCY stream should produce spoofed data via legacy fallback";
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, LowLatency_Callback_Succeeds) {
    CallbackData cbData;
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream,
            kDefaultSampleRate, 1, AAUDIO_FORMAT_PCM_I16,
            AAUDIO_PERFORMANCE_MODE_LOW_LATENCY, AAUDIO_SHARING_MODE_SHARED,
            nonSilentCheckCallback, &cbData));

    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    cbData.waitForFinished();
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    EXPECT_TRUE(cbData.hasNonSilent) << "LOW_LATENCY callback should receive spoofed data";

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, PowerSaving_OpensAndReadsSuccessfully) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream,
            kDefaultSampleRate, 1, AAUDIO_FORMAT_PCM_I16,
            AAUDIO_PERFORMANCE_MODE_POWER_SAVING));

    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    EXPECT_TRUE(readAndCheckNonSilent(stream)) << "POWER_SAVING stream should produce spoofed data";
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, LowLatency_ExclusiveMode_OpensSuccessfully) {
    AAudioStream *stream = nullptr;
    // Exclusive + LOW_LATENCY is the most MMAP-likely configuration
    // With mic spoofing, it should fall back to legacy/shared mode
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream,
            kDefaultSampleRate, 1, AAUDIO_FORMAT_PCM_I16,
            AAUDIO_PERFORMANCE_MODE_LOW_LATENCY, AAUDIO_SHARING_MODE_EXCLUSIVE));

    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    EXPECT_TRUE(readAndCheckNonSilent(stream))
            << "Exclusive LOW_LATENCY stream should still produce spoofed data";
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, SampleRate_8000_ReadsNonSilentData) {
    constexpr int32_t rate = 8000;
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream, rate));
    EXPECT_EQ(rate, AAudioStream_getSampleRate(stream));

    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    EXPECT_TRUE(readAndCheckNonSilent(stream, rate / 10))
            << "8 kHz resampled stream should produce non-silent data";
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, SampleRate_16000_ReadsNonSilentData) {
    constexpr int32_t rate = 16000;
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream, rate));
    EXPECT_EQ(rate, AAudioStream_getSampleRate(stream));

    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    EXPECT_TRUE(readAndCheckNonSilent(stream, rate / 10))
            << "16 kHz resampled stream should produce non-silent data";
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, SampleRate_44100_ReadsNonSilentData) {
    constexpr int32_t rate = 44100;
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream, rate));
    EXPECT_EQ(rate, AAudioStream_getSampleRate(stream));

    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    EXPECT_TRUE(readAndCheckNonSilent(stream, rate / 10))
            << "44.1 kHz resampled stream should produce non-silent data";
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, SampleRate_48000_ReadsNonSilentData) {
    constexpr int32_t rate = 48000;
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream, rate));
    EXPECT_EQ(rate, AAudioStream_getSampleRate(stream));

    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    EXPECT_TRUE(readAndCheckNonSilent(stream))
            << "48 kHz stream should produce non-silent data";
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, Stereo_ReadsNonSilentData) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream,
            kDefaultSampleRate, 2, AAUDIO_FORMAT_PCM_I16));
    EXPECT_EQ(2, AAudioStream_getChannelCount(stream));

    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    EXPECT_TRUE(readAndCheckNonSilent(stream))
            << "Stereo stream should produce non-silent spoofed data";
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, Stereo_BothChannelsHaveEqualMagnitude) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream,
            kDefaultSampleRate, 2, AAUDIO_FORMAT_PCM_I16));
    ASSERT_EQ(2, AAudioStream_getChannelCount(stream));
    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));

    std::vector<int16_t> buf(kReadFrameCount * 2, 0);
    int32_t framesRead = AAudioStream_read(
            stream, buf.data(), kReadFrameCount, kReadTimeoutNanos);
    ASSERT_GT(framesRead, 0);

    // The mic_spoofing library duplicates mono to both channels identically,
    // but the AudioFlinger legacy path may apply its own channel processing
    // (e.g. sign inversion). Verify both channels are non-silent and carry
    // the same magnitude of audio data
    bool lNonSilent = false;
    bool rNonSilent = false;
    for (int32_t i = 0; i < framesRead; i++) {
        int16_t l = buf[i * 2];
        int16_t r = buf[i * 2 + 1];
        EXPECT_EQ(std::abs(l), std::abs(r))
                << "L and R should have equal magnitude at frame " << i;
        if (l != 0) lNonSilent = true;
        if (r != 0) rNonSilent = true;
    }
    EXPECT_TRUE(lNonSilent) << "Left channel should be non-silent";
    EXPECT_TRUE(rNonSilent) << "Right channel should be non-silent";

    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));
    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, StartStop_StateTransitionsCorrectly) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream));

    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    aaudio_stream_state_t state = AAUDIO_STREAM_STATE_UNKNOWN;
    EXPECT_EQ(AAUDIO_OK, AAudioStream_waitForStateChange(
            stream, AAUDIO_STREAM_STATE_STARTING, &state, kNanosPerSecond));
    EXPECT_EQ(AAUDIO_STREAM_STATE_STARTED, state);

    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));
    EXPECT_EQ(AAUDIO_OK, AAudioStream_waitForStateChange(
            stream, AAUDIO_STREAM_STATE_STOPPING, &state, kNanosPerSecond));
    EXPECT_EQ(AAUDIO_STREAM_STATE_STOPPED, state);

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, StartStopRestart_ReadsData) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream));

    // First cycle.
    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    EXPECT_TRUE(readAndCheckNonSilent(stream));
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    // Wait for fully stopped.
    aaudio_stream_state_t state;
    EXPECT_EQ(AAUDIO_OK, AAudioStream_waitForStateChange(
            stream, AAUDIO_STREAM_STATE_STOPPING, &state, kNanosPerSecond));

    // Second cycle: restarting should still work.
    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    EXPECT_TRUE(readAndCheckNonSilent(stream))
            << "Restarted stream should still produce spoofed data";
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, CloseWhileRunning_Succeeds) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream));
    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));

    // Verify the stream is actually producing data
    EXPECT_TRUE(readAndCheckNonSilent(stream));

    // Close without stopping first. AAudio must handle this gracefully
    EXPECT_EQ(AAUDIO_OK, AAudioStream_close(stream));
}

TEST_F(AAudioSpoofedTest, ReleaseThenClose_Succeeds) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream));
    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    EXPECT_TRUE(readAndCheckNonSilent(stream));
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    EXPECT_EQ(AAUDIO_OK, AAudioStream_release(stream));
    EXPECT_EQ(AAUDIO_STREAM_STATE_CLOSING, AAudioStream_getState(stream));

    // Double release should be safe
    EXPECT_EQ(AAUDIO_OK, AAudioStream_release(stream));

    // Close after release.
    EXPECT_EQ(AAUDIO_OK, AAudioStream_close(stream));
}

TEST_F(AAudioSpoofedTest, NegotiatesRequestedParams) {
    constexpr int32_t requestedRate = 44100;
    constexpr int32_t requestedChannels = 2;
    constexpr aaudio_format_t requestedFormat = AAUDIO_FORMAT_PCM_I16;

    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream,
            requestedRate, requestedChannels, requestedFormat));

    EXPECT_EQ(requestedRate, AAudioStream_getSampleRate(stream));
    EXPECT_EQ(requestedChannels, AAudioStream_getChannelCount(stream));
    EXPECT_EQ(requestedFormat, AAudioStream_getFormat(stream));
    EXPECT_EQ(AAUDIO_DIRECTION_INPUT, AAudioStream_getDirection(stream));

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, StreamProperties_ReasonableValues) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream));

    int32_t framesPerBurst = AAudioStream_getFramesPerBurst(stream);
    int32_t bufferCapacity = AAudioStream_getBufferCapacityInFrames(stream);

    EXPECT_GT(framesPerBurst, 0) << "Frames per burst should be positive";
    EXPECT_GT(bufferCapacity, 0) << "Buffer capacity should be positive";
    EXPECT_GE(bufferCapacity, framesPerBurst) << "Buffer capacity should be at least one burst";

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, FramesRead_AdvancesWhileRecording) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream));
    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));

    // Read some data to advance position.
    readAndCheckNonSilent(stream);
    int64_t framesRead1 = AAudioStream_getFramesRead(stream);

    // Read more data.
    readAndCheckNonSilent(stream);
    int64_t framesRead2 = AAudioStream_getFramesRead(stream);

    EXPECT_GT(framesRead2, framesRead1)
            << "getFramesRead() should advance with successive reads";

    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));
    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, TwoConcurrentStreams_BothReadNonSilent) {
    AAudioStream *stream1 = nullptr;
    AAudioStream *stream2 = nullptr;

    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream1,
            48000, 1, AAUDIO_FORMAT_PCM_I16));
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream2,
            44100, 1, AAUDIO_FORMAT_PCM_FLOAT));

    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream1));
    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream2));

    EXPECT_TRUE(readAndCheckNonSilent(stream1))
            << "First concurrent stream should read non-silent data";
    EXPECT_TRUE(readAndCheckNonSilent(stream2))
            << "Second concurrent stream should read non-silent data";

    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream1));
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream2));
    AAudioStream_close(stream1);
    AAudioStream_close(stream2);
}

TEST_F(AAudioSpoofedTest, ConsecutiveReads_ProduceDifferentData) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream,
            kDefaultSampleRate, 1, AAUDIO_FORMAT_PCM_I16));
    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));

    std::vector<int16_t> buf1(kReadFrameCount, 0);
    std::vector<int16_t> buf2(kReadFrameCount, 0);

    int32_t read1 = AAudioStream_read(stream, buf1.data(), kReadFrameCount, kReadTimeoutNanos);
    int32_t read2 = AAudioStream_read(stream, buf2.data(), kReadFrameCount, kReadTimeoutNanos);

    ASSERT_GT(read1, 0);
    ASSERT_GT(read2, 0);

    // Two consecutive reads should produce different data because the
    // spoofed audio source position advances
    int32_t compareFrames = std::min(read1, read2);
    bool identical = std::equal(buf1.begin(), buf1.begin() + compareFrames,
            buf2.begin(), buf2.begin() + compareFrames);
    EXPECT_FALSE(identical) << "Consecutive reads should return different data";

    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));
    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, ReadBeforeStart_ReturnsNoData) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream));

    std::vector<int16_t> buf(kReadFrameCount, 0);
    int32_t framesRead = AAudioStream_read(
            stream, buf.data(), kReadFrameCount, 100 * kNanosPerMillisecond);
    EXPECT_LE(framesRead, 0)
            << "Read before start should return 0 frames or an error";

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, ReadAfterStop_DoesNotBlock) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream));
    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));

    // Confirm reads work while started.
    EXPECT_TRUE(readAndCheckNonSilent(stream));

    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));
    aaudio_stream_state_t state;
    EXPECT_EQ(AAUDIO_OK, AAudioStream_waitForStateChange(
            stream, AAUDIO_STREAM_STATE_STOPPING, &state, kNanosPerSecond));

    // After stop, buffered data may still be readable (this is normal AAudio
    // behavior). Drain any remaining data, then verify that a subsequent read
    // returns 0 or negative (no new data is produced)
    std::vector<int16_t> buf(kReadFrameCount, 0);

    // Drain buffered data.
    while (AAudioStream_read(stream, buf.data(), kReadFrameCount, 0) > 0) {}

    // No more buffered data; a timed read should return 0 or an error
    int32_t framesRead = AAudioStream_read(
            stream, buf.data(), kReadFrameCount, 100 * kNanosPerMillisecond);
    EXPECT_LE(framesRead, 0) << "Read after stop and drain should return 0 or an error";

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, SingleFrameRead_Succeeds) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream));
    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));

    int16_t sample = 0;
    int32_t framesRead = AAudioStream_read(stream, &sample, 1, kReadTimeoutNanos);
    EXPECT_EQ(1, framesRead) << "Should be able to read exactly 1 frame";

    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));
    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, NonBlockingRead_ReturnsImmediately) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream));
    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));

    // Non-blocking read (timeout = 0) should return immediately
    // May return 0 (no data buffered yet) or some frames
    std::vector<int16_t> buf(kReadFrameCount, 0);
    int32_t framesRead = AAudioStream_read(
            stream, buf.data(), kReadFrameCount, 0);
    EXPECT_GE(framesRead, 0) << "Non-blocking read should not return an error";

    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));
    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, InputPreset_VoiceRecognition_Succeeds) {
    AAudioStream *stream = nullptr;
    ASSERT_EQ(AAUDIO_OK, openSpoofedInputStream(&stream,
            kDefaultSampleRate, 1, AAUDIO_FORMAT_PCM_I16,
            AAUDIO_PERFORMANCE_MODE_NONE, AAUDIO_SHARING_MODE_SHARED,
            nullptr, nullptr, AAUDIO_INPUT_PRESET_VOICE_RECOGNITION));

    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));
    EXPECT_TRUE(readAndCheckNonSilent(stream))
            << "VOICE_RECOGNITION preset should produce spoofed data";
    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));

    AAudioStream_close(stream);
}

TEST_F(AAudioSpoofedTest, InputPreset_VoiceCommunication_Rejected) {
    AAudioStream *stream = nullptr;
    // VOICE_COMMUNICATION maps to AUDIO_SOURCE_VOICE_COMMUNICATION, which is outside the
    // mic spoofing allowlist
    EXPECT_EQ(AAUDIO_ERROR_INTERNAL, openSpoofedInputStream(&stream,
            kDefaultSampleRate, 1, AAUDIO_FORMAT_PCM_I16,
            AAUDIO_PERFORMANCE_MODE_NONE, AAUDIO_SHARING_MODE_SHARED,
            nullptr, nullptr, AAUDIO_INPUT_PRESET_VOICE_COMMUNICATION));
    EXPECT_EQ(nullptr, stream);
}

TEST_F(AAudioSpoofedTest, InputPreset_Camcorder_Rejected) {
    AAudioStream *stream = nullptr;
    // CAMCORDER sets privacySensitive=true in AudioStreamBuilder, but it still maps to
    // AUDIO_SOURCE_CAMCORDER, which is outside the mic spoofing allowlist
    EXPECT_EQ(AAUDIO_ERROR_INTERNAL, openSpoofedInputStream(&stream,
            kDefaultSampleRate, 1, AAUDIO_FORMAT_PCM_I16,
            AAUDIO_PERFORMANCE_MODE_NONE, AAUDIO_SHARING_MODE_SHARED,
            nullptr, nullptr, AAUDIO_INPUT_PRESET_CAMCORDER));
    EXPECT_EQ(nullptr, stream);
}

TEST_F(AAudioSpoofedTest, OutputStream_NotAffectedBySpoofing) {
    // Mic spoofing only gates on AAUDIO_DIRECTION_INPUT. Verify that output
    // streams are completely unaffected when mic spoofing is enabled
    AAudioStreamBuilder *builder = nullptr;
    ASSERT_EQ(AAUDIO_OK, AAudio_createStreamBuilder(&builder));

    AAudioStreamBuilder_setDirection(builder, AAUDIO_DIRECTION_OUTPUT);
    AAudioStreamBuilder_setSampleRate(builder, kDefaultSampleRate);
    AAudioStreamBuilder_setChannelCount(builder, 1);
    AAudioStreamBuilder_setFormat(builder, AAUDIO_FORMAT_PCM_I16);

    AAudioStream *stream = nullptr;
    aaudio_result_t result = AAudioStreamBuilder_openStream(builder, &stream);
    AAudioStreamBuilder_delete(builder);
    ASSERT_EQ(AAUDIO_OK, result);

    EXPECT_EQ(AAUDIO_DIRECTION_OUTPUT, AAudioStream_getDirection(stream));
    ASSERT_EQ(AAUDIO_OK, AAudioStream_requestStart(stream));

    // Write silence to verify the stream is functional
    std::vector<int16_t> silence(kDefaultSampleRate / 10, 0);
    int32_t framesWritten = AAudioStream_write(
            stream, silence.data(), silence.size(), kReadTimeoutNanos);
    EXPECT_GT(framesWritten, 0) << "Output stream should accept data";

    EXPECT_EQ(AAUDIO_OK, AAudioStream_requestStop(stream));
    AAudioStream_close(stream);
}

}
