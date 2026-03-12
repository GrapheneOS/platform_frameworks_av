#include <fcntl.h>
#include <gtest/gtest.h>
#include <mic_spoofing.h>
#include <sys/mman.h>
#include <system/audio.h>
#include <unistd.h>

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <vector>

namespace {

constexpr size_t kFrameCount = 4800;
constexpr size_t kFactoryFrameCount = kFrameCount * 3;
constexpr uint32_t kSampleRate = 48000;
constexpr uint32_t kMono = 1;
constexpr uint32_t kStereo = 2;
constexpr float kTwoPi = 6.28318530717958647692f;

int32_t currentUid() {
    return static_cast<int32_t>(getuid());
}

std::vector<float> makeTestSamples() {
    std::vector<float> samples(kFactoryFrameCount);
    for (size_t i = 0; i < samples.size(); ++i) {
        float phase = (static_cast<float>(i) * 440.0f * kTwoPi) / kSampleRate;
        samples[i] = std::sin(phase) * 0.8f;
    }
    return samples;
}

int writeAll(int fd, const uint8_t *data, size_t size) {
    size_t written = 0;
    while (written < size) {
        ssize_t result = TEMP_FAILURE_RETRY(write(fd, data + written, size - written));
        if (result <= 0) {
            return -1;
        }
        written += static_cast<size_t>(result);
    }
    return 0;
}

int testDecoderFactory(
        int sourceFd,
        uint32_t *outSampleRate,
        uint32_t *outChannelCount
) {
    close(sourceFd);

    if (outSampleRate == nullptr || outChannelCount == nullptr) {
        return -1;
    }

    int fd = memfd_create("MicSpoofingTestDecoder", MFD_CLOEXEC);
    if (fd < 0) {
        return -1;
    }

    *outSampleRate = kSampleRate;
    *outChannelCount = kMono;

    const std::vector<float> samples = makeTestSamples();
    const auto *bytes = reinterpret_cast<const uint8_t *>(samples.data());
    const size_t size = samples.size() * sizeof(float);
    if (writeAll(fd, bytes, size) != 0) {
        close(fd);
        return -1;
    }

    if (lseek(fd, 0, SEEK_SET) < 0) {
        close(fd);
        return -1;
    }

    return fd;
}

void ensureDecoderFactoryRegistered() {
    static const bool registered = []() {
        mic_spoofing_set_decoder_factory(&testDecoderFactory);
        return true;
    }();
    (void)registered;
}

class MicSpoofingSourceTest : public ::testing::Test {
protected:
    void SetUp() override {
        ensureDecoderFactoryRegistered();
        source_ = mic_spoofing_create_source(currentUid());
        ASSERT_NE(source_, nullptr)
                << "Failed to create source; ensure spoofed_mic_audio_default.wav is at /system/etc/";
    }

    void TearDown() override {
        if (source_) {
            mic_spoofing_destroy_source(source_);
            source_ = nullptr;
        }
    }

