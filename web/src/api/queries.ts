import { queryOptions } from "@tanstack/react-query";
import { api } from "./client";
import { ApiFailure } from "./errors";
import type {
    ConfigResponse,
    ModelConfig,
    ProviderConfig,
    ServerConfig,
} from "./generated/types.gen";
import { dateQuery } from "../lib/values";
import {
    serializeSearch,
    type ConfigSearch,
    type LogsSearch,
    type RequestsSearch,
    type UsageSearch,
} from "../routing/search";

function requireFilters<T>(filters: T | null): T {
    if (filters === null) throw new ApiFailure("invalid_query", 400);
    return filters;
}
const identity = (filters: Record<string, string | undefined> | null) =>
    filters === null ? null : serializeSearch(filters).toString();

type ConfigView =
    | { section: "server"; data: ServerConfig }
    | { section: "models"; data: ModelConfig[]; nextCursor: string | null }
    | { section: "providers"; data: ProviderConfig[]; nextCursor: string | null };

// The generated union has no discriminator. Bind the result to the requested
// section once, including empty pages, instead of asserting array types in JSX.
function configView(section: ConfigSearch["section"], response: ConfigResponse): ConfigView {
    if (section === "server" && !Array.isArray(response.data))
        return { section, data: response.data };
    if (Array.isArray(response.data) && "nextCursor" in response) {
        if (
            section === "models" &&
            response.data.every((item): item is ModelConfig => "alias" in item)
        )
            return { section, data: response.data, nextCursor: response.nextCursor };
        if (
            section === "providers" &&
            response.data.every((item): item is ProviderConfig => "id" in item)
        )
            return { section, data: response.data, nextCursor: response.nextCursor };
    }
    throw new ApiFailure("invalid_response");
}

export const queries = {
    status: (epoch: number, valid = true) =>
        queryOptions({
            queryKey: [epoch, "status"],
            queryFn: ({ signal }) => {
                requireFilters(valid ? true : null);
                return api.status(signal);
            },
            enabled: valid,
        }),
    usage: (epoch: number, filters: UsageSearch | null) =>
        queryOptions({
            queryKey: [epoch, "usage", identity(filters)],
            queryFn: ({ signal }) => {
                const { from, to, ...rest } = requireFilters(filters);
                return api.usage({ ...rest, ...dateQuery(from, to) }, signal);
            },
            enabled: filters !== null,
        }),
    requests: (epoch: number, filters: RequestsSearch | null, cursor?: string) =>
        queryOptions({
            queryKey: [epoch, "requests", identity(filters), cursor ?? null],
            queryFn: ({ signal }) => {
                const { from, to, ...rest } = requireFilters(filters);
                return api.requests(
                    cursor ? { cursor, limit: 50 } : { ...rest, ...dateQuery(from, to), limit: 50 },
                    signal,
                );
            },
            enabled: filters !== null,
        }),
    request: (epoch: number, id: string | null, valid = true) =>
        queryOptions({
            queryKey: [epoch, "request", valid ? id : null],
            queryFn: ({ signal }) => api.request(requireFilters(valid ? id : null), signal),
            enabled: valid && id !== null,
        }),
    files: (epoch: number, valid = true) =>
        queryOptions({
            queryKey: [epoch, "log-files"],
            queryFn: ({ signal }) => {
                requireFilters(valid ? true : null);
                return api.files(signal);
            },
            enabled: valid,
        }),
    logs: (epoch: number, filters: LogsSearch | null, cursor?: string, enabled = true) =>
        queryOptions({
            queryKey: [epoch, "logs", identity(filters), cursor ?? null],
            queryFn: ({ signal }) => {
                const current = requireFilters(filters);
                return api.logs(
                    cursor ? { cursor, limit: 100 } : { ...current, limit: 100 },
                    signal,
                );
            },
            enabled: filters !== null && enabled,
        }),
    config: (epoch: number, filters: ConfigSearch | null, cursor?: string) =>
        queryOptions({
            queryKey: [epoch, "config", identity(filters), cursor ?? null],
            queryFn: async ({ signal }) => {
                const { section } = requireFilters(filters);
                const response = await api.config(
                    section === "server"
                        ? { section }
                        : cursor
                          ? { cursor, limit: 50 }
                          : { section, limit: 50 },
                    signal,
                );
                return configView(section, response);
            },
            enabled: filters !== null,
        }),
};
