#ifndef EVIM_CORE_H
#define EVIM_CORE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define EVIM_CORE_ABI_VERSION 3u
#define EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V1 1u
#define EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V2 2u
#define EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION \
  EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V2

typedef uint64_t EvimDocumentHandle;
typedef uint64_t EvimCoreHandle;
typedef uint64_t EvimViewId;
typedef uint32_t EvimStatus;

#define EVIM_STATUS_OK 0u
#define EVIM_STATUS_INVALID_ARGUMENT 1u
#define EVIM_STATUS_NULL_POINTER 2u
#define EVIM_STATUS_INVALID_HANDLE 3u
#define EVIM_STATUS_STALE_REVISION 4u
#define EVIM_STATUS_INVALID_UTF8 5u
#define EVIM_STATUS_INVALID_ENCODING 6u
#define EVIM_STATUS_INVALID_FORMAT 7u
#define EVIM_STATUS_INVALID_FILE_FORMAT 8u
#define EVIM_STATUS_BUFFER_TOO_SMALL 9u
#define EVIM_STATUS_INVALID_RANGE 10u
#define EVIM_STATUS_NOT_GRAPHEME_BOUNDARY 11u
#define EVIM_STATUS_UNREPRESENTABLE_CHARACTER 12u
#define EVIM_STATUS_AMBIGUOUS_PROJECTION 13u
#define EVIM_STATUS_UNSUPPORTED_OPERATION 14u
#define EVIM_STATUS_POLICY_REQUIRED 15u
#define EVIM_STATUS_RESOURCE_EXHAUSTED 16u
#define EVIM_STATUS_VERIFICATION_FAILED 17u
#define EVIM_STATUS_LENGTH_OVERFLOW 18u
#define EVIM_STATUS_DOCUMENT_BUSY 19u
#define EVIM_STATUS_CORE_BUSY 20u
#define EVIM_STATUS_INVALID_VIEW 21u
#define EVIM_STATUS_INVALID_PROVIDER 22u
#define EVIM_STATUS_PROVIDER_FAILURE 23u
#define EVIM_STATUS_INVALID_KEY 24u
#define EVIM_STATUS_CORE_FAILURE 25u
#define EVIM_STATUS_UNSTABLE_SHAPING_CONTEXT 26u
#define EVIM_STATUS_VERTICAL_VIEWPORT_ORIGIN_UNSUPPORTED 27u
#define EVIM_STATUS_INTERNAL_ERROR 254u
#define EVIM_STATUS_PANIC 255u

#define EVIM_ENCODING_UTF8 1u
#define EVIM_ENCODING_LATIN1 2u
#define EVIM_ENCODING_UTF16_LE 3u
#define EVIM_ENCODING_UTF16_BE 4u

#define EVIM_FORMAT_PLAIN_TEXT 1u
#define EVIM_FORMAT_MARKDOWN 2u

#define EVIM_FILE_FORMAT_DETECT 0u
#define EVIM_FILE_FORMAT_UNIX 1u
#define EVIM_FILE_FORMAT_DOS 2u
#define EVIM_FILE_FORMAT_MAC 3u

typedef struct EvimDocumentOptions {
  uint32_t struct_size;
  uint32_t encoding;
  uint32_t format;
  uint32_t file_format;
} EvimDocumentOptions;

#define EVIM_DOCUMENT_OPTIONS_SIZE 16u

#define EVIM_PROVIDER_THREADING_ANY_WORKER 1u
#define EVIM_PROVIDER_THREADING_DEDICATED_SERIAL 2u
#define EVIM_PROVIDER_THREADING_FRONTEND_MAIN 3u

#define EVIM_RENDER_THREADING_ANY 1u
#define EVIM_RENDER_THREADING_DEDICATED_SERIAL 2u
#define EVIM_RENDER_THREADING_FRONTEND_MAIN 3u

#define EVIM_LAYOUT_EXECUTION_WORKER_POOL 1u
#define EVIM_LAYOUT_EXECUTION_DEDICATED_SERIAL 2u
#define EVIM_LAYOUT_EXECUTION_FRONTEND_MAIN 3u

#define EVIM_TEXT_DIRECTION_AUTO 0u
#define EVIM_TEXT_DIRECTION_LEFT_TO_RIGHT 1u
#define EVIM_TEXT_DIRECTION_RIGHT_TO_LEFT 2u

