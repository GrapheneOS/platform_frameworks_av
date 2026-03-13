#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

bool mic_spoofing_is_enabled_for_uid(int32_t uid);

typedef int (*mic_spoofing_decoder_factory_fn)(int source_fd, uint32_t *out_sample_rate,
        uint32_t *out_channel_count);

void mic_spoofing_set_decoder_factory(mic_spoofing_decoder_factory_fn factory);
int mic_spoofing_start_streaming_decoder(int32_t uid, uint32_t *out_sample_rate,
        uint32_t *out_channel_count);
void mic_spoofing_set_pending_source_fd(int fd, uint32_t source_sample_rate,
        uint32_t source_channel_count);
void mic_spoofing_clear_pending_source_fd(void);

void *mic_spoofing_create_source(int32_t uid);
void mic_spoofing_destroy_source(void *source);
size_t mic_spoofing_read_samples(void *source, uint8_t *buffer, size_t frame_count,
        uint32_t sample_rate, uint32_t channel_count, uint32_t audio_format);

#ifdef __cplusplus
}
#endif
