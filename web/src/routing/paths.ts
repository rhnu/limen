import { matchPath } from "react-router";

export const paths = {
    home: "/",
    usage: "/usage",
    requests: "/requests",
    requestPattern: "/requests/:id",
    logs: "/logs",
    status: "/status",
    config: "/config",
} as const;

export const managementNavigation = [
    { to: paths.usage, label: "用量" },
    { to: paths.requests, label: "请求记录" },
    { to: paths.logs, label: "运行日志" },
    { to: paths.status, label: "系统状态" },
    { to: paths.config, label: "配置" },
];

export function requestPath(id: string): string {
    return `${paths.requests}/${encodeURIComponent(id)}`;
}

/** Navigation state can be edited by callers; accept only local management destinations. */
export function safeReturnTo(value: unknown, fallback: string = paths.usage): string {
    if (
        typeof value !== "string" ||
        value.length > 4096 ||
        value.includes("\\") ||
        value.includes("#") ||
        Array.from(value).some((character) => character.charCodeAt(0) < 32)
    )
        return fallback;
    const pathname = value.split("?")[0];
    if (!pathname) return fallback;
    const allowed =
        managementNavigation.some(({ to }) => to === pathname) ||
        matchPath(paths.requestPattern, pathname) !== null;
    return allowed ? value : fallback;
}
