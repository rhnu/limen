/// <reference path="./modules.d.ts" />

/**
 * Optional editor/static-checking contract for QuickJS configuration modules.
 * Import these types in JSDoc; they provide no JavaScript runtime module.
 * Rust remains authoritative for JSON numbers, cycles, prototypes, resource limits,
 * URL/header validity, routing references and Chat/SSE protocol semantics.
 */
export type JsonValue = null | boolean | number | string | JsonValue[] | JsonObject;
export interface JsonObject {
    [key: string]: JsonValue;
}

export type DeepReadonly<T> = {
    readonly [Key in keyof T]: DeepReadonly<T[Key]>;
};

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
export type UpstreamContentType = "json" | "sse" | "other";

/** Null values mean that routing or receipt of upstream headers has not happened. */
export interface Meta<P extends Phase = Phase> {
    readonly requestId: string;
    readonly phase: P;
    readonly model: string | null;
    readonly provider: string | null;
    readonly configuredUpstreamModel: string | null;
    readonly clientStream: boolean | null;
    readonly upstreamStatus: number | null;
    readonly upstreamContentType: UpstreamContentType | null;
}

export interface RequestMeta extends Meta<"onRequest"> {
    readonly model: null;
    readonly provider: null;
    readonly configuredUpstreamModel: null;
    readonly clientStream: boolean;
    readonly upstreamStatus: null;
    readonly upstreamContentType: null;
}

export interface RoutedMeta<P extends Phase> extends Meta<P> {
    readonly model: string;
    readonly provider: string;
    readonly configuredUpstreamModel: string;
    readonly clientStream: boolean;
}

export interface BeforeSendMeta extends RoutedMeta<"beforeSend"> {
    readonly upstreamStatus: null;
    readonly upstreamContentType: null;
}

/** These are the original upstream facts, independent of afterReceive edits. */
export interface ReceivedMeta<P extends Phase> extends RoutedMeta<P> {
    readonly upstreamStatus: number;
    readonly upstreamContentType: UpstreamContentType;
}

/** The gateway validates messages as objects; extensions stay ordinary JSON. */
export type RequestBody = JsonObject & {
    model: string;
    messages: JsonObject[];
    stream?: boolean;
    max_completion_tokens?: number;
};

/** Known SSE fields plus JSON extensions, matching the gateway's chunk validator. */
export type StreamFunction = JsonObject & {
    name?: string | null;
    arguments?: string | null;
};
export type StreamToolCall = JsonObject & {
    index: number;
    id?: string | null;
    type?: string | null;
    function?: StreamFunction;
};
export type StreamDelta = JsonObject & {
    role?: string;
    content?: string | JsonObject[] | null;
    refusal?: string | null;
    reasoning?: string | null;
    reasoning_content?: string | null;
    reasoning_details?: JsonObject[] | null;
    audio?: JsonObject | null;
    tool_calls?: StreamToolCall[];
};
export type StreamChoice = JsonObject & {
    index: number;
    delta?: StreamDelta;
    finish_reason?: string | null;
};
/** Open Chat data, including annotation and metadata events without generation choices. */
export type StreamEvent = JsonObject & {
    id?: string;
    object?: string;
    created?: number;
    model?: string;
    choices?: StreamChoice[];
    usage?: JsonValue;
};

/** A frozen API with mutable contents; names are case-insensitive at runtime. */
export interface HeadersDraft {
    /** Returns a fresh array; editing it does not change the header draft. */
    readonly getAll: (name: string) => string[];
    /** Transport headers (including Content-Type) remain under Rust ownership. */
    readonly append: (name: string, value: string) => void;
    readonly set: (name: string, value: string) => void;
    readonly delete: (name: string) => void;
}

/** A read-only view of explicitly allowlisted client headers, separate from outbound headers. */
export interface ReadonlyHeaders {
    /** Names are case-insensitive. A missing or unexposed field returns a fresh empty array. */
    readonly getAll: (name: string) => string[];
}
export interface ClientContext {
    readonly headers: ReadonlyHeaders;
}
export type RejectionStatus = 400 | 403 | 422;

export interface ObservationRecorder {
    /** Records a registered code in this hook's private, bounded commit buffer. */
    readonly record: (name: string) => void;
}
interface ObservationContext {
    readonly observe: ObservationRecorder;
}

