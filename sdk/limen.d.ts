/// <reference path="./modules.d.ts" />

import type * as Content from "limen:content";
import type * as Policies from "limen:policies";

/**
 * A value accepted at the script/host JSON boundary; import as a type or through JSDoc only.
 * @remarks The host rejects cycles, custom prototypes, accessors, non-finite numbers and
 * unsafe integers, and enforces resource limits beyond what TypeScript can express.
 */
export type JsonValue = null | boolean | number | string | JsonValue[] | JsonObject;
/** Open JSON object; omit absent fields rather than assigning `undefined`. */
export interface JsonObject {
    /** JSON-valued own properties; functions and non-JSON values cannot cross the host boundary. */
    [key: string]: JsonValue;
}

/**
 * Recursively marks object properties and array elements as read-only.
 * @typeParam T - Value exposed through an immutable host snapshot.
 */
export type DeepReadonly<T> = {
    readonly [Key in keyof T]: DeepReadonly<T[Key]>;
};

/** Public hook phases; registration methods determine which phases a scope supports. */
export type Phase =
    | "onRequest"
    | "beforeSend"
    | "afterReceive"
    | "onResponse"
    | "onUpstreamEvent"
    | "onStreamEvent"
    | "onContent"
    | "onErrorResponse"
    | "onError"
    | "onFinish";
/** Classification of the original upstream media type, before response-header edits. */
export type UpstreamContentType = "json" | "sse" | "other";

/**
 * Immutable request identity and original transport facts; unavailable facts are `null`.
 * @typeParam P - Literal phase for the current hook invocation.
 */
export interface Meta<P extends Phase = Phase> {
    /** Host-generated correlation ID shared by diagnostics and the downstream response. */
    readonly requestId: string;
    /** Phase being executed; independent of mutable response drafts. */
    readonly phase: P;
    /** Selected client-facing model alias, or `null` before routing. */
    readonly model: string | null;
    /** Selected provider ID, or `null` before routing. */
    readonly provider: string | null;
    /** Configured upstream model, independent of later `body.model` edits. */
    readonly configuredUpstreamModel: string | null;
    /** Original client streaming intent, or `null` before a valid request establishes it. */
    readonly clientStream: boolean | null;
    /** Original upstream HTTP status, or `null` before response headers arrive. */
    readonly upstreamStatus: number | null;
    /** Original upstream media classification, or `null` before response headers arrive. */
    readonly upstreamContentType: UpstreamContentType | null;
}

/** Pre-routing facts for `onRequest`; no provider, model or upstream response is selected yet. */
export interface RequestMeta extends Meta<"onRequest"> {
    /** Routing has not run. */
    readonly model: null;
    /** Routing has not run. */
    readonly provider: null;
    /** Routing has not run. */
    readonly configuredUpstreamModel: null;
    /** Original client streaming intent, captured before any hook edits. */
    readonly clientStream: boolean;
    /** No upstream request has been sent. */
    readonly upstreamStatus: null;
    /** No upstream request has been sent. */
    readonly upstreamContentType: null;
}

/**
 * Facts after the one routing decision; request edits never select a different pipeline.
 * @typeParam P - Phase executing with an established route.
 */
export interface RoutedMeta<P extends Phase> extends Meta<P> {
    /** Selected client-facing alias. */
    readonly model: string;
    /** Selected provider ID. */
    readonly provider: string;
    /** Configured upstream model before `beforeSend` edits. */
    readonly configuredUpstreamModel: string;
    /** Original client streaming intent. */
    readonly clientStream: boolean;
}

/** Routed facts before the single upstream request is sent. */
export interface BeforeSendMeta extends RoutedMeta<"beforeSend"> {
    /** Upstream headers have not arrived. */
    readonly upstreamStatus: null;
    /** Upstream headers have not arrived. */
    readonly upstreamContentType: null;
}

/**
 * Original upstream response facts, independent of `afterReceive` edits.
 * @typeParam P - Phase executing after upstream headers have arrived.
 */
export interface ReceivedMeta<P extends Phase> extends RoutedMeta<P> {
    /** Original upstream HTTP status. */
    readonly upstreamStatus: number;
    /** Original upstream media classification used by protocol processing. */
    readonly upstreamContentType: UpstreamContentType;
}

/** The gateway validates messages as objects; extensions stay ordinary JSON. */
export type RequestBody = JsonObject & {
    /** Client alias before routing; upstream model after defaults and model selection. */
    model: string;
    /** Chat messages validated as JSON objects; provider-specific fields remain open. */
    messages: JsonObject[];
    /** Requested transport; edits do not change `meta.clientStream`. */
    stream?: boolean;
    /** Optional completion budget; numeric and provider constraints remain host-validated. */
    max_completion_tokens?: number;
};

