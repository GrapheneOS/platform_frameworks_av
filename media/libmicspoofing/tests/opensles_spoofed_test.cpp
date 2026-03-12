#include <gtest/gtest.h>
#include <SLES/OpenSLES.h>
#include <SLES/OpenSLES_Android.h>
#include <mic_spoofing.h>
#include <mic_spoofing_decoder.h>

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <mutex>
#include <unistd.h>
#include <vector>

namespace {

constexpr int32_t kSampleRate = 48000;
constexpr int32_t kNumChannels = 1;
constexpr int32_t kBitsPerSample = 16;
constexpr int32_t kBytesPerFrame = kNumChannels * (kBitsPerSample / 8);

constexpr int32_t kBufferFrames = kSampleRate / 10;
constexpr int32_t kBufferSizeBytes = kBufferFrames * kBytesPerFrame;
constexpr int32_t kNumBuffers = 2;

void ensureDecoderFactoryRegistered() {
    static const bool registered = []() {
        mic_spoofing_set_decoder_factory(&mic_spoofing_decoder_start);
        return true;
    }();
    (void)registered;
}

struct RecordCallbackData {
    std::mutex mutex;
    std::condition_variable cv;
    std::atomic<int32_t> callbackCount{0};
    bool hasNonSilent = false;
    bool finished = false;

    // Double-buffering: the buffer queue alternates between these
    uint8_t buffers[kNumBuffers][kBufferSizeBytes];

    void signalFinished() {
        {
            std::lock_guard<std::mutex> lock(mutex);
            finished = true;
        }
        cv.notify_one();
    }

    bool waitForFinished(int timeoutMs = 5000) {
        std::unique_lock<std::mutex> lock(mutex);
        return cv.wait_for(lock, std::chrono::milliseconds(timeoutMs),
                [this] { return finished; });
    }
};

void bufferQueueCallback(SLAndroidSimpleBufferQueueItf bqItf, void *context) {
    auto *data = static_cast<RecordCallbackData *>(context);
    int32_t idx = data->callbackCount.fetch_add(1);

    // Check the most recently filled buffer for non-silent samples
    int32_t bufIdx = idx % kNumBuffers;
    if (!data->hasNonSilent) {
        const auto *samples = reinterpret_cast<const int16_t *>(
                data->buffers[bufIdx]);
        for (int32_t i = 0; i < kBufferFrames * kNumChannels; i++) {
            if (samples[i] != 0) {
                data->hasNonSilent = true;
                break;
            }
        }
    }

    // Stop after enough callbacks or once non-silence is detected
    if (idx >= 10 || data->hasNonSilent) {
        data->signalFinished();
        return;
    }

    // Re-enqueue the buffer for the next callback.
    int32_t nextBufIdx = (idx + 1) % kNumBuffers;
    (*bqItf)->Enqueue(bqItf, data->buffers[nextBufIdx], kBufferSizeBytes);
}

class OpenSLESSpoofedTest : public ::testing::Test {
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

    // Creates an OpenSL ES engine and returns the engine object
    // Caller must destroy via (*engineObj)->Destroy(engineObj)
    static SLresult createEngine(SLObjectItf *outEngineObj,
            SLEngineItf *outEngineItf) {
        SLresult result = slCreateEngine(outEngineObj, 0, nullptr, 0,
                nullptr, nullptr);
        if (result != SL_RESULT_SUCCESS) return result;

        result = (**outEngineObj)->Realize(*outEngineObj, SL_BOOLEAN_FALSE);
        if (result != SL_RESULT_SUCCESS) {
            (**outEngineObj)->Destroy(*outEngineObj);
            *outEngineObj = nullptr;
            return result;
        }

        result = (**outEngineObj)->GetInterface(*outEngineObj, SL_IID_ENGINE,
                outEngineItf);
        if (result != SL_RESULT_SUCCESS) {
            (**outEngineObj)->Destroy(*outEngineObj);
            *outEngineObj = nullptr;
        }
        return result;
    }