export interface RequestContext<State extends JsonObject = JsonObject> extends ObservationContext {
    readonly client: ClientContext;
    /** Records a business rejection (default 400), committed only when this hook succeeds. */
    readonly reject: (status?: RejectionStatus) => void;
    readonly meta: RequestMeta;
    /** The reference is fixed; its JSON fields can change. Starts as an empty object. */
    readonly state: State;
    body: RequestBody;
}
export interface BeforeSendContext<
    State extends JsonObject = JsonObject,
> extends ObservationContext {
    readonly client: ClientContext;
    readonly reject: (status?: RejectionStatus) => void;
    readonly meta: BeforeSendMeta;
    readonly state: State;
    body: RequestBody;
    url: string;
    readonly headers: HeadersDraft;
}
export interface AfterReceiveContext<
    State extends JsonObject = JsonObject,
> extends ObservationContext {
    readonly client: ClientContext;
    readonly meta: ReceivedMeta<"afterReceive">;
    readonly state: State;
    status: number;
    readonly headers: HeadersDraft;
}
export interface ResponseContext<State extends JsonObject = JsonObject> extends ObservationContext {
    readonly client: ClientContext;
    readonly meta: ReceivedMeta<"onResponse">;
    readonly state: State;
    /** Upstream JSON and its extensions are not restricted to a fixed response schema. */
    body: JsonObject;
}
export interface EventContext<
    P extends "onUpstreamEvent" | "onStreamEvent",
    State extends JsonObject = JsonObject,
> extends ObservationContext {
    readonly client: ClientContext;
    readonly meta: ReceivedMeta<P>;
    readonly state: State;
    event: StreamEvent;
    /** Original SSE envelope; null for events synthesized from a complete JSON response. */
    readonly sse: DeepReadonly<{ event: string; id: string | null; retry: string | null }> | null;
    /** At most once per invocation. [] drops the event; [a, b] replaces it in order. */
    readonly replaceEvents: (events: StreamEvent[]) => void;
}
export type UpstreamEventContext<State extends JsonObject = JsonObject> = EventContext<
    "onUpstreamEvent",
    State
>;
export type StreamEventContext<State extends JsonObject = JsonObject> = EventContext<
    "onStreamEvent",
    State
>;

/** Semantic lifecycle shared by complete JSON and SSE responses. Tool text is raw arguments. */
export type ContentEvent =
    | { type: "part_start"; choice: number; part: string; kind: "text" | "reasoning" | "refusal" }
    | {
          type: "part_start";
          choice: number;
          part: string;
          kind: "tool";
          tool: {
              index: number;
              type: "function" | "custom" | "legacy";
              id?: string;
              name?: string;
          };
      }
    | {
          type: "part_delta";
          choice: number;
          part: string;
          text: string;
          tool?: { id?: string; name?: string; type?: "function" | "custom" };
      }
    | { type: "part_end"; choice: number; part: string; finishReason?: string }
    | { type: "choice_end"; choice: number; finishReason: string }
    | { type: "response_end" };

export interface ContentContext<State extends JsonObject = JsonObject> extends ObservationContext {
    readonly client: ClientContext;
    readonly meta: ReceivedMeta<"onContent">;
    readonly state: State;
    event: ContentEvent;
    /** Source envelope only; null for complete JSON. Never use it to select business semantics. */
    readonly sse: DeepReadonly<{ event: string; id: string | null; retry: string | null }> | null;
    /** At most once per invocation; each result enters the remaining content steps in order. */
    readonly replaceEvents: (events: ContentEvent[]) => void;
}

/** Runs before an error response is sent; routing and upstream receipt may be absent. */
export interface ErrorResponseContext<
    State extends JsonObject = JsonObject,
