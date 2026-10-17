import type { JsonObject, PipelineStep } from "../limen";

export interface CapCompletionTokensOptions {
  name: string;
  max: number;
  field?: "max_completion_tokens" | "max_tokens";
  /** Registered observation code, emitted only when the selected field changes. */
  onChange?: string;
}

export interface DropRequestFieldsOptions {
  name: string;
  fields: readonly string[];
  /** Registered observation code, emitted once when any field is deleted. */
  onChange?: string;
}

export interface ForwardClientHeaderOptions {
  name: string;
  /** Explicitly allowlisted business header, read case-insensitively by the host. */
  from: string;
  to: string;
}

export function capCompletionTokens<State extends JsonObject = JsonObject>(
  options: CapCompletionTokensOptions,
): PipelineStep<State>;

export function dropRequestFields<State extends JsonObject = JsonObject>(
  options: DropRequestFieldsOptions,
): PipelineStep<State>;

export function forwardClientHeader<State extends JsonObject = JsonObject>(
  options: ForwardClientHeaderOptions,
): PipelineStep<State>;