/** Function-call delta with open JSON extensions; strings may arrive in fragments. */
export type StreamFunction = JsonObject & {
    /** Function-name fragment, when supplied. */
    name?: string | null;
    /** Raw argument-string fragment; an individual delta need not be valid JSON. */
    arguments?: string | null;
};
/** Tool-call delta keyed by its nonnegative index within a choice. */
export type StreamToolCall = JsonObject & {
    /** Stable tool-call index within the source choice. */
    index: number;
    /** Tool-call ID fragment, when supplied. */
    id?: string | null;
    /** Tool type, when supplied by the upstream. */
    type?: string | null;
    /** Function-name and argument fragments. */
    function?: StreamFunction;
};
/** Open generation delta; fields may be absent on metadata-only events. */
export type StreamDelta = JsonObject & {
    /** Speaker role, commonly supplied only in the first generation delta. */
    role?: string;
    /** Text fragment, structured content blocks, or an explicit null. */
    content?: string | JsonObject[] | null;
    /** Refusal-text fragment. */
    refusal?: string | null;
    /** Provider reasoning-text fragment. */
    reasoning?: string | null;
    /** Alternate provider reasoning-text field. */
    reasoning_content?: string | null;
    /** Structured reasoning details preserved as open JSON objects. */
    reasoning_details?: JsonObject[] | null;
    /** Audio payload or trailing audio metadata. */
    audio?: JsonObject | null;
    /** Tool-call fragments; each entry identifies its own tool index. */
    tool_calls?: StreamToolCall[];
};
/** One source choice; a finish reason does not by itself prove stream completion. */
export type StreamChoice = JsonObject & {
    /** Nonnegative source choice index. */
    index: number;
    /** Generation delta; may be absent on non-generation events. */
    delta?: StreamDelta;
    /** Upstream completion reason; null or absent while generation remains open. */
    finish_reason?: string | null;
};
/** Open Chat data, including annotation and metadata events without generation choices. */
export type StreamEvent = JsonObject & {
    /** Upstream response identifier, when present. */
    id?: string;
    /** Upstream object discriminator, when present. */
    object?: string;
    /** Upstream creation timestamp, conventionally Unix seconds. */
    created?: number;
    /** Model field exposed to raw hooks; the host applies downstream alias handling. */
    model?: string;
    /** Choice data; usage-only and metadata events may omit it. */
    choices?: StreamChoice[];
    /** Open usage payload; authoritative statistics are sampled before script edits. */
    usage?: JsonValue;
};

/** Immutable source SSE envelope; JSON data and protocol control markers are separate. */
export interface SseEnvelope {
    /** Source event name; unnamed events use the SSE default `message`. */
    readonly event: string;
    /** Source frame's ID, or `null` when absent; not an application request ID. */
    readonly id: string | null;
    /** Source retry field as text, or `null` when absent; never enables gateway retries. */
    readonly retry: string | null;
}

/** A frozen API with mutable contents; names are case-insensitive at runtime. */
export interface HeadersDraft {
    /**
     * Reads all values without exposing the mutable backing store.
     * @param name - Case-insensitive header name.
     * @returns A fresh array, empty when absent; editing it does not change the draft.
     */
    readonly getAll: (name: string) => string[];
    /**
     * Adds a value while preserving existing values.
     * @param name - Business header name; transport headers remain host-owned.
     * @param value - Header value validated by the host.
     * @throws If the header name or value is invalid or host-controlled.
     */
    readonly append: (name: string, value: string) => void;
    /**
     * Replaces all values for a business header.
     * @param name - Case-insensitive header name; use `contentType` in error-response hooks.
     * @param value - Header value validated by the host.
     * @throws If the header name or value is invalid or host-controlled.
     */
    readonly set: (name: string, value: string) => void;
    /**
     * Removes all values for a business header.
     * @param name - Case-insensitive header name.
     * @throws If the header name is invalid or host-controlled.
     */
    readonly delete: (name: string) => void;
}

/** A read-only view of explicitly allowlisted client headers, separate from outbound headers. */
export interface ReadonlyHeaders {
    /**
     * Reads explicitly exposed client business-header values.
     * @param name - Case-insensitive header name.
     * @returns A fresh array, empty for missing, unexposed or Connection-excluded fields.
     */
    readonly getAll: (name: string) => string[];
}
/** Immutable, allowlisted client input; no authentication or Cookie access. */
export interface ClientContext {
    /** Read-only business headers; values are never forwarded automatically. */
    readonly headers: ReadonlyHeaders;
}
/** HTTP statuses supported by business-policy rejection. */
export type RejectionStatus = 400 | 403 | 422;

/** Bounded, transactional recorder for names approved in server observability configuration. */
export interface ObservationRecorder {
    /**
     * Queues an observation for commit with the current hook or content batch.
     * @param name - Registered event name; no arbitrary payload or sensitive data.
     * @throws If the name is invalid or unregistered; capacity overflow only drops observations.
     */
    readonly record: (name: string) => void;
}
/** Shared observation capability, available in every hook phase. */
interface ObservationContext {
    /** Candidate observations commit only when the invocation remains valid. */
    readonly observe: ObservationRecorder;
}

/**
 * Pre-routing request draft after authentication and initial body validation.
 * @typeParam State - Per-request JSON state; starts as `{}` regardless of this static type.
 */
