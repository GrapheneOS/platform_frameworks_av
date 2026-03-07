#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

bool mic_spoofing_is_enabled_for_uid(int32_t uid);
void *mic_spoofing_create_source(int32_t uid);
void mic_spoofing_destroy_source(void *source);
size_t mic_spoofing_read_samples(void *source, uint8_t *buffer, size_t frame_count,
        uint32_t sample_rate, uint32_t channel_count, uint32_t audio_format);

#ifdef __cplusplus
}
#endif
