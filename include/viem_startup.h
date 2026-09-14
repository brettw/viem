#ifndef VIEM_STARTUP_H
#define VIEM_STARTUP_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Input: UTF-8 JSON argv array, without argv[0]. Output: UTF-8 JSON
 * {arguments: {filenames, splitCount, initialLine}, error}. Parse errors use
 * null arguments and a message; the ABI call succeeds. Null/zero output queries
 * required bytes and returns BUFFER_TOO_SMALL. Pointer regions are disjoint. */
uint32_t viem_parse_launch_arguments(const uint8_t *input, uint64_t input_length,
                                   uint8_t *output, uint64_t capacity,
                                   uint64_t *required);
#ifdef __cplusplus
}
#endif
#endif