export interface RequestContext<State extends JsonObject = JsonObject> extends ObservationContext {
    /** Read-only allowlisted client input. */
    readonly client: ClientContext;
    /**
     * Records one business rejection; does not throw or stop callback execution.
     * @param status - Allowed rejection status; defaults to 400.
     * @throws On repeated rejection or an unsupported status.
     * @remarks Rejection and valid state commit only if the hook succeeds; return to stop local work.
     */
    readonly reject: (status?: RejectionStatus) => void;
    /** Original client intent with no selected route. */
    readonly meta: RequestMeta;
    /** The reference is fixed; its JSON fields can change. Starts as an empty object. */
    readonly state: State;
    /** Mutable request; changing `model` here affects the subsequent routing decision. */
    body: RequestBody;
}
/**
 * Routed request draft after shallow defaults merge, before the single upstream send.
 * @typeParam State - Mutable JSON state shared by this request's successful hooks.
 */
export interface BeforeSendContext<
    State extends JsonObject = JsonObject,
> extends ObservationContext {
    /** Read-only allowlisted client input. */
    readonly client: ClientContext;
    /**
     * Records one transactional rejection without stopping the callback.
     * @param status - Allowed rejection status; defaults to 400.
     * @throws On repeated rejection or an unsupported status.
     */
    readonly reject: (status?: RejectionStatus) => void;
    /** Selected route and original client streaming intent; no upstream response yet. */
    readonly meta: BeforeSendMeta;
    /** Fixed reference to the mutable per-request JSON draft. */
    readonly state: State;
    /** Outbound request; `model` or `stream` edits do not reroute or change original client intent. */
    body: RequestBody;
    /** Complete outbound HTTP(S) URL; no userinfo or fragment, and edits do not reroute. */
    url: string;
    /** Outbound business-header draft, separate from client headers. */
    readonly headers: HeadersDraft;
}
/**
 * Response-header draft received before upstream body processing.
 * @typeParam State - Mutable per-request JSON state.
 */
export interface AfterReceiveContext<
    State extends JsonObject = JsonObject,
> extends ObservationContext {
    /** Read-only allowlisted client input. */
    readonly client: ClientContext;
    /** Original upstream status and media facts, unaffected by draft edits. */
    readonly meta: ReceivedMeta<"afterReceive">;
    /** Fixed reference to the mutable per-request JSON draft. */
    readonly state: State;
    /** Candidate response status; cannot reinterpret an upstream failure as success. */
    status: number;
    /** Response business-header draft; transport and media ownership remain with the host. */
    readonly headers: HeadersDraft;
}
/**
 * Complete JSON response draft, including an SSE aggregate when the client requested JSON.
 * @typeParam State - Mutable per-request JSON state.
 */
export interface ResponseContext<State extends JsonObject = JsonObject> extends ObservationContext {
    /** Read-only allowlisted client input. */
    readonly client: ClientContext;
    /** Original upstream facts; SSE aggregates still report upstream media as `sse`. */
    readonly meta: ReceivedMeta<"onResponse">;
    /** Fixed reference to the mutable per-request JSON draft. */
    readonly state: State;
    /** Upstream JSON and its extensions are not restricted to a fixed response schema. */
    body: JsonObject;
}
/**
 * Raw Chat data-event draft; protocol control markers such as `[DONE]` remain host-owned.
 * @typeParam P - Raw upstream or downstream event phase.
 * @typeParam State - Mutable per-request JSON state.
 */
export interface EventContext<
    P extends "onUpstreamEvent" | "onStreamEvent",
    State extends JsonObject = JsonObject,
> extends ObservationContext {
    /** Read-only allowlisted client input. */
    readonly client: ClientContext;
    /** Original upstream response facts. */
    readonly meta: ReceivedMeta<P>;
    /** Fixed reference to the mutable per-request JSON draft. */
    readonly state: State;
    /** Mutable open Chat chunk, forwarded after successful validation unless replaced. */
    event: StreamEvent;
    /** Original SSE envelope; null for events synthesized from a complete JSON response. */
    readonly sse: SseEnvelope | null;
    /**
     * Replaces this source event at most once per callback.
     * @param events - Ordered output; `[]` drops the event, `[a, b]` expands it for later steps.
     * @throws On repeated replacement; the host validates protocol, expansion and output budgets.
     */
    readonly replaceEvents: (events: StreamEvent[]) => void;
}
/**
 * Real upstream SSE data before aggregation or downstream processing; excludes comments and DONE.
 * @typeParam State - Mutable per-request JSON state.
 */
export type UpstreamEventContext<State extends JsonObject = JsonObject> = EventContext<
    "onUpstreamEvent",
    State
>;
/**
 * Outgoing Chat chunk, from real SSE or synthesized JSON, before downstream emission.
 * @typeParam State - Mutable per-request JSON state.
 */
export type StreamEventContext<State extends JsonObject = JsonObject> = EventContext<
    "onStreamEvent",
    State
>;

/**
 * Semantic string-leaf lifecycle shared by JSON and SSE; tool text contains raw arguments/input.
 * @remarks Choice indices are nonnegative; part IDs are opaque and stable within a choice.
 * Parts close before their choice; successful `response_end` never flushes a truncated stream.
 */
