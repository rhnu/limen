import { validateKey } from "../api/client";
import { ApiFailure } from "../api/errors";
import { isCurrentSession, useSession, replaceSession } from "./store";

let restoring: { epoch: number; promise: Promise<void> } | null = null;

/** Validate restored credentials once per session; late completion cannot change a new session. */
export function restoreSession(): Promise<void> {
    const { key, epoch, phase } = useSession.getState();
    if (!key || phase !== "verifying") return Promise.resolve();
    if (restoring?.epoch === epoch) return restoring.promise;
    useSession.setState({ verificationError: null });
    const promise = validateKey(key, epoch)
        .then(() => {
            if (isCurrentSession(epoch)) useSession.setState({ phase: "authenticated" });
        })
        .catch((error: unknown) => {
            if (!isCurrentSession(epoch)) return;
            if (error instanceof ApiFailure && error.status === 401) {
                replaceSession(null, "public");
                useSession.setState({ authError: "管理密钥已失效，请重新登录。" });
            } else {
                useSession.setState({ phase: "blocked", verificationError: error });
            }
        })
        .finally(() => {
            if (restoring?.epoch === epoch) restoring = null;
        });
    restoring = { epoch, promise };
    return promise;
}
