// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C"
{
#endif

    typedef struct TtaHandle TtaHandle;

    typedef struct
    {
        uint32_t sample_rate;
        uint32_t total_samples;
        uint16_t channels;
        uint16_t bits_per_sample;
    } TtaStreamInfo;

    // The caller owns context and the source. They must outlive the decoder.
    // read: return bytes copied (0 = EOF, negative = error), never more than size.
    // seek: origin 0/1/2 = start/current/end; return absolute position or -1.
    // Callbacks run synchronously and must not throw, unwind, or reenter the handle.
    typedef struct
    {
        void *context;
        int64_t (*read)(void *context, uint8_t *output, size_t size);
        int64_t (*seek)(void *context, int64_t offset, int32_t origin);
    } TtaIo;

    TtaHandle *tta_open(TtaIo io, TtaStreamInfo *info, char *error, size_t error_capacity);
    void tta_close(TtaHandle *handle);
    // Returns channel samples written, 0 for EOF/zero capacity, or -1 on error.
    // PCM is interleaved native-endian S32, with source samples shifted to full scale.
    // Output must fit one whole sample frame; partial trailing capacity is ignored.
    int64_t tta_read(TtaHandle *handle, int32_t *output, size_t capacity);
    // Seeks in sample frames, not milliseconds. Clears errors; clamps to EOF.
    int32_t tta_seek(TtaHandle *handle, uint64_t sample);
    uint64_t tta_position(const TtaHandle *handle);
    // Borrowed NUL-terminated text, invalidated by any subsequent handle operation.
    const char *tta_last_error(const TtaHandle *handle);

    // Each handle requires exclusive access; separate handles may run concurrently.
    // Close each handle exactly once, before releasing its callback context.

#ifdef __cplusplus
}
#endif