export type ContentEvent =
    | {
          /** Opens one non-tool string leaf. */
          type: "part_start";
          /** Nonnegative choice index. */
          choice: number;
          /** Stable part identifier; do not parse its format. */
          part: string;
          /** Semantic category of the string leaf. */
          kind: "text" | "reasoning" | "refusal";
      }
    | {
          /** Opens one tool-argument or custom-tool-input leaf. */
          type: "part_start";
          /** Nonnegative choice index. */
          choice: number;
          /** Stable part identifier within the choice. */
          part: string;
          /** Discriminator requiring tool metadata. */
          kind: "tool";
          /** Initial tool identity; later deltas may supply ID or name fragments. */
          tool: {
              /** Nonnegative tool index within the choice. */
              index: number;
              /** Function, custom tool or legacy function-call encoding. */
              type: "function" | "custom" | "legacy";
              /** Initial tool-call ID, when available. */
              id?: string;
              /** Initial tool name, when available. */
              name?: string;
          };
      }
    | {
          /** Appends text or tool metadata to an open part. */
          type: "part_delta";
          /** Choice owning the open part. */
          choice: number;
          /** Previously opened part identifier. */
          part: string;
          /** String fragment; tool arguments need not be complete JSON at this boundary. */
          text: string;
          /** Tool-only metadata, forwarded even when the text fragment is empty. */
          tool?: {
              /** Tool-call ID fragment. */
              id?: string;
              /** Tool-name fragment. */
              name?: string;
              /** Explicit tool encoding, when first established by this delta. */
              type?: "function" | "custom";
          };
      }
    | {
          /** Closes a part; emit any retained tail before forwarding this event. */
          type: "part_end";
          /** Choice owning the part. */
          choice: number;
          /** Open part being closed. */
          part: string;
          /** Upstream completion reason, if known at the part boundary. */
          finishReason?: string;
      }
    | {
          /** Closes generation for a choice after all its parts end. */
          type: "choice_end";
          /** Choice that cannot produce further generation content. */
          choice: number;
          /** Required completion reason. */
          finishReason: string;
      }
    | {
          /** Successful source completion; permits state/observations, not new content parts. */
          type: "response_end";
      };

/**
 * Semantic event draft; one step's callbacks for a source body commit as a single batch.
 * @typeParam State - Per-request JSON state, staged across callbacks within the batch.
 */
export interface ContentContext<State extends JsonObject = JsonObject> extends ObservationContext {
    /** Read-only allowlisted client input. */
    readonly client: ClientContext;
    /** Original response facts; semantic policy should branch on events rather than transport. */
    readonly meta: ReceivedMeta<"onContent">;
    /** Fixed reference; the entire source batch rolls back on validation or rendering failure. */
    readonly state: State;
    /** Mutable semantic event, subject to part/choice lifecycle validation. */
    event: ContentEvent;
    /** Source envelope only; null for complete JSON. Never use it to select business semantics. */
    readonly sse: SseEnvelope | null;
    /**
     * Replaces this semantic event at most once per callback.
     * @param events - Ordered events for remaining steps; `[]` drops this event.
     * @throws On repeated replacement; output must satisfy lifecycle and shared output limits.
     */
    readonly replaceEvents: (events: ContentEvent[]) => void;
}

/**
 * Error-response draft before headers are sent; executes provider, model, then global hooks.
 * @typeParam State - Mutable per-request JSON state; commits with the entire valid error draft.
 */
export interface ErrorResponseContext<
    State extends JsonObject = JsonObject,
> extends ObservationContext {
    /** Read-only allowlisted client input. */
    readonly client: ClientContext;
    /** Original request facts; routing or upstream receipt may not have happened. */
    readonly meta: Meta<"onErrorResponse">;
    /** Fixed reference to the mutable per-request JSON draft. */
    readonly state: State;
    /** Original failure classification, unaffected by response edits. */
    readonly error: DeepReadonly<ObserverError>;
    /** Error status: 3xx–5xx for upstream HTTP failures, 4xx–5xx for gateway failures. */
    status: number;
    /** Response business-header draft; media type uses the separate `contentType` field. */
    readonly headers: HeadersDraft;
    /** UTF-8 error text. null preserves the current bytes, including non-text bodies. */
    body: string | null;
    /** null removes Content-Type. Rust validates non-null header values. */
    contentType: string | null;
}

/** Sanitized failure categories exposed to script observers; raw exceptions are not included. */
export type ErrorCode =
    | "invalid_request"
    | "unauthorized"
    | "unknown_model"
    | "payload_too_large"
    | "busy"
    | "deadline_exceeded"
    | "cancelled"
    | "request_rejected"
    | "hook_failed"
    | "upstream_failed"
    | "upstream_protocol"
    | "internal_error"
    | "unavailable";
