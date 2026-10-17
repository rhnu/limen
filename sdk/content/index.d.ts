import type { JsonObject, PipelineStep } from "../limen";

export type PartTextKind = "text" | "reasoning" | "refusal" | "tool";
export interface PartTextInfo {
  readonly choice: number;
  readonly part: string;
  readonly kind: PartTextKind;
  /** The upstream choice reason when supplied at the part boundary. */
  readonly finishReason?: string;
}
export interface MapPartTextOptions {
  name: string;
  kinds: readonly PartTextKind[];
  /** Strict retained input-text bound per selected part, measured in UTF-16 units. */
  maxBufferedChars: number;
  maxParts?: number;
  /** Unique request-state slot; defaults to content/map/<name>. */
  stateKey?: string;
  /** Pure synchronous callback; output is separately subject to gateway output limits. */
  transform: (text: string, info: Readonly<PartTextInfo>) => string;
}
/** Buffers selected part text and maps it once at part_end, forwarding tool metadata immediately. */
export function mapPartText<State extends JsonObject = JsonObject>(options: MapPartTextOptions): PipelineStep<State>;

export interface ReplaceTextOptions {
  name: string;
  search: string;
  replacement: string;
  /** Maximum retained UTF-16 code units per part; must cover search.length - 1. */
  maxBufferedChars: number;
  maxParts?: number;
  /** Unique request-state slot; defaults to content/replace/<name>. */
  stateKey?: string;
}
/** Literal, non-overlapping replacement independent of source delta boundaries. */
export function replaceText<State extends JsonObject = JsonObject>(options: ReplaceTextOptions): PipelineStep<State>;

export interface SyntheticTextToolOptions {
  name: string;
  toolName?: string;
  /** Bound on JSON-serialized undecided source events (UTF-16 units), plus fixed parser bookkeeping. */
  maxBufferedChars: number;
  maxPendingEvents?: number;
  maxParts?: number;
  /** Unique request-state slot; defaults to content/tool/<name>. */
  stateKey?: string;
}
/**
 * Appends a text-output function and a user transport instruction without changing history.
 * Accepts absent/auto tool_choice and ordinary function tools; rejects legacy
 * functions, required/named/none choices, structured output, n != 1 and parallel_tool_calls:true.
 * The synthetic arguments must be exactly {"text": string}, with Unicode scalar text.
 * Actual tool calls pass through, including choices that also contain synthetic text.
 * Only a tool_calls finish with synthetic text alone becomes stop; real tools retain it.
 * length/content_filter may close a valid incomplete JSON prefix and preserve that reason.
 * Parser state is plain bounded JSON and never retains the complete argument string.
 */
export function syntheticTextTool<State extends JsonObject = JsonObject>(options: SyntheticTextToolOptions): PipelineStep<State>;
