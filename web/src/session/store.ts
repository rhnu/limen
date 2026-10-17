import { create } from "zustand";

export const storageKey = "limen.managerKey";
type StoragePort = Pick<Storage, "getItem" | "setItem" | "removeItem">;
export type SessionState = {
    key: string | null;
    phase: "public" | "verifying" | "authenticated" | "blocked";
    epoch: number;
    storageError: string | null;
    authError: string | null;
    verificationError: unknown;
};

export function readKey(storage: StoragePort): Pick<SessionState, "key" | "storageError"> {
    try {
        return { key: storage.getItem(storageKey), storageError: null };
    } catch {
        return { key: null, storageError: "浏览器存储不可读，无法恢复管理会话。" };
    }
}

export function writeKey(storage: StoragePort, key: string | null): string | null {
    try {
        if (key === null) storage.removeItem(storageKey);
        else storage.setItem(storageKey, key);
        return null;
    } catch {
        return key === null
            ? "浏览器存储清除失败，请在浏览器设置中清除本站数据。"
            : "浏览器存储写入失败，本次会话仅在当前页面有效。";
    }
}

function initial(): SessionState {
    let restored: ReturnType<typeof readKey>;
    try {
        restored = readKey(window.localStorage);
    } catch {
        restored = { key: null, storageError: "浏览器存储不可用。" };
    }
    return {
        ...restored,
        phase: restored.key ? "verifying" : "public",
        epoch: 0,
        authError: null,
        verificationError: null,
    };
}

export const useSession = create<SessionState>(initial);
const cleanup = new Set<() => void>();
export function onSessionReset(callback: () => void): () => void {
    cleanup.add(callback);
    return () => {
        cleanup.delete(callback);
    };
}

export function replaceSession(key: string | null, phase: SessionState["phase"]): number {
    for (const callback of cleanup) callback();
    let storageError: string | null;
    try {
        storageError = writeKey(window.localStorage, key);
    } catch {
        storageError = "浏览器存储不可用，请检查本站的存储权限。";
    }
    const epoch = useSession.getState().epoch + 1;
    useSession.setState({
        key,
        phase,
        epoch,
        storageError,
        authError: null,
        verificationError: null,
    });
    return epoch;
}

export function isCurrentSession(epoch: number): boolean {
    return useSession.getState().epoch === epoch;
}