/** Controlled failure snapshot; response adaptation cannot change its original classification. */
export interface ObserverError {
    /** Stable gateway failure category. */
    readonly code: ErrorCode;
    /** HTTP failure status captured for this observer invocation. */
    readonly status: number;
    /** Fixed, sanitized message; never the raw upstream or JavaScript exception. */
    readonly message: string;
    /** Request correlation ID; uses the error envelope's snake_case spelling. */
    readonly request_id: string;
    /** May be a gateway operation such as authentication, not only a hook phase. */
    readonly phase: string;
    /** Pipeline step name, or `null` for failures outside a step. */
    readonly step: string | null;
}
/** Request statistics available to terminal observers; unavailable milestones remain `null`. */
export interface ObserverStats {
    /** Original upstream usage sampled before script body edits; frozen at terminal outcome. */
    readonly usage: UsageStats;
    /** Millisecond durations and milestones frozen before cleanup hooks execute. */
    readonly timings: RequestTimings;
    /** Client request body bytes read, excluding HTTP headers. */
    readonly inputBytes: number;
    /** Encoded upstream request body bytes, excluding HTTP headers. */
    readonly outboundBytes: number;
    /** Upstream body bytes read, including partial reads before failure. */
    readonly upstreamBytes: number;
    /** Downstream body bytes accounted by the gateway; not proof of client receipt. */
    readonly outputBytes: number;
    /** Milliseconds from request start until upstream headers arrive. */
    readonly upstreamHeadersMs: number | null;
    /** Milliseconds since request start, sampled separately for each observer invocation. */
    readonly elapsedMs: number;
    /** Actual outbound model after `beforeSend`, or `null` before it is established. */
    readonly finalUpstreamModel: string | null;
    /** Sanitized origin (scheme, host and optional port), without path or query. */
    readonly target: string | null;
}

/**
 * Native cumulative usage; counts are nonnegative safe integers or `null`, never inferred zeros.
 * @remarks Latest nondecreasing fields win; chunks are not summed. Invalid/conflicting reports
 * suppress effective counts. Reported usage does not prove upstream or downstream success.
 */
export interface UsageStats {
    /** Original body protocol, or `null` before body processing starts. */
    readonly source: "json" | "sse" | null;
    /** Missing/partial/complete core counts, invalid fields, or regressing cumulative counts. */
    readonly quality: "missing" | "partial" | "reported" | "invalid" | "conflicting";
    /** Original JSON completed, or SSE passed both DONE and choice-completion checks. */
    readonly upstreamCompleted: boolean;
    /** Reported prompt/input token count; missing or suppressed counts are `null`. */
    readonly inputTokens: number | null;
    /** Reported completion/output token count. */
    readonly outputTokens: number | null;
    /** Reported total count; not calculated from input and output. */
    readonly totalTokens: number | null;
    /** Cached-input subset from upstream details; not added to total again. */
    readonly cachedInputTokens: number | null;
    /** Reasoning-output subset from upstream details; not added to total again. */
    readonly reasoningOutputTokens: number | null;
}
/** Aggregate of available measurements; zero samples differs from a measured zero duration. */
export interface TimingSample {
    /** Sum of measured durations in milliseconds, preserving fractional precision. */
    readonly totalMs: number;
    /** Number of available measurements included in the sum. */
    readonly samples: number;
}
/** JavaScript hook timings for one non-cleanup phase; includes interrupted host waits. */
export interface HookTimingSummary {
    /** Number of started host hook calls. */
    readonly calls: number;
    /** Runtime-reported failures; unreported host interruptions have no failure report. */
    readonly failedCalls: number;
    /** Runtime-reported cancellation/deadline calls plus interrupted waits without a report. */
    readonly interruptedCalls: number;
    /** Runtime reports received; may be lower than calls when host waiting is interrupted. */
    readonly reportedCalls: number;
    /** Total host-observed wall time, including time already waited before interruption. */
    readonly wallMs: number;
    /** Time waiting in the VM command queue. */
    readonly queue: TimingSample;
    /** Initial synchronous JavaScript invocation time. */
    readonly initialCall: TimingSample;
    /** Promise wait through terminal observation; includes shared scheduling, not just CPU work. */
    readonly pending: TimingSample;
    /** Host-to-JavaScript context construction and conversion time. */
    readonly inputBridge: TimingSample;
    /** JavaScript-to-host output projection and conversion time. */
    readonly outputBridge: TimingSample;
}
/** Processing phases included in frozen request timings; cleanup is measured separately. */
export type RequestHookPhase = Exclude<Phase, "onErrorResponse" | "onError" | "onFinish">;
/** Native policy execution timings, separate from JavaScript/VM measurements. */
export interface NativeTimingSummary {
    /** Number of native stage invocations. */
    readonly calls: number;
    /** Failed invocations, including interrupted calls. */
    readonly failedCalls: number;
    /** Invocations interrupted by cancellation or deadline. */
    readonly interruptedCalls: number;
    /** Total host-observed execution time in milliseconds. */
    readonly wallMs: number;
}
/** Millisecond timings frozen at terminal outcome; absent stages are `null` or omitted map keys. */
export interface RequestTimings {
    /** Request start to terminal outcome, excluding subsequent error writes and cleanup hooks. */
    readonly requestMs: number | null;
    /** Initial request body reading, parsing and validation. */
    readonly requestDecodeMs: number | null;
    /** Upstream send to response headers; unlike `upstreamHeadersMs`, excludes earlier request work. */
    readonly upstreamHeadersWaitMs: number | null;
    /** Upstream send to first nonempty body bytes. */
    readonly upstreamFirstByteMs: number | null;
    /** Upstream send to first valid Chat chunk; `null` for JSON upstreams. */
    readonly upstreamFirstChunkMs: number | null;
    /** Request start to first model event entering the bounded downstream body. */
    readonly downstreamFirstEventMs: number | null;
    /** Cumulative upstream read waits, excluding policy work and downstream waits. */
    readonly upstreamReadWaitMs: number | null;
    /** Cumulative waits for downstream output-channel capacity. */
    readonly downstreamQueueWaitMs: number | null;
    /** SSE body drain-acknowledgement wait; absent on JSON paths. */
    readonly downstreamDrainMs: number | null;
    /** JavaScript hook summaries, present only for phases that executed. */
    readonly hooks: Readonly<Partial<Record<RequestHookPhase, HookTimingSummary>>>;
    /** Native policy summaries, present only for phases that executed. */
    readonly native: Readonly<Partial<Record<RequestHookPhase, NativeTimingSummary>>>;
}

