#pragma once

#include <system/audio.h>

namespace android {

static inline bool micSpoofingAllowsAudioSource(audio_source_t source) {
    // Keep spoofing limited to generic microphone capture sources
    switch (source) {
        case AUDIO_SOURCE_DEFAULT:
        case AUDIO_SOURCE_MIC:
        case AUDIO_SOURCE_VOICE_RECOGNITION:
            return true;
        default:
            return false;
    }
}

} // namespace android
