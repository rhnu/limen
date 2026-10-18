/** Ambient declarations for modules embedded in Limen; no SDK runtime files are needed. */
declare module "limen:runtime" {
    /**
     * Reads one process environment variable without exposing the environment map.
     * @param name - Nonempty variable name without `=` or NUL; subject to the host byte limit.
     * @returns The Unicode value, `""` when empty, or `undefined` when absent.
     * @throws If the name is invalid, a byte limit is exceeded, or the value is not Unicode.
     */
    export function env(name: string): string | undefined;
}

declare module "limen:gateway" {
    type JsonObject = import("./limen").JsonObject;

    /**
     * Startup registration scope.
     * @typeParam State - Per-request JSON state, initially `{}`.
     */
    export type GatewayScope<State extends JsonObject = JsonObject> =
        import("./limen").GatewayScope<State>;
    /**
     * Provider pipeline and owned-model registration.
     * @typeParam State - Per-request JSON state.
     */
    export type ProviderScope<State extends JsonObject = JsonObject> =
        import("./limen").ProviderScope<State>;
    /**
     * Model pipeline registration.
     * @typeParam State - Per-request JSON state.
     */
    export type ModelScope<State extends JsonObject = JsonObject> =
        import("./limen").ModelScope<State>;
    /**
     * One step's hook registration.
     * @typeParam State - Per-request JSON state.
     */
    export type StepScope<State extends JsonObject = JsonObject> =
        import("./limen").StepScope<State>;
    /** Upstream endpoint, credentials, headers and shallow request defaults. */
    export type ProviderOptions = import("./limen").ProviderOptions;
    /** Upstream model name and model-specific shallow request defaults. */
    export type ModelOptions = import("./limen").ModelOptions;
    /**
     * Synchronous startup callback.
     * @typeParam State - Per-request JSON state.
     */
    export type GatewaySetup<State extends JsonObject = JsonObject> =
        import("./limen").GatewaySetup<State>;
    /**
     * Synchronous step callback.
     * @typeParam State - Per-request JSON state.
     */
    export type StepSetup<State extends JsonObject = JsonObject> =
        import("./limen").StepSetup<State>;
    /** Zero-argument factory consumed once by the host at startup. */
    export type GatewayFactory = import("./limen").GatewayFactory;
    /**
     * Creates the configuration module's default export; setup runs when the host invokes it.
     * @typeParam State - Per-request JSON state; required fields are not initialized automatically.
     * @param setup - Synchronous registration callback that must return `undefined`.
     * @returns A gateway factory to default-export from the configuration module.
     * @throws On invalid setup; registration and configuration errors fail startup.
     */
    export function defineGateway<State extends JsonObject = JsonObject>(
        setup: GatewaySetup<State>,
    ): GatewayFactory;
}

declare module "limen:policies" {
    type BuiltinStep = import("./limen").BuiltinStep;
    /** Options for a native `beforeSend` token-budget policy. */
    export interface CapCompletionTokensOptions {
        /** Step name, unique within the receiving provider or model pipeline. */
        name: string;
        /** Positive safe integer used when the selected field is absent or exceeds this value. */
        max: number;
        /**
         * Budget field to enforce.
         * @defaultValue `"max_completion_tokens"`
         */
        field?: "max_completion_tokens" | "max_tokens";
        /** Registered observation code, emitted only when the selected field changes. */
        onChange?: string;
    }

    /** Options for deleting top-level request fields in native `beforeSend`. */
    export interface DropRequestFieldsOptions {
        /** Step name, unique within the receiving pipeline. */
        name: string;
        /** Nonempty, unique field names; `model`, `messages` and `stream` are forbidden. */
        fields: readonly string[];
        /** Registered observation code, emitted once when any field is deleted. */
        onChange?: string;
    }

    /** Options for explicit, single-value client business-header forwarding. */
    export interface ForwardClientHeaderOptions {
        /** Step name, unique within the receiving pipeline. */
        name: string;
        /** Explicitly allowlisted business header, read case-insensitively by the host. */
        from: string;
        /** Outbound business header to replace; sensitive and transport headers are forbidden. */
        to: string;
    }

    /**
     * Sets or caps one token field; invalid values or the other budget field reject with 422.
     * @param options - Token limit, selected field and optional registered observation name.
     * @returns A native step for `provider.use()` or `model.use()`.
     * @throws On malformed options; the host validates option values at startup.
     */
    export function capCompletionTokens(options: CapCompletionTokensOptions): BuiltinStep;

    /**
     * Deletes the specified own request fields while preserving other extensions.
     * @param options - Fields to delete and optional registered observation name.
     * @returns A native step with a frozen copy of the field list.
     * @throws On malformed options; the host validates field names at startup.
     */
    export function dropRequestFields(options: DropRequestFieldsOptions): BuiltinStep;

    /**
     * Forwards one exposed header in `beforeSend`; duplicates reject with 422.
     * @param options - Source and destination business headers; absence leaves the target unchanged.
     * @returns A native step subject to the client-header allowlist and Connection exclusions.
     * @throws On malformed options; the host validates header names at startup.
     */
    export function forwardClientHeader(options: ForwardClientHeaderOptions): BuiltinStep;
}