#define EVIM_FONT_SLANT_UPRIGHT 0u
#define EVIM_FONT_SLANT_ITALIC 1u
#define EVIM_FONT_SLANT_OBLIQUE 2u

#define EVIM_SHAPE_PURPOSE_METRICS_ONLY 1u
#define EVIM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA 2u

#define EVIM_BOUNDARY_AFFINITY_UPSTREAM 1u
#define EVIM_BOUNDARY_AFFINITY_DOWNSTREAM 2u

#define EVIM_KEY_CHARACTER 1u
#define EVIM_KEY_ESCAPE 2u
#define EVIM_KEY_ENTER 3u
#define EVIM_KEY_TAB 4u
#define EVIM_KEY_BACKSPACE 5u
#define EVIM_KEY_DELETE 6u
#define EVIM_KEY_LEFT 7u
#define EVIM_KEY_RIGHT 8u
#define EVIM_KEY_UP 9u
#define EVIM_KEY_DOWN 10u
#define EVIM_KEY_HOME 11u
#define EVIM_KEY_END 12u
#define EVIM_KEY_PAGE_UP 13u
#define EVIM_KEY_PAGE_DOWN 14u
#define EVIM_KEY_CONTROL_CHARACTER 15u

#define EVIM_COMMAND_STATUS_NONE 0u
#define EVIM_COMMAND_STATUS_COMPLETE 1u
#define EVIM_COMMAND_STATUS_PENDING 2u
#define EVIM_COMMAND_STATUS_CANCELLED 3u
#define EVIM_COMMAND_STATUS_NEEDS_MORE_LAYOUT 4u
#define EVIM_COMMAND_STATUS_SEARCH_NOT_FOUND 5u
#define EVIM_COMMAND_STATUS_UNSUPPORTED 6u
#define EVIM_COMMAND_STATUS_ERROR 7u

#define EVIM_MODE_NORMAL 1u
#define EVIM_MODE_INSERT 2u
#define EVIM_MODE_REPLACE 3u
#define EVIM_MODE_VISUAL_CHARACTER 4u
#define EVIM_MODE_VISUAL_LINE 5u
#define EVIM_MODE_VISUAL_BLOCK 6u
#define EVIM_MODE_COMMAND_LINE 7u

#define EVIM_OUTCOME_HAS_COMMAND (1u << 0)
#define EVIM_OUTCOME_CURSOR_MOVED (1u << 1)
#define EVIM_OUTCOME_DOCUMENT_CHANGED (1u << 2)
#define EVIM_OUTCOME_MODE_CHANGED (1u << 3)
#define EVIM_OUTCOME_LAYOUT_CHANGED (1u << 4)
#define EVIM_OUTCOME_HAS_POSITION_MAP (1u << 5)
#define EVIM_OUTCOME_HAS_LAYOUT (1u << 6)
#define EVIM_OUTCOME_HAS_EXTERNAL_EFFECTS (1u << 7)
#define EVIM_OUTCOME_HAS_COMPOSITION_CHANGES (1u << 8)

typedef struct EvimUtf8Slice {
  const uint8_t *data;
  uint64_t length;
} EvimUtf8Slice;

typedef struct EvimTextMetricsV1 {
  float ascent;
  float descent;
  float leading;
} EvimTextMetricsV1;

typedef struct EvimShapedBoundsV1 {
  float x;
  float y;
  float width;
  float height;
} EvimShapedBoundsV1;

typedef struct EvimOpenTypeFeatureV1 {
  uint8_t tag[4];
  uint32_t value;
} EvimOpenTypeFeatureV1;

typedef struct EvimResolvedTextStyleV1 {
  uint32_t struct_size;
  uint32_t slant;
  uint32_t direction;
  uint32_t has_language;
  uint32_t has_script;
  uint32_t reserved;
  float size;
  float weight;
  float letter_spacing;
  float baseline_shift;
  const EvimUtf8Slice *font_families;
  uint64_t font_family_count;
  EvimUtf8Slice language;
  EvimUtf8Slice script;
  const EvimOpenTypeFeatureV1 *features;
  uint64_t feature_count;
} EvimResolvedTextStyleV1;

#define EVIM_RESOLVED_TEXT_STYLE_V1_SIZE \
  ((uint32_t)sizeof(EvimResolvedTextStyleV1))

typedef struct EvimShapeStyleRunV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t text_start;
  uint64_t text_end;
  EvimResolvedTextStyleV1 style;
} EvimShapeStyleRunV1;