/**
 * Terminal observation snapshot; cannot alter the outcome or request state.
 * @typeParam P - Error or finish observation phase.
 * @typeParam State - Per-request JSON state exposed recursively read-only.
 */
interface ObserverContext<
    P extends "onError" | "onFinish",
    State extends JsonObject,
> extends ObservationContext {
    /** Read-only allowlisted client input. */
    readonly client: ClientContext;
    /** Request facts known at terminal outcome; early failures may lack routing or receipt. */
    readonly meta: Meta<P>;
    /** Immutable snapshot of the last committed per-request state. */
    readonly state: DeepReadonly<State>;
    /** Read-only request snapshot with frozen usage/timings and observer-time `elapsedMs`. */
    readonly stats: ObserverStats;
}
/**
 * Failure/cancellation observer; executes before `onFinish` within the shared cleanup budget.
 * @typeParam State - Per-request JSON state, recursively read-only in this phase.
 */
export type ErrorContext<State extends JsonObject = JsonObject> = ObserverContext<
    "onError",
    State
> & {
    /** Failure or client cancellation; deadlines are failures. */
    readonly outcome: "failed" | "cancelled";
    /** Sanitized terminal failure. */
    readonly error: ObserverError;
};
/**
 * Final observer for every outcome when the VM and cleanup budget remain available.
 * @typeParam State - Per-request JSON state, recursively read-only in this phase.
 */
export type FinishContext<State extends JsonObject = JsonObject> = ObserverContext<
    "onFinish",
    State
> &
    (
        | {
              /** Gateway processing completed successfully. */
              readonly outcome: "success";
              /** Success has no error field; narrow on `outcome` before reading failures. */
              readonly error?: never;
          }
        | {
              /** Failed processing or client cancellation. */
              readonly outcome: "failed" | "cancelled";
              /** Sanitized terminal failure. */
              readonly error: ObserverError;
          }
    );

/**
 * Mutates a private context draft; successful validation commits it while the call remains valid.
 * @typeParam Context - Phase-specific capabilities and editable fields.
 * @param context - Invocation-local draft; do not retain it or store request data in shared closures.
 * @returns Nothing, or a native Promise resolving to `undefined`; replacement objects are invalid.
 * @remarks Throws, rejection, invalid output or cancellation discard this invocation's draft.
 * Earlier commits and closure side effects are not rolled back; cancelled Promises may keep running.
 */
export type Hook<Context> = (context: Context) => void | Promise<void>;

/**
 * Phase callbacks accepted by a JavaScript pipeline step.
 * @typeParam State - Per-request JSON state.
 */
interface PipelineHooks<State extends JsonObject> {
    /** Request adaptation after defaults and routing, before the upstream send. */
    beforeSend: Hook<BeforeSendContext<State>>;
    /** Response-header adaptation before reading the upstream body. */
    afterReceive: Hook<AfterReceiveContext<State>>;
    /** Complete JSON adaptation, including SSE aggregates. */
    onResponse: Hook<ResponseContext<State>>;
    /** Real upstream SSE Chat data adaptation before semantic projection or aggregation. */
    onUpstreamEvent: Hook<UpstreamEventContext<State>>;
    /** Final outgoing SSE Chat chunk adaptation, including synthesized chunks. */
    onStreamEvent: Hook<StreamEventContext<State>>;
    /** Semantic text/reasoning/refusal/tool transformation across all four transport paths. */
    onContent: Hook<ContentContext<State>>;
    /** Error adaptation before a response starts; preserves failure classification. */
    onErrorResponse: Hook<ErrorResponseContext<State>>;
}
/**
 * Requires at least one property while leaving the others optional.
 * @typeParam T - Property set.
 */
type AtLeastOne<T> = Partial<T> & { [Key in keyof T]: Pick<T, Key> }[keyof T];
/**
 * JavaScript step with at least one hook; native descriptors cannot be mixed with callbacks.
 * @typeParam State - Per-request JSON state shared by this step's hooks.
 */
