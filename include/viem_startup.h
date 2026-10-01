#ifndef VIEM_STARTUP_H
#define VIEM_STARTUP_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Input: UTF-8 JSON argv array, without argv[0]. Output: UTF-8 JSON
 * {arguments: {filenames, splitCount, verticalSplits, initialLine}, error}. Parse errors use
 * null arguments and a message; the ABI call succeeds. Null/zero output queries
 * required bytes and returns BUFFER_TOO_SMALL. Pointer regions are disjoint. */
uint32_t viem_parse_launch_arguments(const uint8_t *input, uint64_t input_length,
                                   uint8_t *output, uint64_t capacity,
                                   uint64_t *required);
/* Load UTF-8 startup commands (at most 1 MiB) before creating any views.
 * Valid lines apply even when other lines fail. Diagnostics are synchronous,
 * one-based, and their message bytes are borrowed only during the callback.
 * The optional callback runs after the serial core lease is released. */
typedef void (*ViemStartupDiagnosticCallback)(void *context, uint64_t line,
                                             const uint8_t *message, uint64_t length);
uint32_t viem_core_initialize_startup(uint64_t core, const uint8_t *input, uint64_t length,
                                    ViemStartupDiagnosticCallback diagnostic, void *context);
#ifdef __cplusplus
}
#endif
#endif