#define EVIM_SHAPE_STYLE_RUN_V1_SIZE ((uint32_t)sizeof(EvimShapeStyleRunV1))

/*
 * Provider-owned opaque render resource. identifier is an integer token, not
 * a native pointer for core to dereference. The provider keeps the token valid
 * for owner/threading while metrics_generation remains current and the owning
 * view remains attached. It may retire the token when that generation becomes
 * stale or when the view is removed/core is destroyed. ABI v1 has no
 * retain/release transfer; a caller must not retain a stale or detached token.
 */
typedef struct EvimRenderRunHandleV1 {
  uint64_t owner;
  uint64_t identifier;
  uint64_t metrics_generation;
  uint32_t threading;
  uint32_t reserved;
} EvimRenderRunHandleV1;

typedef struct EvimClusterCaretStopV1 {
  uint64_t text_offset;
  float inline_offset;
  uint32_t affinity;
} EvimClusterCaretStopV1;

typedef struct EvimShapedClusterV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t text_start;
  uint64_t text_end;
  float advance;
  EvimTextMetricsV1 metrics;
  EvimShapedBoundsV1 typographic_bounds;
  EvimShapedBoundsV1 ink_bounds;
  uint32_t bidi_level;
  uint32_t has_render_run;
  EvimUtf8Slice fallback_font;
  const EvimClusterCaretStopV1 *caret_stops;
  uint64_t caret_stop_count;
  EvimRenderRunHandleV1 render_run;
} EvimShapedClusterV1;

#define EVIM_SHAPED_CLUSTER_V1_SIZE ((uint32_t)sizeof(EvimShapedClusterV1))

typedef struct EvimShapingDiagnosticV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t text_start;
  uint64_t text_end;
  EvimUtf8Slice message;
} EvimShapingDiagnosticV1;

#define EVIM_SHAPING_DIAGNOSTIC_V1_SIZE \
  ((uint32_t)sizeof(EvimShapingDiagnosticV1))

typedef struct EvimShapeRequestV1 {
  uint32_t struct_size;
  uint32_t purpose;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t measurement_environment_id;
  uint64_t metrics_generation;
  uint64_t text_start;
  uint64_t text_end;
  EvimUtf8Slice text;
  EvimUtf8Slice context_before;
  EvimUtf8Slice context_after;
  const EvimShapeStyleRunV1 *style_runs;
  uint64_t style_run_count;
  EvimResolvedTextStyleV1 default_style;
  float scale;
  uint32_t has_render_run_policy;
  uint64_t render_run_owner;
  uint32_t render_run_threading;
  /*
   * ABI v2 supplies the containing paragraph's EVIM_TEXT_DIRECTION_* value.
   * It also defines text_start..text_end as a stable ownership interior:
   * shape context_before + text + context_after, then return every whole
   * cluster whose logical start lies in the interior. Such a cluster may end
   * in context_after; omit one whose start lies in context_before. The context
   * slices contain immediately adjacent complete grapheme sequences.
   *
   * ABI v1 providers see zero in this fixed-layout slot and may continue to
   * address it as reserved. Their legacy exact-interior responses remain
   * accepted. The anonymous union preserves source and binary compatibility
   * without changing request-array stride.
   */
  union {
    uint32_t paragraph_base_direction;
    uint32_t reserved;
  };
} EvimShapeRequestV1;

#define EVIM_SHAPE_REQUEST_V1_SIZE ((uint32_t)sizeof(EvimShapeRequestV1))

typedef struct EvimShapeResponseV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t measurement_environment_id;
  uint64_t metrics_generation;
  uint64_t text_start;
  uint64_t text_end;
  const EvimShapedClusterV1 *clusters;
  uint64_t cluster_count;
  const uint64_t *visual_order;
  uint64_t visual_order_count;
  EvimTextMetricsV1 default_metrics;
  const EvimShapingDiagnosticV1 *diagnostics;
  uint64_t diagnostic_count;
} EvimShapeResponseV1;

/*
 * A response echoes the request's text_start..text_end ownership interior.
 * Under provider ABI v2, returned cluster ends may extend into context_after;
 * response bounds do not expand to include those cluster tails.
 */

#define EVIM_SHAPE_RESPONSE_V1_SIZE ((uint32_t)sizeof(EvimShapeResponseV1))

