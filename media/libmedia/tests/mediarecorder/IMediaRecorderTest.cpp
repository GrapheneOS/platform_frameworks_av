#include <android-base/unique_fd.h>
#include <binder/Binder.h>
#include <binder/IServiceManager.h>
#include <errno.h>
#include <gtest/gtest.h>
#include <media/IMediaPlayerService.h>
#include <media/IMediaRecorder.h>
#include <signal.h>
#include <string>
#include <unistd.h>

namespace android {

using android::content::AttributionSourceState;

class IMediaRecorderTest : public testing::Test {
  protected:
    static void SetUpTestSuite() {
        sPreviousSigpipeHandler = signal(SIGPIPE, SIG_IGN);
    }

    static void TearDownTestSuite() {
        signal(SIGPIPE, sPreviousSigpipeHandler);
    }

    void SetUp() override {
        sp<IServiceManager> serviceManager = defaultServiceManager();
        ASSERT_NE(serviceManager, nullptr);

        sp<IBinder> mediaPlayerService = serviceManager->waitForService(String16("media.player"));
        ASSERT_NE(mediaPlayerService, nullptr);

        sp<IMediaPlayerService> service = IMediaPlayerService::asInterface(mediaPlayerService);
        ASSERT_NE(service, nullptr);

        AttributionSourceState attributionSource;
        attributionSource.packageName = std::string("IMediaRecorderTest");
        attributionSource.token = sp<BBinder>::make();
        recorder_ = service->createMediaRecorder(attributionSource);
        ASSERT_NE(recorder_, nullptr);
    }

    void TearDown() override {
        if (recorder_ != nullptr) {
            ASSERT_EQ(recorder_->release(), OK);
            recorder_.clear();
        }
    }

    static void CreatePipe(base::unique_fd* readFd, base::unique_fd* writeFd) {
        int fds[2];
        ASSERT_EQ(pipe(fds), 0);
        readFd->reset(fds[0]);
        writeFd->reset(fds[1]);
    }

    static void ExpectPipeBroken(const base::unique_fd& writeFd) {
        ASSERT_TRUE(writeFd.ok());
        const uint8_t sample = 0x5a;
        errno = 0;
        ASSERT_EQ(TEMP_FAILURE_RETRY(write(writeFd.get(), &sample, sizeof(sample))), -1);
        ASSERT_EQ(errno, EPIPE);
    }

    sp<IMediaRecorder> recorder_;

    static sighandler_t sPreviousSigpipeHandler;
};

sighandler_t IMediaRecorderTest::sPreviousSigpipeHandler = SIG_DFL;

TEST_F(IMediaRecorderTest, SetMicSpoofingSourceFdReplacesPreviousPipe) {
    base::unique_fd firstRead;
    base::unique_fd firstWrite;
    CreatePipe(&firstRead, &firstWrite);
    ASSERT_EQ(recorder_->setMicSpoofingSourceFd(firstRead.get(), 48000, 2), OK);
    firstRead.reset();

    base::unique_fd secondRead;
    base::unique_fd secondWrite;
    CreatePipe(&secondRead, &secondWrite);
    ASSERT_EQ(recorder_->setMicSpoofingSourceFd(secondRead.get(), 44100, 1), OK);
    secondRead.reset();

    ExpectPipeBroken(firstWrite);
}

TEST_F(IMediaRecorderTest, StartFailureClearsPendingSpoofedSourceState) {
    base::unique_fd readFd;
    base::unique_fd writeFd;
    CreatePipe(&readFd, &writeFd);
    ASSERT_EQ(recorder_->setMicSpoofingSourceFd(readFd.get(), 48000, 2), OK);
    readFd.reset();

    ASSERT_NE(recorder_->start(), OK);
    ExpectPipeBroken(writeFd);
}

TEST_F(IMediaRecorderTest, ResetClearsStoredSpoofedSourceState) {
    base::unique_fd readFd;
    base::unique_fd writeFd;
    CreatePipe(&readFd, &writeFd);
    ASSERT_EQ(recorder_->setMicSpoofingSourceFd(readFd.get(), 48000, 2), OK);
    readFd.reset();

    ASSERT_EQ(recorder_->reset(), OK);
    ExpectPipeBroken(writeFd);
}

TEST_F(IMediaRecorderTest, CloseClearsStoredSpoofedSourceState) {
    base::unique_fd readFd;
    base::unique_fd writeFd;
    CreatePipe(&readFd, &writeFd);
    ASSERT_EQ(recorder_->setMicSpoofingSourceFd(readFd.get(), 48000, 2), OK);
    readFd.reset();

    ASSERT_EQ(recorder_->init(), OK);
    ASSERT_EQ(recorder_->close(), OK);
    ExpectPipeBroken(writeFd);
}

TEST_F(IMediaRecorderTest, ReleaseClearsStoredSpoofedSourceState) {
    base::unique_fd readFd;
    base::unique_fd writeFd;
    CreatePipe(&readFd, &writeFd);
    ASSERT_EQ(recorder_->setMicSpoofingSourceFd(readFd.get(), 48000, 2), OK);
    readFd.reset();

    ASSERT_EQ(recorder_->release(), OK);
    recorder_.clear();
    ExpectPipeBroken(writeFd);
}

}