export type ScriptStep<State extends JsonObject = JsonObject> = {
    /** Nonempty step name, unique within its provider or model pipeline. */
    name: string;
    /** Native policy descriptors are forbidden on JavaScript steps. */
    builtin?: never;
} & AtLeastOne<PipelineHooks<State>>;

/** Discriminated native policy options; prefer the `limen:policies`/`limen:content` factories. */
export type BuiltinPolicy =
    | ({
          /** Native completion-budget enforcement in `beforeSend`. */
          kind: "capCompletionTokens";
      } & Omit<Policies.CapCompletionTokensOptions, "name">)
    | ({
          /** Native request-field deletion in `beforeSend`. */
          kind: "dropRequestFields";
      } & Omit<Policies.DropRequestFieldsOptions, "name">)
    | ({
          /** Native explicit client-header forwarding in `beforeSend`. */
          kind: "forwardClientHeader";
      } & Omit<Policies.ForwardClientHeaderOptions, "name">)
    | ({
          /** Native literal replacement in `onContent`. */
          kind: "replaceText";
      } & Omit<Content.ReplaceTextOptions, "name">)
    | ({
          /** Native tool transport in `beforeSend` and `onContent`. */
          kind: "syntheticTextTool";
      } & Omit<Content.SyntheticTextToolOptions, "name">);
/** Native policy state remains owned by Rust and is absent from ctx.state. */
export type BuiltinStep = {
    /** Step name, unique within the receiving pipeline. */
    readonly name: string;
    /** Immutable policy kind/options; callbacks are forbidden on the same step. */
    readonly builtin: Readonly<BuiltinPolicy>;
} & { [Key in keyof PipelineHooks<JsonObject>]?: never };
/**
 * One explicitly ordered JavaScript or native step accepted by a registration scope.
 * @typeParam State - Per-request JSON state used by JavaScript hooks only.
 */
export type PipelineStep<State extends JsonObject = JsonObject> = ScriptStep<State> | BuiltinStep;

/** Shallow defaults: model overrides provider, then client values win; no recursive merge. */
export type Defaults = JsonObject & {
    /** Model selection is controlled by explicit routing, not defaults. */
    model?: never;
    /** Client history cannot be supplied through defaults. */
    messages?: never;
    /** Transport intent cannot be supplied through defaults. */
    stream?: never;
};
/** Provider-level upstream connection and request defaults; validated once at startup. */
export interface ProviderOptions {
    /** Complete HTTP(S) Chat Completions URL; no userinfo or fragment. */
    endpoint: string;
    /** Separate upstream authentication; client credentials are never forwarded by default. */
    auth?: {
        /** Nonempty upstream Bearer token; cannot coexist with an Authorization entry in headers. */
        bearer: string;
    };
    /** Static outbound headers; names/values and transport restrictions are host-validated. */
    headers?: Record<string, string>;
    /** Provider fallback request fields, overridden by model defaults and client values. */
    defaults?: Defaults;
}
/**
 * Model settings; provider ownership comes from the scope used to register it.
 * @typeParam Options - Factory-specific static fields; raw providers accept no typed options.
 */
export interface ModelOptions<Options = never> {
    /** Nonempty upstream model name substituted for the client-facing alias after routing. */
    upstream: string;
    /** Shallow defaults overriding the provider's defaults; client values still win. */
    defaults?: Defaults;
    /** Typed static provider options, lowered into shallow defaults by a provider factory. */
    options?: Options;
}
/**
 * Synchronous gateway assembly callback invoked once at startup.
 * @typeParam State - Per-request JSON state type; runtime initialization is always `{}`.
 * @param gateway - Registration scope, closed when this callback exits.
 * @returns `undefined`; async functions, Promises, scopes and configuration objects are invalid.
 */
export type GatewaySetup<State extends JsonObject = JsonObject> = (
    gateway: GatewayScope<State>,
) => undefined;
/**
 * Synchronous callback registering at least one phase on a named JavaScript step.
 * @typeParam State - Per-request JSON state type shared with the enclosing scope.
 * @param step - Registration scope, closed as soon as this callback exits.
 * @returns `undefined`; duplicate phases and empty steps fail startup.
 */
export type StepSetup<State extends JsonObject = JsonObject> = (
    step: StepScope<State>,
) => undefined;
/**
 * Configuration entry factory returned by `defineGateway`; invoked once by the host at startup.
 * @returns The host-captured registration result; scripts should default-export the factory.
 */
export type GatewayFactory = () => unknown;
/**
 * Hook registration for one JavaScript step; each phase may be registered only once.
 * @typeParam State - Per-request JSON state type shared by registered hooks.
 */