typedef uint64_t (*EvimMetricsGenerationCallback)(void *context);
typedef uint32_t (*EvimShapeBatchCallback)(
    void *context, const EvimShapeRequestV1 *requests, uint64_t request_count,
    EvimShapeResponseV1 *responses, uint64_t response_capacity);

/*
 * The provider table is copied by evim_core_view_add. Provider ABI v1 and v2
 * are accepted; v1 receives Auto in paragraph_base_direction. Context and
 * callback functions must remain valid until the view is removed or its core is
 * successfully destroyed. EVIM_STATUS_CORE_BUSY means destruction did not
 * occur and does not end these lifetimes. Every response pointer returned by
 * shape_batch must remain readable until the next provider callback for that
 * view; core copies all values immediately. Opaque render-run tokens instead
 * follow the generation/view lifetime documented on EvimRenderRunHandleV1.
 *
 * A successful ABI-v2 response affirms that its ownership interior is stable
 * under arbitrary text outside the supplied bounded context. A provider that
 * cannot make that guarantee returns EVIM_STATUS_UNSTABLE_SHAPING_CONTEXT from
 * shape_batch instead of returning partial or uncacheable measurements. Core
 * then installs and caches nothing from the failed batch.
 */
typedef struct EvimTextMeasurementProviderV1 {
  uint32_t struct_size;
  uint32_t abi_version;
  void *context;
  uint64_t measurement_environment_id;
  uint32_t threading;
  uint32_t has_render_run_policy;
  uint64_t render_run_owner;
  uint32_t render_run_threading;
  uint32_t reserved;
  EvimMetricsGenerationCallback metrics_generation;
  EvimShapeBatchCallback shape_batch;
} EvimTextMeasurementProviderV1;

#define EVIM_TEXT_MEASUREMENT_PROVIDER_V1_SIZE \
  ((uint32_t)sizeof(EvimTextMeasurementProviderV1))

typedef struct EvimViewOptionsV1 {
  uint32_t struct_size;
  uint32_t execution_context;
  float width;
  float height;
} EvimViewOptionsV1;

#define EVIM_VIEW_OPTIONS_V1_SIZE ((uint32_t)sizeof(EvimViewOptionsV1))

/*
 * Absolute presentation origin. left is always requested. HAS_TOP reserves
 * the vertical form of this versioned request, but core currently returns
 * EVIM_STATUS_VERTICAL_VIEWPORT_ORIGIN_UNSUPPORTED without changing either
 * component when it is set. The expected_* identity is ignored for a
 * horizontal-only request and reserves a revision-bound contract for future
 * vertical support.
 */
typedef struct EvimViewportOriginV1 {
  uint32_t struct_size;
  uint32_t flags;
  float left;
  float top;
  uint64_t expected_document_id;
  uint64_t expected_document_revision;
  uint64_t expected_layout_revision;
  uint64_t expected_configuration_generation;
  uint64_t expected_measurement_environment_id;
  uint64_t expected_metrics_generation;
} EvimViewportOriginV1;

#define EVIM_VIEWPORT_ORIGIN_V1_SIZE \
  ((uint32_t)sizeof(EvimViewportOriginV1))
#define EVIM_VIEWPORT_ORIGIN_HAS_TOP (1u << 0)

#define EVIM_VIEWPORT_STATE_WRAP (1u << 0)
#define EVIM_VIEWPORT_STATE_MAXIMUM_LEFT_EXACT (1u << 1)
#define EVIM_VIEWPORT_STATE_TOP_EXACT (1u << 2)
#define EVIM_VIEWPORT_STATE_HAS_LAYOUT (1u << 3)

/*
 * maximum_left is authoritative only with MAXIMUM_LEFT_EXACT. A missing
 * TOP_EXACT flag means the current top depends on estimated prefix heights;
 * it remains presentation state but is not an exact absolute document y. The
 * dependency identity is captured atomically with these flags and values.
 */
typedef struct EvimViewportStateV1 {
  uint32_t struct_size;
  uint32_t flags;
  float left;
  float top;
  float maximum_left;
  float reserved;
  uint64_t document_id;
  uint64_t document_revision;
  uint64_t layout_revision;
  uint64_t configuration_generation;
  uint64_t measurement_environment_id;
  uint64_t metrics_generation;
} EvimViewportStateV1;

#define EVIM_VIEWPORT_STATE_V1_SIZE \
  ((uint32_t)sizeof(EvimViewportStateV1))