    // Creates and realizes an audio recorder with a buffer queue sink
    static SLresult createRecorder(SLEngineItf engineItf,
            SLObjectItf *outRecorderObj,
            SLRecordItf *outRecordItf,
            SLAndroidSimpleBufferQueueItf *outBqItf,
            int32_t sampleRate = kSampleRate,
            int32_t numChannels = kNumChannels) {
        // Data source: audio input device.
        SLDataLocator_IODevice ioDevice = {
            SL_DATALOCATOR_IODEVICE,
            SL_IODEVICE_AUDIOINPUT,
            SL_DEFAULTDEVICEID_AUDIOINPUT,
            nullptr
        };
        SLDataSource source = {&ioDevice, nullptr};

        // Data sink: Android simple buffer queue with PCM format
        SLDataLocator_AndroidSimpleBufferQueue bufferQueue = {
            SL_DATALOCATOR_ANDROIDSIMPLEBUFFERQUEUE,
            static_cast<SLuint32>(kNumBuffers)
        };
        SLAndroidDataFormat_PCM_EX pcmFormat = {
            SL_ANDROID_DATAFORMAT_PCM_EX,
            static_cast<SLuint32>(numChannels),
            static_cast<SLuint32>(sampleRate * 1000),
            SL_PCMSAMPLEFORMAT_FIXED_16,
            16, // containerSize
            numChannels == 1 ? SL_SPEAKER_FRONT_LEFT
                    : (SL_SPEAKER_FRONT_LEFT | SL_SPEAKER_FRONT_RIGHT),
            SL_BYTEORDER_LITTLEENDIAN,
            SL_ANDROID_PCM_REPRESENTATION_SIGNED_INT
        };
        SLDataSink sink = {&bufferQueue, &pcmFormat};

        // Request the buffer queue interface
        const SLInterfaceID ids[] = {SL_IID_ANDROIDSIMPLEBUFFERQUEUE};
        const SLboolean req[] = {SL_BOOLEAN_TRUE};

        SLresult result = (*engineItf)->CreateAudioRecorder(
                engineItf, outRecorderObj, &source, &sink, 1, ids, req);
        if (result != SL_RESULT_SUCCESS) return result;

        result = (**outRecorderObj)->Realize(*outRecorderObj, SL_BOOLEAN_FALSE);
        if (result != SL_RESULT_SUCCESS) {
            (**outRecorderObj)->Destroy(*outRecorderObj);
            *outRecorderObj = nullptr;
            return result;
        }

        result = (**outRecorderObj)->GetInterface(*outRecorderObj,
                SL_IID_RECORD, outRecordItf);
        if (result != SL_RESULT_SUCCESS) {
            (**outRecorderObj)->Destroy(*outRecorderObj);
            *outRecorderObj = nullptr;
            return result;
        }

        result = (**outRecorderObj)->GetInterface(*outRecorderObj,
                SL_IID_ANDROIDSIMPLEBUFFERQUEUE, outBqItf);
        if (result != SL_RESULT_SUCCESS) {
            (**outRecorderObj)->Destroy(*outRecorderObj);
            *outRecorderObj = nullptr;
        }
        return result;
    }
};

TEST_F(OpenSLESSpoofedTest, Record_I16Mono_ProducesNonSilentData) {
    SLObjectItf engineObj = nullptr;
    SLEngineItf engineItf = nullptr;
    ASSERT_EQ(SL_RESULT_SUCCESS, createEngine(&engineObj, &engineItf));

    SLObjectItf recorderObj = nullptr;
    SLRecordItf recordItf = nullptr;
    SLAndroidSimpleBufferQueueItf bqItf = nullptr;
    ASSERT_EQ(SL_RESULT_SUCCESS, createRecorder(engineItf, &recorderObj,
            &recordItf, &bqItf));

    RecordCallbackData cbData{};
    memset(cbData.buffers, 0, sizeof(cbData.buffers));

    ASSERT_EQ(SL_RESULT_SUCCESS,
            (*bqItf)->RegisterCallback(bqItf, bufferQueueCallback, &cbData));

    // Enqueue initial buffers
    for (int i = 0; i < kNumBuffers; i++) {
        ASSERT_EQ(SL_RESULT_SUCCESS,
                (*bqItf)->Enqueue(bqItf, cbData.buffers[i], kBufferSizeBytes));
    }

    // Start recording.
    ASSERT_EQ(SL_RESULT_SUCCESS, (*recordItf)->SetRecordState(recordItf, SL_RECORDSTATE_RECORDING));

    // Wait for callbacks to deliver data.
    EXPECT_TRUE(cbData.waitForFinished())
            << "Timed out waiting for buffer queue callbacks";
    EXPECT_TRUE(cbData.hasNonSilent)
            << "OpenSL ES recorder should produce non-silent spoofed data";
    EXPECT_GT(cbData.callbackCount.load(), 0)
            << "Buffer queue callback should have been invoked at least once";

    (*recordItf)->SetRecordState(recordItf, SL_RECORDSTATE_STOPPED);
    (*recorderObj)->Destroy(recorderObj);
    (*engineObj)->Destroy(engineObj);
}

TEST_F(OpenSLESSpoofedTest, Record_I16Stereo_ProducesNonSilentData) {
    SLObjectItf engineObj = nullptr;
    SLEngineItf engineItf = nullptr;
    ASSERT_EQ(SL_RESULT_SUCCESS, createEngine(&engineObj, &engineItf));

    SLObjectItf recorderObj = nullptr;
    SLRecordItf recordItf = nullptr;
    SLAndroidSimpleBufferQueueItf bqItf = nullptr;
    ASSERT_EQ(SL_RESULT_SUCCESS, createRecorder(engineItf, &recorderObj,
            &recordItf, &bqItf, kSampleRate, 2));

    RecordCallbackData cbData{};
    memset(cbData.buffers, 0, sizeof(cbData.buffers));

    ASSERT_EQ(SL_RESULT_SUCCESS, (*bqItf)->RegisterCallback(bqItf, bufferQueueCallback, &cbData));

    for (int i = 0; i < kNumBuffers; i++) {
        ASSERT_EQ(SL_RESULT_SUCCESS,
                (*bqItf)->Enqueue(bqItf, cbData.buffers[i], kBufferSizeBytes));
    }

    ASSERT_EQ(SL_RESULT_SUCCESS, (*recordItf)->SetRecordState(recordItf, SL_RECORDSTATE_RECORDING));

    EXPECT_TRUE(cbData.waitForFinished());
    EXPECT_TRUE(cbData.hasNonSilent)
            << "Stereo OpenSL ES recorder should produce non-silent spoofed data";

    (*recordItf)->SetRecordState(recordItf, SL_RECORDSTATE_STOPPED);
    (*recorderObj)->Destroy(recorderObj);
    (*engineObj)->Destroy(engineObj);
}

TEST_F(OpenSLESSpoofedTest, StartStop_StateTransitionsCorrectly) {
    SLObjectItf engineObj = nullptr;
    SLEngineItf engineItf = nullptr;
    ASSERT_EQ(SL_RESULT_SUCCESS, createEngine(&engineObj, &engineItf));

    SLObjectItf recorderObj = nullptr;
    SLRecordItf recordItf = nullptr;
    SLAndroidSimpleBufferQueueItf bqItf = nullptr;
    ASSERT_EQ(SL_RESULT_SUCCESS, createRecorder(engineItf, &recorderObj, &recordItf, &bqItf));

    // Initial state should be stopped
    SLuint32 state = 0;
    EXPECT_EQ(SL_RESULT_SUCCESS, (*recordItf)->GetRecordState(recordItf, &state));
    EXPECT_EQ(SL_RECORDSTATE_STOPPED, state);

    // Enqueue a buffer so recording can proceed.
    RecordCallbackData cbData{};
    (*bqItf)->RegisterCallback(bqItf, bufferQueueCallback, &cbData);
    (*bqItf)->Enqueue(bqItf, cbData.buffers[0], kBufferSizeBytes);

    // Start recording.
    EXPECT_EQ(SL_RESULT_SUCCESS, (*recordItf)->SetRecordState(recordItf, SL_RECORDSTATE_RECORDING));
    EXPECT_EQ(SL_RESULT_SUCCESS, (*recordItf)->GetRecordState(recordItf, &state));
    EXPECT_EQ(SL_RECORDSTATE_RECORDING, state);

    // Stop.
    EXPECT_EQ(SL_RESULT_SUCCESS, (*recordItf)->SetRecordState(recordItf, SL_RECORDSTATE_STOPPED));
    EXPECT_EQ(SL_RESULT_SUCCESS, (*recordItf)->GetRecordState(recordItf, &state));
    EXPECT_EQ(SL_RECORDSTATE_STOPPED, state);

    (*recorderObj)->Destroy(recorderObj);
    (*engineObj)->Destroy(engineObj);
}

TEST_F(OpenSLESSpoofedTest, Record_8kHz_ProducesNonSilentData) {
    SLObjectItf engineObj = nullptr;
    SLEngineItf engineItf = nullptr;
    ASSERT_EQ(SL_RESULT_SUCCESS, createEngine(&engineObj, &engineItf));

    SLObjectItf recorderObj = nullptr;
    SLRecordItf recordItf = nullptr;
    SLAndroidSimpleBufferQueueItf bqItf = nullptr;
    ASSERT_EQ(SL_RESULT_SUCCESS, createRecorder(engineItf, &recorderObj,
            &recordItf, &bqItf, 8000, 1));

    RecordCallbackData cbData{};
    memset(cbData.buffers, 0, sizeof(cbData.buffers));

    ASSERT_EQ(SL_RESULT_SUCCESS, (*bqItf)->RegisterCallback(bqItf, bufferQueueCallback, &cbData));

    for (int i = 0; i < kNumBuffers; i++) {
        ASSERT_EQ(SL_RESULT_SUCCESS, (*bqItf)->Enqueue(bqItf, cbData.buffers[i], kBufferSizeBytes));
    }

    ASSERT_EQ(SL_RESULT_SUCCESS, (*recordItf)->SetRecordState(recordItf, SL_RECORDSTATE_RECORDING));

    EXPECT_TRUE(cbData.waitForFinished());
    EXPECT_TRUE(cbData.hasNonSilent)
            << "8 kHz OpenSL ES recorder should produce non-silent spoofed data";

    (*recordItf)->SetRecordState(recordItf, SL_RECORDSTATE_STOPPED);
    (*recorderObj)->Destroy(recorderObj);
    (*engineObj)->Destroy(engineObj);
}

TEST_F(OpenSLESSpoofedTest, TwoConcurrentRecorders_BothProduceData) {
    SLObjectItf engineObj = nullptr;
    SLEngineItf engineItf = nullptr;
    ASSERT_EQ(SL_RESULT_SUCCESS, createEngine(&engineObj, &engineItf));

    // First recorder: 48 kHz mono
    SLObjectItf recObj1 = nullptr;
    SLRecordItf recItf1 = nullptr;
    SLAndroidSimpleBufferQueueItf bqItf1 = nullptr;
    ASSERT_EQ(SL_RESULT_SUCCESS, createRecorder(engineItf, &recObj1,
            &recItf1, &bqItf1, 48000, 1));

    // Second recorder: 44.1 kHz mono
    SLObjectItf recObj2 = nullptr;
    SLRecordItf recItf2 = nullptr;
    SLAndroidSimpleBufferQueueItf bqItf2 = nullptr;
    ASSERT_EQ(SL_RESULT_SUCCESS, createRecorder(engineItf, &recObj2, &recItf2, &bqItf2, 44100, 1));

    RecordCallbackData cbData1{};
    RecordCallbackData cbData2{};
    memset(cbData1.buffers, 0, sizeof(cbData1.buffers));
    memset(cbData2.buffers, 0, sizeof(cbData2.buffers));

    (*bqItf1)->RegisterCallback(bqItf1, bufferQueueCallback, &cbData1);
    (*bqItf2)->RegisterCallback(bqItf2, bufferQueueCallback, &cbData2);

    for (int i = 0; i < kNumBuffers; i++) {
        (*bqItf1)->Enqueue(bqItf1, cbData1.buffers[i], kBufferSizeBytes);
        (*bqItf2)->Enqueue(bqItf2, cbData2.buffers[i], kBufferSizeBytes);
    }

    (*recItf1)->SetRecordState(recItf1, SL_RECORDSTATE_RECORDING);
    (*recItf2)->SetRecordState(recItf2, SL_RECORDSTATE_RECORDING);

    EXPECT_TRUE(cbData1.waitForFinished());
    EXPECT_TRUE(cbData2.waitForFinished());
    EXPECT_TRUE(cbData1.hasNonSilent) << "First concurrent recorder should produce non-silent data";
    EXPECT_TRUE(cbData2.hasNonSilent) << "Second concurrent recorder should produce non-silent data";

    (*recItf1)->SetRecordState(recItf1, SL_RECORDSTATE_STOPPED);
    (*recItf2)->SetRecordState(recItf2, SL_RECORDSTATE_STOPPED);
    (*recObj1)->Destroy(recObj1);
    (*recObj2)->Destroy(recObj2);
    (*engineObj)->Destroy(engineObj);
}

}