> extends ObservationContext {
    readonly client: ClientContext;
    readonly meta: Meta<"onErrorResponse">;
    readonly state: State;
    /** Original failure classification, unaffected by response edits. */
    readonly error: DeepReadonly<ObserverError>;
    /** Rust requires an error status; a failure cannot become success. */
    status: number;
    readonly headers: HeadersDraft;
    /** UTF-8 error text. null preserves the current bytes, including non-text bodies. */
    body: string | null;
    /** null removes Content-Type. Rust validates non-null header values. */
    contentType: string | null;
}

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
export interface ObserverError {
    readonly code: ErrorCode;
    readonly status: number;
    readonly message: string;
    readonly request_id: string;
    /** May be a gateway operation such as authentication, not only a hook phase. */
    readonly phase: string;
    readonly step: string | null;
}
export interface ObserverStats {
    readonly usage: UsageStats;
    readonly timings: RequestTimings;
    readonly inputBytes: number;
    readonly outboundBytes: number;
    readonly upstreamBytes: number;
    readonly outputBytes: number;
    /** Milliseconds from request start until upstream headers arrive. */
    readonly upstreamHeadersMs: number | null;
    readonly elapsedMs: number;
    readonly finalUpstreamModel: string | null;
    /** Sanitized origin (scheme, host and optional port), without path or query. */
    readonly target: string | null;
}

export interface UsageStats {
    readonly source: "json" | "sse" | null;
    readonly quality: "missing" | "partial" | "reported" | "invalid" | "conflicting";
    readonly upstreamCompleted: boolean;
    readonly inputTokens: number | null;
    readonly outputTokens: number | null;
    readonly totalTokens: number | null;
    readonly cachedInputTokens: number | null;
    readonly reasoningOutputTokens: number | null;
}
export interface TimingSample {
    readonly totalMs: number;
    readonly samples: number;
}
export interface HookTimingSummary {
    readonly calls: number;
    readonly failedCalls: number;
    readonly interruptedCalls: number;
    readonly reportedCalls: number;
    readonly wallMs: number;
    readonly queue: TimingSample;
    readonly initialCall: TimingSample;
    readonly pending: TimingSample;
    readonly inputBridge: TimingSample;
    readonly outputBridge: TimingSample;
}
export type RequestHookPhase = Exclude<Phase, "onErrorResponse" | "onError" | "onFinish">;
export interface NativeTimingSummary {
    readonly calls: number;
    readonly failedCalls: number;
    readonly interruptedCalls: number;
    readonly wallMs: number;
}
export interface RequestTimings {
    readonly requestMs: number | null;
    readonly requestDecodeMs: number | null;
    readonly upstreamHeadersWaitMs: number | null;
    readonly upstreamFirstByteMs: number | null;
    readonly upstreamFirstChunkMs: number | null;
    readonly downstreamFirstEventMs: number | null;
    readonly upstreamReadWaitMs: number | null;
    readonly downstreamQueueWaitMs: number | null;
    readonly downstreamDrainMs: number | null;
    readonly hooks: Readonly<Partial<Record<RequestHookPhase, HookTimingSummary>>>;
    readonly native: Readonly<Partial<Record<RequestHookPhase, NativeTimingSummary>>>;
}

interface ObserverContext<
    P extends "onError" | "onFinish",
    State extends JsonObject,
> extends ObservationContext {
    readonly client: ClientContext;
    readonly meta: Meta<P>;
    readonly state: DeepReadonly<State>;
    readonly stats: ObserverStats;
}
export type ErrorContext<State extends JsonObject = JsonObject> = ObserverContext<
    "onError",
    State
> & {
    readonly outcome: "failed" | "cancelled";
    readonly error: ObserverError;
};
export type FinishContext<State extends JsonObject = JsonObject> = ObserverContext<
    "onFinish",
    State
> &
    (
        | { readonly outcome: "success"; readonly error?: never }
        | { readonly outcome: "failed" | "cancelled"; readonly error: ObserverError }
    );

/**
 * The union rejects returned objects in sync and async JS hooks while allowing
 * ordinary no-return functions. A bare `void` target alone would accept objects.
 */
export type Hook<Context> = (context: Context) => void | Promise<void>;

interface PipelineHooks<State extends JsonObject> {
    beforeSend: Hook<BeforeSendContext<State>>;
    afterReceive: Hook<AfterReceiveContext<State>>;
    onResponse: Hook<ResponseContext<State>>;
    onUpstreamEvent: Hook<UpstreamEventContext<State>>;
    onStreamEvent: Hook<StreamEventContext<State>>;
    onContent: Hook<ContentContext<State>>;
    onErrorResponse: Hook<ErrorResponseContext<State>>;
}
type AtLeastOne<T> = Partial<T> & { [Key in keyof T]: Pick<T, Key> }[keyof T];
/** A JavaScript step registers at least one hook and cannot contain a native descriptor. */
export type ScriptStep<State extends JsonObject = JsonObject> = {
    name: string;
    builtin?: never;
} & AtLeastOne<PipelineHooks<State>>;