typedef struct EvimKeyInputV1 {
  uint32_t struct_size;
  uint32_t kind;
  uint32_t codepoint;
  uint32_t reserved;
} EvimKeyInputV1;

#define EVIM_KEY_INPUT_V1_SIZE ((uint32_t)sizeof(EvimKeyInputV1))

/*
 * Exact formatted-snapshot replacement target for native marked text. The
 * frontend passes its current selection, or an empty range at its insertion
 * caret, using UTF-8 byte offsets for document_revision.
 */
typedef struct EvimCompositionBeginV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_revision;
  uint64_t replacement_start;
  uint64_t replacement_end;
} EvimCompositionBeginV1;

#define EVIM_COMPOSITION_BEGIN_V1_SIZE \
  ((uint32_t)sizeof(EvimCompositionBeginV1))

/* selected_start/end are UTF-8 byte offsets relative to marked_text. */
typedef struct EvimCompositionUpdateV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_revision;
  EvimUtf8Slice marked_text;
  uint64_t selected_start;
  uint64_t selected_end;
} EvimCompositionUpdateV1;

#define EVIM_COMPOSITION_UPDATE_V1_SIZE \
  ((uint32_t)sizeof(EvimCompositionUpdateV1))

typedef struct EvimCompositionCommitV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_revision;
  EvimUtf8Slice committed_text;
} EvimCompositionCommitV1;

#define EVIM_COMPOSITION_COMMIT_V1_SIZE \
  ((uint32_t)sizeof(EvimCompositionCommitV1))

typedef struct EvimCompositionCancelV1 {
  uint32_t struct_size;
  uint32_t reserved;
  uint64_t document_revision;
} EvimCompositionCancelV1;

#define EVIM_COMPOSITION_CANCEL_V1_SIZE \
  ((uint32_t)sizeof(EvimCompositionCancelV1))

typedef struct EvimCoreOutcomeV1 {
  uint32_t struct_size;
  uint32_t command_status;
  uint32_t mode;
  uint32_t flags;
  uint64_t document_revision;
  uint64_t view_id;
  uint64_t cursor_utf8_offset;
  uint64_t layout_revision;
  uint64_t configuration_generation;
  uint64_t measurement_environment_id;
  uint64_t metrics_generation;
} EvimCoreOutcomeV1;

#define EVIM_CORE_OUTCOME_V1_SIZE ((uint32_t)sizeof(EvimCoreOutcomeV1))

uint32_t evim_core_abi_version(void);

/*
 * All strings and byte sequences are length-delimited and are never assumed
 * or written to be NUL-terminated. A null data pointer is permitted only for
 * a zero-length input or a zero-capacity length query.
 *
 * Document handles are opaque, process-local, nonzero tokens. They are never
 * reused. Calls are thread-safe. A document is checked out for one serial
 * operation without retaining a registry lock; a concurrent operation on the
 * same document returns EVIM_STATUS_DOCUMENT_BUSY instead of blocking.
 * Destroying a checked-out document also returns EVIM_STATUS_DOCUMENT_BUSY,
 * retains the handle, and must be retried after the operation returns.
 */
EvimStatus evim_document_create(const uint8_t *source,
                                uint64_t source_length,
                                const EvimDocumentOptions *options,
                                EvimDocumentHandle *out_document,
                                uint64_t *out_revision);

EvimStatus evim_document_destroy(EvimDocumentHandle document);

EvimStatus evim_document_revision(EvimDocumentHandle document,
                                  uint64_t *out_revision);

/*
 * For either copy function, pass output=NULL and output_capacity=0 to query
 * the required byte count. A nonempty value then returns
 * EVIM_STATUS_BUFFER_TOO_SMALL and always populates out_required.
 */
EvimStatus evim_document_copy_source_bytes(EvimDocumentHandle document,
                                           uint64_t expected_revision,
                                           uint8_t *output,
                                           uint64_t output_capacity,
                                           uint64_t *out_required);

EvimStatus evim_document_copy_formatted_utf8(EvimDocumentHandle document,
                                             uint64_t expected_revision,
                                             uint8_t *output,
                                             uint64_t output_capacity,
                                             uint64_t *out_required);

/* start and end are half-open byte offsets in the formatted UTF-8 snapshot. */
EvimStatus evim_document_replace_formatted_utf8(
    EvimDocumentHandle document, uint64_t expected_revision, uint64_t start,
    uint64_t end, const uint8_t *replacement, uint64_t replacement_length,
    uint64_t *out_revision);

