import * as sdk from "./generated/sdk.gen";
import type {
    GetConfigData,
    GetLogsData,
    GetRequestsData,
    GetUsageData,
} from "./generated/types.gen";
import { isCurrentSession, onSessionReset, replaceSession, useSession } from "../session/store";
import { mapFailure } from "./errors";

const controllers = new Set<AbortController>();
onSessionReset(() => {
    for (const controller of controllers) controller.abort();
    controllers.clear();
});

type Options = {
    baseUrl: string;
    headers: { Authorization: string };
    signal: AbortSignal;
    credentials: "omit";
    cache: "no-store";
    redirect: "error";
    throwOnError: false;
};
async function call<T>(
    invoke: (options: Options) => Promise<{ data?: T; error?: unknown; response?: Response }>,
    signal?: AbortSignal,
    candidate?: { key: string; epoch: number },
): Promise<T> {
    const session = useSession.getState();
    const key = candidate?.key ?? session.key;
    const epoch = candidate?.epoch ?? session.epoch;
    if (!key || (!candidate && session.phase !== "authenticated"))
        throw new DOMException("Aborted", "AbortError");
    const controller = new AbortController();
    controllers.add(controller);
    try {
        const result = await invoke({
            baseUrl: window.location.origin,
            headers: { Authorization: `Bearer ${key}` },
            signal: signal ? AbortSignal.any([signal, controller.signal]) : controller.signal,
            credentials: "omit",
            cache: "no-store",
            redirect: "error",
            throwOnError: false,
        });
        if (!isCurrentSession(epoch) || controller.signal.aborted || signal?.aborted)
            throw new DOMException("Aborted", "AbortError");
        if (result.error !== undefined || result.data === undefined) {
            const failure = mapFailure(result.error, result.response);
            if (failure.status === 401 && !candidate) {
                replaceSession(null, "public");
                useSession.setState({ authError: failure.message });
            }
            throw failure;
        }
        return result.data;
    } finally {
        controllers.delete(controller);
    }
}

export const validateKey = (key: string, epoch: number) =>
    call((options) => sdk.getStatus(options), undefined, { key, epoch });
export const api = {
    status: (signal?: AbortSignal) => call((options) => sdk.getStatus(options), signal),
    usage: (query: GetUsageData["query"], signal?: AbortSignal) =>
        call((options) => sdk.getUsage({ ...options, query }), signal),
    requests: (query: GetRequestsData["query"], signal?: AbortSignal) =>
        call((options) => sdk.getRequests({ ...options, query }), signal),
    request: (requestId: string, signal?: AbortSignal) =>
        call((options) => sdk.getRequest({ ...options, path: { requestId } }), signal),
    files: (signal?: AbortSignal) => call((options) => sdk.getLogFiles(options), signal),
    logs: (query: GetLogsData["query"], signal?: AbortSignal) =>
        call((options) => sdk.getLogs({ ...options, query }), signal),
    config: (query: GetConfigData["query"], signal?: AbortSignal) =>
        call((options) => sdk.getConfig({ ...options, query }), signal),
};