export type BuiltinPolicy =
    | ({ kind: "capCompletionTokens" } & Omit<
          import("limen:policies").CapCompletionTokensOptions,
          "name"
      >)
    | ({ kind: "dropRequestFields" } & Omit<
          import("limen:policies").DropRequestFieldsOptions,
          "name"
      >)
    | ({ kind: "forwardClientHeader" } & Omit<
          import("limen:policies").ForwardClientHeaderOptions,
          "name"
      >)
    | ({ kind: "replaceText" } & Omit<import("limen:content").ReplaceTextOptions, "name">)
    | ({ kind: "syntheticTextTool" } & Omit<
          import("limen:content").SyntheticTextToolOptions,
          "name"
      >);
/** Native policy state remains owned by Rust and is absent from ctx.state. */
export type BuiltinStep = {
    readonly name: string;
    readonly builtin: Readonly<BuiltinPolicy>;
} & { [Key in keyof PipelineHooks<JsonObject>]?: never };
export type PipelineStep<State extends JsonObject = JsonObject> = ScriptStep<State> | BuiltinStep;

/** Defaults are shallow; client values win. Routing control fields are forbidden. */
export type Defaults = JsonObject & {
    model?: never;
    messages?: never;
    stream?: never;
};
export interface ProviderOptions {
    /** Complete HTTP(S) Chat Completions URL; no userinfo or fragment. */
    endpoint: string;
    auth?: { bearer: string };
    headers?: Record<string, string>;
    defaults?: Defaults;
}
export interface ModelOptions {
    upstream: string;
    defaults?: Defaults;
}
/** Synchronous assembly callbacks must finish without returning a value. */
export type GatewaySetup<State extends JsonObject = JsonObject> = (
    gateway: GatewayScope<State>,
) => undefined;
export type StepSetup<State extends JsonObject = JsonObject> = (
    step: StepScope<State>,
) => undefined;
/** Startup invokes the exported factory once, with no arguments. */
export type GatewayFactory = () => unknown;
export interface StepScope<State extends JsonObject = JsonObject> {
    readonly beforeSend: (hook: Hook<BeforeSendContext<State>>) => undefined;
    readonly afterReceive: (hook: Hook<AfterReceiveContext<State>>) => undefined;
    readonly onResponse: (hook: Hook<ResponseContext<State>>) => undefined;
    readonly onContent: (hook: Hook<ContentContext<State>>) => undefined;
    readonly onUpstreamEvent: (hook: Hook<UpstreamEventContext<State>>) => undefined;
    readonly onStreamEvent: (hook: Hook<StreamEventContext<State>>) => undefined;
    readonly onErrorResponse: (hook: Hook<ErrorResponseContext<State>>) => undefined;
}
export interface ModelScope<State extends JsonObject = JsonObject> {
    readonly use: (step: PipelineStep<State>) => undefined;
    readonly step: (name: string, setup: StepSetup<State>) => undefined;
}
export interface ProviderScope<State extends JsonObject = JsonObject> extends ModelScope<State> {
    readonly model: (alias: string, options: ModelOptions) => ModelScope<State>;
}
/** Scopes are valid only during setup; each hook and declaration name is unique. */
export interface GatewayScope<State extends JsonObject = JsonObject> {
    readonly provider: (id: string, options: ProviderOptions) => ProviderScope<State>;
    readonly onRequest: (hook: Hook<RequestContext<State>>) => undefined;
    readonly onErrorResponse: (hook: Hook<ErrorResponseContext<State>>) => undefined;
    readonly onError: (hook: Hook<ErrorContext<State>>) => undefined;
    readonly onFinish: (hook: Hook<FinishContext<State>>) => undefined;
}

export interface LimenHost {
    /** Missing => undefined; a present empty value remains "". May throw on invalid input. */
    readonly env: (name: string) => string | undefined;
}

declare global {
    /** The only Limen host namespace; no Node.js, DOM, network or filesystem globals. */
    const limen: LimenHost;
}