/*
 * Core handles own a document, command/controller state, and attached views.
 * They are opaque, process-local, nonzero, and never reused. The core registry
 * lock is never held while invoking a provider callback. Reentrant access to
 * the same checked-out core returns EVIM_STATUS_CORE_BUSY. Destroying a
 * checked-out core does the same without removing it; the caller must retain
 * all provider-owned state and retry destruction after the operation returns.
 */
EvimStatus evim_core_create(const uint8_t *source, uint64_t source_length,
                            const EvimDocumentOptions *options,
                            EvimCoreHandle *out_core,
                            uint64_t *out_revision);

EvimStatus evim_core_destroy(EvimCoreHandle core);
EvimStatus evim_core_revision(EvimCoreHandle core, uint64_t *out_revision);

EvimStatus evim_core_view_add(
    EvimCoreHandle core, const EvimViewOptionsV1 *options,
    const EvimTextMeasurementProviderV1 *provider, EvimViewId *out_view,
    EvimCoreOutcomeV1 *out_outcome);
EvimStatus evim_core_view_remove(EvimCoreHandle core, EvimViewId view);
EvimStatus evim_core_view_state(EvimCoreHandle core, EvimViewId view,
                                EvimCoreOutcomeV1 *out_outcome);
EvimStatus evim_core_view_viewport_state(EvimCoreHandle core, EvimViewId view,
                                         EvimViewportStateV1 *out_state);

EvimStatus evim_core_view_send_key(EvimCoreHandle core, EvimViewId view,
                                   const EvimKeyInputV1 *input,
                                   EvimCoreOutcomeV1 *out_outcome);
EvimStatus evim_core_view_send_text(EvimCoreHandle core, EvimViewId view,
                                    const uint8_t *text, uint64_t text_length,
                                    EvimCoreOutcomeV1 *out_outcome);

/*
 * Composition requests are exact-revision operations. Begin replacement and
 * update selection ranges use UTF-8 byte offsets and must also be extended-
 * grapheme boundaries. Update and cancel never edit source. Commit applies its
 * final UTF-8 payload as one source transaction and one undo unit. On a model
 * policy failure, source stays exact and the final payload remains marked so
 * the frontend can cancel or retry under another policy.
 */
EvimStatus evim_core_view_composition_begin(
    EvimCoreHandle core, EvimViewId view,
    const EvimCompositionBeginV1 *request, EvimCoreOutcomeV1 *out_outcome);
EvimStatus evim_core_view_composition_update(
    EvimCoreHandle core, EvimViewId view,
    const EvimCompositionUpdateV1 *request, EvimCoreOutcomeV1 *out_outcome);
EvimStatus evim_core_view_composition_commit(
    EvimCoreHandle core, EvimViewId view,
    const EvimCompositionCommitV1 *request, EvimCoreOutcomeV1 *out_outcome);
EvimStatus evim_core_view_composition_cancel(
    EvimCoreHandle core, EvimViewId view,
    const EvimCompositionCancelV1 *request, EvimCoreOutcomeV1 *out_outcome);

/*
 * Horizontal-only origin changes do not shape, reflow, or replace immutable
 * layout. HAS_TOP currently returns the typed unsupported status atomically.
 */
EvimStatus evim_core_view_set_viewport_origin(
    EvimCoreHandle core, EvimViewId view,
    const EvimViewportOriginV1 *request, EvimCoreOutcomeV1 *out_outcome);

EvimStatus evim_core_view_resize(EvimCoreHandle core, EvimViewId view,
                                 float width, float height,
                                 EvimCoreOutcomeV1 *out_outcome);
EvimStatus evim_core_view_set_wrap(EvimCoreHandle core, EvimViewId view,
                                   uint32_t wrap,
                                   EvimCoreOutcomeV1 *out_outcome);

EvimStatus evim_core_copy_source_bytes(EvimCoreHandle core,
                                       uint64_t expected_revision,
                                       uint8_t *output,
                                       uint64_t output_capacity,
                                       uint64_t *out_required);
EvimStatus evim_core_copy_formatted_utf8(EvimCoreHandle core,
                                         uint64_t expected_revision,
                                         uint8_t *output,
                                         uint64_t output_capacity,
                                         uint64_t *out_required);

#ifdef __cplusplus
}
#endif

#endif /* EVIM_CORE_H */