    void *source_ = nullptr;
};


TEST(MicSpoofing, CreateSource_ReturnsNonNull) {
    ensureDecoderFactoryRegistered();
    void *source = mic_spoofing_create_source(currentUid());
    EXPECT_NE(source, nullptr);
    mic_spoofing_destroy_source(source);
}

TEST(MicSpoofing, DestroyNullSource_NoOp) {
    mic_spoofing_destroy_source(nullptr);
}

TEST(MicSpoofing, StartStreamingDecoder_NullOutputs_ReturnsFailure) {
    EXPECT_EQ(mic_spoofing_start_streaming_decoder(0, nullptr, nullptr), -1);
}

TEST(MicSpoofing, PendingSourceFd_ApiCallsAreSafe) {
    int fd = open("/dev/null", O_RDONLY | O_CLOEXEC);
    ASSERT_GE(fd, 0);

    mic_spoofing_set_pending_source_fd(fd, kSampleRate, kMono);
    mic_spoofing_clear_pending_source_fd();
    mic_spoofing_clear_pending_source_fd();
}

TEST(MicSpoofing, StartStreamingDecoder_CurrentUid_ReturnsReadablePipe) {
    ensureDecoderFactoryRegistered();
    uint32_t sampleRate = 0;
    uint32_t channelCount = 0;
    int fd = mic_spoofing_start_streaming_decoder(currentUid(), &sampleRate, &channelCount);
    ASSERT_GE(fd, 0);
    EXPECT_GT(sampleRate, 0u);
    EXPECT_GT(channelCount, 0u);

    std::vector<float> buf(256, 0.0f);
    ssize_t bytes = TEMP_FAILURE_RETRY(read(fd, buf.data(), buf.size() * sizeof(float)));
    EXPECT_GT(bytes, 0);
    EXPECT_TRUE(std::any_of(buf.begin(), buf.end(), [](float sample) { return sample != 0.0f; }));
    close(fd);
}

TEST(MicSpoofing, CreateDestroyMultipleSources) {
    ensureDecoderFactoryRegistered();
    void *s1 = mic_spoofing_create_source(currentUid());
    void *s2 = mic_spoofing_create_source(currentUid());
    ASSERT_NE(s1, nullptr);
    ASSERT_NE(s2, nullptr);
    EXPECT_NE(s1, s2) << "Each source should be a unique allocation";
    mic_spoofing_destroy_source(s1);
    mic_spoofing_destroy_source(s2);
}

TEST(MicSpoofing, CreateSource_ForeignUid_FillsSilence) {
    void *source = mic_spoofing_create_source(currentUid() + 1);
    ASSERT_NE(source, nullptr);

    std::vector<int16_t> buf(kFrameCount, 1);
    size_t frames = mic_spoofing_read_samples(
            source, reinterpret_cast<uint8_t *>(buf.data()),
            kFrameCount, kSampleRate, kMono, AUDIO_FORMAT_PCM_16_BIT);
    EXPECT_EQ(frames, kFrameCount);
    EXPECT_TRUE(std::all_of(buf.begin(), buf.end(), [](int16_t sample) { return sample == 0; }));
    mic_spoofing_destroy_source(source);
}

TEST(MicSpoofing, ReadSamples_NullSource_FillsSilence) {
    // 16-bit PCM: silence is zero bytes
    {
        const size_t bufSize = kFrameCount * kMono * sizeof(int16_t);
        std::vector<uint8_t> buf(bufSize, 0xFF);
        size_t frames = mic_spoofing_read_samples(
                nullptr, buf.data(), kFrameCount, kSampleRate, kMono, AUDIO_FORMAT_PCM_16_BIT);
        EXPECT_EQ(frames, kFrameCount);
        EXPECT_TRUE(std::all_of(buf.begin(), buf.end(), [](uint8_t b) { return b == 0; }))
                << "16-bit null-source buffer should be filled with zeros";
    }

    // 8-bit PCM: silence is 128 (unsigned PCM midpoint)
    {
        const size_t bufSize = kFrameCount * kMono;
        std::vector<uint8_t> buf(bufSize, 0);
        size_t frames = mic_spoofing_read_samples(
                nullptr, buf.data(), kFrameCount, kSampleRate, kMono, AUDIO_FORMAT_PCM_8_BIT);
        EXPECT_EQ(frames, kFrameCount);
        EXPECT_TRUE(std::all_of(buf.begin(), buf.end(), [](uint8_t b) { return b == 128; }))
                << "8-bit null-source buffer should be filled with 128";
    }
}

TEST_F(MicSpoofingSourceTest, ReadSamples_Pcm16Bit_NonSilent) {
    const size_t sampleCount = kFrameCount * kMono;
    std::vector<int16_t> buf(sampleCount, 0);
    size_t frames = mic_spoofing_read_samples(
            source_, reinterpret_cast<uint8_t *>(buf.data()),
            kFrameCount, kSampleRate, kMono, AUDIO_FORMAT_PCM_16_BIT);
    EXPECT_EQ(frames, kFrameCount);
    EXPECT_TRUE(std::any_of(buf.begin(), buf.end(), [](int16_t s) { return s != 0; }))
            << "16-bit output should contain non-silent samples";
}

TEST_F(MicSpoofingSourceTest, ReadSamples_PcmFloat_NonSilent) {
    const size_t sampleCount = kFrameCount * kMono;
    std::vector<float> buf(sampleCount, 0.0f);
    size_t frames = mic_spoofing_read_samples(
            source_, reinterpret_cast<uint8_t *>(buf.data()),
            kFrameCount, kSampleRate, kMono, AUDIO_FORMAT_PCM_FLOAT);
    EXPECT_EQ(frames, kFrameCount);
    EXPECT_TRUE(std::any_of(buf.begin(), buf.end(), [](float s) { return s != 0.0f; }))
            << "Float output should contain non-silent samples";
    for (float s : buf) {
        EXPECT_GE(s, -1.0f);
        EXPECT_LE(s, 1.0f);
    }
}

TEST_F(MicSpoofingSourceTest, ReadSamples_Pcm8Bit_NonSilent) {
    const size_t sampleCount = kFrameCount * kMono;
    std::vector<uint8_t> buf(sampleCount, 128);
    size_t frames = mic_spoofing_read_samples(
            source_, buf.data(), kFrameCount, kSampleRate, kMono, AUDIO_FORMAT_PCM_8_BIT);
    EXPECT_EQ(frames, kFrameCount);
    EXPECT_TRUE(std::any_of(buf.begin(), buf.end(), [](uint8_t s) { return s != 128; }))
            << "8-bit output should contain values other than 128 (silence)";
}

TEST_F(MicSpoofingSourceTest, ReadSamples_Pcm24BitPacked_NonSilent) {
    const size_t byteCount = kFrameCount * kMono * 3;
    std::vector<uint8_t> buf(byteCount, 0);
    size_t frames = mic_spoofing_read_samples(
            source_, buf.data(), kFrameCount, kSampleRate, kMono, AUDIO_FORMAT_PCM_24_BIT_PACKED);
    EXPECT_EQ(frames, kFrameCount);
    bool hasNonZero = false;
    for (size_t i = 0; i < kFrameCount && !hasNonZero; i++) {
        size_t base = i * 3;
        if (buf[base] != 0 || buf[base + 1] != 0 || buf[base + 2] != 0) {
            hasNonZero = true;
        }
    }
    EXPECT_TRUE(hasNonZero) << "24-bit packed output should contain non-silent samples";
}

TEST_F(MicSpoofingSourceTest, ReadSamples_Pcm32Bit_NonSilent) {
    const size_t sampleCount = kFrameCount * kMono;
    std::vector<int32_t> buf(sampleCount, 0);
    size_t frames = mic_spoofing_read_samples(
            source_, reinterpret_cast<uint8_t *>(buf.data()),
            kFrameCount, kSampleRate, kMono, AUDIO_FORMAT_PCM_32_BIT);
    EXPECT_EQ(frames, kFrameCount);
    EXPECT_TRUE(std::any_of(buf.begin(), buf.end(), [](int32_t s) { return s != 0; }))
            << "32-bit output should contain non-silent samples";
}

TEST_F(MicSpoofingSourceTest, ReadSamples_ChannelConversion) {
    // Mono source read as stereo: L and R channels should be identical
    const size_t sampleCount = kFrameCount * kStereo;
    std::vector<int16_t> buf(sampleCount, 0);
    size_t frames = mic_spoofing_read_samples(
            source_, reinterpret_cast<uint8_t *>(buf.data()),
            kFrameCount, kSampleRate, kStereo, AUDIO_FORMAT_PCM_16_BIT);
    EXPECT_EQ(frames, kFrameCount);
    bool hasNonSilent = false;
    for (size_t i = 0; i < kFrameCount; i++) {
        int16_t l = buf[i * 2];
        int16_t r = buf[i * 2 + 1];
        EXPECT_EQ(l, r) << "Mono->stereo: L and R should match at frame " << i;
        if (l != 0) hasNonSilent = true;
    }
    EXPECT_TRUE(hasNonSilent) << "Stereo output should be non-silent";
}

TEST_F(MicSpoofingSourceTest, ReadSamples_SampleRateConversion) {
    constexpr uint32_t kTargetRate = 16000;
    constexpr size_t kTargetFrames = 1600; // 100 ms
    const size_t sampleCount = kTargetFrames * kMono;
    std::vector<int16_t> buf(sampleCount, 0);
    size_t frames = mic_spoofing_read_samples(
            source_, reinterpret_cast<uint8_t *>(buf.data()),
            kTargetFrames, kTargetRate, kMono, AUDIO_FORMAT_PCM_16_BIT);
    EXPECT_EQ(frames, kTargetFrames);
    EXPECT_TRUE(std::any_of(buf.begin(), buf.end(), [](int16_t s) { return s != 0; }))
            << "Resampled output should be non-silent";
}

TEST_F(MicSpoofingSourceTest, ReadSamples_UnknownFormat_FillsSilence) {
    constexpr uint32_t kUnknownFormat = 0xFFu;
    // Unknown format defaults to 2 bytes per sample in fill_silence
    const size_t bufSize = kFrameCount * kMono * 2;
    std::vector<uint8_t> buf(bufSize, 0xFF);
    size_t frames = mic_spoofing_read_samples(
            source_, buf.data(), kFrameCount, kSampleRate, kMono, kUnknownFormat);
    EXPECT_EQ(frames, kFrameCount);
    EXPECT_TRUE(std::all_of(buf.begin(), buf.end(), [](uint8_t b) { return b == 0; }))
            << "Unknown format should produce silence (zeros)";
}

TEST_F(MicSpoofingSourceTest, ReadSamples_ZeroFrames_ReturnsZero) {
    uint8_t buf[64];
    size_t frames = mic_spoofing_read_samples(
            source_, buf, 0, kSampleRate, kMono, AUDIO_FORMAT_PCM_16_BIT);
    EXPECT_EQ(frames, 0u);
}

TEST_F(MicSpoofingSourceTest, ReadSamples_ZeroSampleRate_ReturnsZero) {
    std::vector<uint8_t> buf(kFrameCount * kMono * sizeof(int16_t));
    size_t frames = mic_spoofing_read_samples(
            source_, buf.data(), kFrameCount, 0, kMono, AUDIO_FORMAT_PCM_16_BIT);
    EXPECT_EQ(frames, 0u);
}

TEST_F(MicSpoofingSourceTest, ReadSamples_ZeroChannelCount_ReturnsZero) {
    std::vector<uint8_t> buf(kFrameCount * sizeof(int16_t));
    size_t frames = mic_spoofing_read_samples(
            source_, buf.data(), kFrameCount, kSampleRate, 0, AUDIO_FORMAT_PCM_16_BIT);
    EXPECT_EQ(frames, 0u);
}

TEST_F(MicSpoofingSourceTest, ReadSamples_NullBuffer_ReturnsZero) {
    size_t frames = mic_spoofing_read_samples(
            source_, nullptr, kFrameCount, kSampleRate, kMono, AUDIO_FORMAT_PCM_16_BIT);
    EXPECT_EQ(frames, 0u);
}

TEST_F(MicSpoofingSourceTest, ReadSamples_OverflowFrameCount_ReturnsZero) {
    uint8_t buf[64];
    // frame_count * channel_count overflows size_t
    size_t hugeFrameCount = SIZE_MAX / 2 + 1;
    size_t frames = mic_spoofing_read_samples(
            source_, buf, hugeFrameCount, kSampleRate, kStereo, AUDIO_FORMAT_PCM_16_BIT);
    EXPECT_EQ(frames, 0u);
}

TEST_F(MicSpoofingSourceTest, ReadSamples_Pcm24BitPacked_OverflowByteCount_ReturnsZero) {
    uint8_t buf[64];
    // frame_count * channel_count fits in size_t, but sample_count * 3 overflows
    size_t frameCount = SIZE_MAX / 6 + 1;
    size_t frames = mic_spoofing_read_samples(
            source_, buf, frameCount, kSampleRate, kStereo, AUDIO_FORMAT_PCM_24_BIT_PACKED);
    EXPECT_EQ(frames, 0u);
}

TEST_F(MicSpoofingSourceTest, ReadSamples_ConsecutiveReads_AdvancePosition) {
    const size_t sampleCount = kFrameCount * kMono;
    std::vector<int16_t> buf1(sampleCount, 0);
    std::vector<int16_t> buf2(sampleCount, 0);

    mic_spoofing_read_samples(
            source_, reinterpret_cast<uint8_t *>(buf1.data()),
            kFrameCount, kSampleRate, kMono, AUDIO_FORMAT_PCM_16_BIT);
    mic_spoofing_read_samples(
            source_, reinterpret_cast<uint8_t *>(buf2.data()),
            kFrameCount, kSampleRate, kMono, AUDIO_FORMAT_PCM_16_BIT);

    // Two consecutive reads from the same source should return different data
    // because the internal read position advances
    EXPECT_NE(buf1, buf2) << "Consecutive reads should produce different data";
}

TEST_F(MicSpoofingSourceTest, ReadSamples_SingleFrame_Succeeds) {
    int16_t sample = 0;
    size_t frames = mic_spoofing_read_samples(
            source_, reinterpret_cast<uint8_t *>(&sample),
            1, kSampleRate, kMono, AUDIO_FORMAT_PCM_16_BIT);
    EXPECT_EQ(frames, 1u);
}

TEST_F(MicSpoofingSourceTest, ReadSamples_MisalignedBuffer_Pcm16Bit_FillsSilence) {
    // A buffer misaligned for int16_t should trigger the alignment guard and
    // fill silence instead of reading source data
    const size_t bufSize = kFrameCount * sizeof(int16_t) + 1;
    std::vector<uint8_t> raw(bufSize, 0xFF);
    uint8_t *misaligned = raw.data();
    if (reinterpret_cast<uintptr_t>(misaligned) % alignof(int16_t) == 0) {
        misaligned += 1;
    }

    size_t frames = mic_spoofing_read_samples(
            source_, misaligned, kFrameCount, kSampleRate, kMono, AUDIO_FORMAT_PCM_16_BIT);
    EXPECT_EQ(frames, kFrameCount);
    const size_t byteCount = kFrameCount * kMono * sizeof(int16_t);
    for (size_t i = 0; i < byteCount; i++) {
        EXPECT_EQ(misaligned[i], 0) << "Misaligned 16-bit buffer should be silence at byte " << i;
    }
}

TEST_F(MicSpoofingSourceTest, ReadSamples_MisalignedBuffer_PcmFloat_FillsSilence) {
    // A buffer misaligned for float should trigger the alignment guard
    const size_t bufSize = kFrameCount * sizeof(float) + 3;
    std::vector<uint8_t> raw(bufSize, 0xFF);
    uint8_t *misaligned = raw.data();
    if (reinterpret_cast<uintptr_t>(misaligned) % alignof(float) == 0) {
        misaligned += 2; // 2-byte aligned but not 4-byte aligned
    }

    size_t frames = mic_spoofing_read_samples(
            source_, misaligned, kFrameCount, kSampleRate, kMono, AUDIO_FORMAT_PCM_FLOAT);
    EXPECT_EQ(frames, kFrameCount);
    const size_t byteCount = kFrameCount * kMono * sizeof(float);
    for (size_t i = 0; i < byteCount; i++) {
        EXPECT_EQ(misaligned[i], 0) << "Misaligned float buffer should be silence at byte " << i;
    }
}

TEST_F(MicSpoofingSourceTest, ReadSamples_MisalignedBuffer_Pcm32Bit_FillsSilence) {
    // A buffer misaligned for int32_t should trigger the alignment guard
    const size_t bufSize = kFrameCount * sizeof(int32_t) + 3;
    std::vector<uint8_t> raw(bufSize, 0xFF);
    uint8_t *misaligned = raw.data();
    if (reinterpret_cast<uintptr_t>(misaligned) % alignof(int32_t) == 0) {
        misaligned += 2; // 2-byte aligned but not 4-byte aligned
    }

    size_t frames = mic_spoofing_read_samples(
            source_, misaligned, kFrameCount, kSampleRate, kMono, AUDIO_FORMAT_PCM_32_BIT);
    EXPECT_EQ(frames, kFrameCount);
    const size_t byteCount = kFrameCount * kMono * sizeof(int32_t);
    for (size_t i = 0; i < byteCount; i++) {
        EXPECT_EQ(misaligned[i], 0) << "Misaligned 32-bit buffer should be silence at byte " << i;
    }
}

TEST_F(MicSpoofingSourceTest, ReadSamples_ExcessiveFrameCount_ReturnsZero) {
    // The Rust FFI checks sample_count > MAX_READ_SAMPLES (2,880,000) and returns 0
    constexpr size_t kExcessiveFrames = 3000000;
    const size_t bufSize = kExcessiveFrames * kMono * sizeof(int16_t);
    std::vector<uint8_t> buf(bufSize, 0xAB);
    size_t frames = mic_spoofing_read_samples(
            source_, buf.data(), kExcessiveFrames, kSampleRate, kMono,
            AUDIO_FORMAT_PCM_16_BIT);
    EXPECT_EQ(frames, 0u);
    // Buffer should remain untouched (sentinel 0xAB).
    EXPECT_TRUE(std::all_of(buf.begin(), buf.end(), [](uint8_t b) { return b == 0xAB; }))
            << "Buffer should be untouched when frame count exceeds MAX_READ_SAMPLES";
}

TEST_F(MicSpoofingSourceTest, ReadSamples_ChannelCountExceedsU16Max_ReturnsZero) {
    // The Rust FFI checks channel_count > u16::MAX and returns 0
    constexpr uint32_t kHugeChannelCount = 70000;
    uint8_t buf[64] = {};
    size_t frames = mic_spoofing_read_samples(
            source_, buf, 100, kSampleRate, kHugeChannelCount, AUDIO_FORMAT_PCM_16_BIT);
    EXPECT_EQ(frames, 0u);
}

}