declare module "limen:content" {
    type JsonObject = import("./limen").JsonObject;
    type BuiltinStep = import("./limen").BuiltinStep;
    type ScriptStep<State extends JsonObject> = import("./limen").ScriptStep<State>;
    /** Semantic string leaves; `tool` contains raw arguments or custom-tool input. */
    export type PartTextKind = "text" | "reasoning" | "refusal" | "tool";
    /** Frozen metadata passed to a complete-part text transform. */
    export interface PartTextInfo {
        /** Nonnegative source choice index. */
        readonly choice: number;
        /** Stable, opaque part identifier within the choice; do not parse its format. */
        readonly part: string;
        /** Semantic category selected by the policy. */
        readonly kind: PartTextKind;
        /** The upstream choice reason when supplied at the part boundary. */
        readonly finishReason?: string;
    }
    /** Options for buffering selected parts and mapping their complete text in JavaScript. */
    export interface MapPartTextOptions {
        /** Step name, unique within the receiving pipeline; 1–128 characters from `A-Za-z0-9._/-`. */
        name: string;
        /** Nonempty set of distinct part kinds to buffer; other parts pass through. */
        kinds: readonly PartTextKind[];
        /** Positive safe integer bounding retained input text per part in UTF-16 code units. */
        maxBufferedChars: number;
        /**
         * Positive safe integer limiting simultaneously buffered parts.
         * @defaultValue `64`
         */
        maxParts?: number;
        /**
         * Exclusive `ctx.state` key; same syntax as name.
         * @defaultValue `"content/map/<name>"`
         */
        stateKey?: string;
        /**
         * Maps a selected part at `part_end`; never called to flush a truncated stream.
         * @param text - Complete buffered input string, including an empty part.
         * @param info - Frozen part identity, kind and optional upstream finish reason.
         * @returns Replacement text; must be synchronous and subject to gateway output limits.
         */
        transform: (text: string, info: Readonly<PartTextInfo>) => string;
    }
    /**
     * Delays selected text until `part_end`, while forwarding tool metadata immediately.
     * @typeParam State - Per-request JSON state containing this policy's exclusive buffer slot.
     * @param options - Selected part kinds, buffer bounds and synchronous transform.
     * @returns A JavaScript `onContent` step; its buffer is visible in `ctx.state` until completion.
     * @throws On invalid options; overflow or an invalid transform result fails the request.
     */
    export function mapPartText<State extends JsonObject = JsonObject>(
        options: MapPartTextOptions,
    ): ScriptStep<State>;

    /** Options for streaming literal replacement in text parts only. */
    export interface ReplaceTextOptions {
        /** Step name, unique within the receiving pipeline. */
        name: string;
        /** Nonempty literal search string; matches are non-overlapping and may span deltas. */
        search: string;
        /** Literal replacement; empty text deletes matches, with no regex substitution syntax. */
        replacement: string;
        /** Positive retained-text bound in UTF-16 code units; must cover `search.length - 1`. */
        maxBufferedChars: number;
        /**
         * Positive bound on tracked parts.
         * @defaultValue `64`
         */
        maxParts?: number;
    }
    /**
     * Replaces text across delta boundaries and flushes the retained suffix at `part_end`.
     * @param options - Literal search/replacement and explicit per-part buffer limit.
     * @returns A native `onContent` step with private Rust state, outside `ctx.state`.
     * @throws On malformed options; the host validates limits and search text at startup.
     */
    export function replaceText(options: ReplaceTextOptions): BuiltinStep;

    /** Options for transporting assistant text through an injected function tool. */
    export interface SyntheticTextToolOptions {
        /** Step name, unique within the receiving pipeline. */
        name: string;
        /**
         * Unique tool name matching `[A-Za-z_][A-Za-z0-9_-]{0,63}`.
         * @defaultValue `"limen_emit_text"`
         */
        toolName?: string;
        /** UTF-16 bound on undecided serialized events and metadata; at least the tool-name length. */
        maxBufferedChars: number;
        /**
         * Positive bound on events held while tool identity is undecided.
         * @defaultValue `64`
         */
        maxPendingEvents?: number;
        /**
         * Positive bound on tracked parts.
         * @defaultValue `64`
         */
        maxParts?: number;
    }
    /**
     * Appends a text function and user instruction, requires tools and disables parallel calls.
     * @param options - Tool identity and bounded pending-event state; not a full-answer length limit.
     * @returns A native `beforeSend`/`onContent` step that decodes exactly `{"text": string}`.
     * @throws On malformed options; incompatible requests reject with 422, invalid tool JSON fails.
     * @remarks Accepts absent/auto `tool_choice` and function tools. Rejects legacy functions,
     * `response_format`, `n != 1`, parallel calls and tool-name collisions. Real tools pass through;
     * only synthetic-only `tool_calls` becomes `stop`. `length`/`content_filter` may close a valid
     * incomplete prefix. Parser state is private to Rust; complete arguments are never buffered.
     */
    export function syntheticTextTool(options: SyntheticTextToolOptions): BuiltinStep;
}