export interface StepScope<State extends JsonObject = JsonObject> {
    /**
     * Registers outbound request adaptation after defaults; provider steps precede model steps.
     * @param hook - Callback with mutable body, URL, headers and state; may reject the request.
     * @returns `undefined` after registration.
     */
    readonly beforeSend: (hook: Hook<BeforeSendContext<State>>) => undefined;
    /**
     * Registers adaptation after upstream headers arrive, before body reads.
     * @param hook - Callback with mutable status, business headers and state.
     * @returns `undefined` after registration.
     */
    readonly afterReceive: (hook: Hook<AfterReceiveContext<State>>) => undefined;
    /**
     * Registers complete JSON adaptation, including a completed SSE aggregate.
     * @param hook - Callback with mutable open response body and state.
     * @returns `undefined` after registration.
     */
    readonly onResponse: (hook: Hook<ResponseContext<State>>) => undefined;
    /**
     * Registers semantic transformation across JSON/SSE transport combinations.
     * @param hook - Event callback; each source body's callbacks share one transactional batch.
     * @returns `undefined` after registration.
     */
    readonly onContent: (hook: Hook<ContentContext<State>>) => undefined;
    /**
     * Registers raw upstream SSE data adaptation before projection or aggregation.
     * @param hook - Callback with mutable Chat chunk, event replacement and state capabilities.
     * @returns `undefined` after registration.
     */
    readonly onUpstreamEvent: (hook: Hook<UpstreamEventContext<State>>) => undefined;
    /**
     * Registers outgoing SSE chunk adaptation, including chunks synthesized from JSON.
     * @param hook - Callback with mutable Chat chunk, event replacement and state capabilities.
     * @returns `undefined` after registration.
     */
    readonly onStreamEvent: (hook: Hook<StreamEventContext<State>>) => undefined;
    /**
     * Registers failure-response adaptation; skipped once downstream headers have started.
     * @param hook - Callback with mutable error response and state; cannot turn failure into success.
     * @returns `undefined` after registration.
     */
    readonly onErrorResponse: (hook: Hook<ErrorResponseContext<State>>) => undefined;
}
/**
 * Ordered pipeline registration, valid until the enclosing gateway setup exits.
 * @typeParam State - Per-request JSON state type for JavaScript steps.
 */
export interface ModelScope<State extends JsonObject = JsonObject> {
    /**
     * Appends a factory-produced or explicitly constructed pipeline step.
     * @param step - Named JavaScript or native step; names are unique across `use` and `step` here.
     * @returns `undefined` after registration.
     * @throws On duplicate names or a closed scope; the host validates the descriptor at startup.
     */
    readonly use: (step: PipelineStep<State>) => undefined;
    /**
     * Appends a JavaScript step assembled by a synchronous callback.
     * @param name - Nonempty name unique within this pipeline, shared with `use` registrations.
     * @param setup - Registers at least one phase; its step scope closes immediately on return.
     * @returns `undefined` after registration.
     * @throws On closed scope, duplicates, empty steps or invalid setup/return value.
     */
    readonly step: (name: string, setup: StepSetup<State>) => undefined;
}
/**
 * Provider pipeline registration and creation of models owned by that provider.
 * @typeParam State - Per-request JSON state type shared with the gateway setup.
 * @typeParam Options - Factory-specific static model options; unavailable on raw providers.
 */
export interface ProviderScope<
    State extends JsonObject = JsonObject,
    Options = never,
> extends ModelScope<State> {
    /**
     * Declares a client-facing model alias in this provider.
     * @param alias - Nonempty, globally unique model name exposed in the explicit model list.
     * @param options - Upstream model name and optional model-level defaults.
     * @returns A model scope for registering steps after the provider pipeline.
     * @throws On duplicate alias or closed scope; the host validates options at startup.
     */
    readonly model: (alias: string, options: ModelOptions<Options>) => ModelScope<State>;
}
/**
 * Startup-only provider/global-hook registration; each global hook may be registered once.
 * @typeParam State - Per-request JSON state type shared by all registered hooks.
 */
export interface GatewayScope<State extends JsonObject = JsonObject> {
    /**
     * Declares an upstream provider; all its models are created through the returned scope.
     * @param id - Nonempty provider identifier unique within this gateway.
     * @param options - Endpoint, separate upstream credentials, headers and request defaults.
     * @returns A provider scope, valid until gateway setup exits.
     * @throws On duplicate ID or closed scope; the host validates options at startup.
     */
    readonly provider: (id: string, options: ProviderOptions) => ProviderScope<State>;
    /**
     * Registers the global request callback before the single routing decision.
     * @param hook - Callback with mutable client request/state; may reject the request.
     * @returns `undefined` after registration.
     */
    readonly onRequest: (hook: Hook<RequestContext<State>>) => undefined;
    /**
     * Registers global error-response adaptation after any routed provider/model adapters.
     * @param hook - Runs only before response start, within the shared cleanup budget.
     * @returns `undefined` after registration.
     */
    readonly onErrorResponse: (hook: Hook<ErrorResponseContext<State>>) => undefined;
    /**
     * Registers read-only failure/cancellation observation before `onFinish`.
     * @param hook - Observer that cannot change state, response or terminal outcome.
     * @returns `undefined` after registration.
     */
    readonly onError: (hook: Hook<ErrorContext<State>>) => undefined;
    /**
     * Registers read-only terminal observation for success, failure and cancellation.
     * @param hook - Observer subject to VM availability and the remaining cleanup budget.
     * @returns `undefined` after registration.
     */
    readonly onFinish: (hook: Hook<FinishContext<State>>) => undefined;
}
