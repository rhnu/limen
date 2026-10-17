import type { ApiError } from "./generated/types.gen";

const messages: Record<string, string> = {
    invalid_query: "查询参数无效，请检查筛选条件。",
    unauthorized: "管理密钥无效或已失效，请重新登录。",
    not_found: "记录或管理接口不存在。",
    method_not_allowed: "此操作不受支持。",
    cursor_expired: "查询游标已失效，请重置查询。",
    query_too_broad: "查询范围过大，请缩小时间范围或筛选条件。",
    storage_unavailable: "存储暂不可用。",
    busy: "管理接口繁忙，请稍后手动重试。",
    query_timeout: "查询超时，请缩小查询范围后重试。",
    network_error: "无法读取管理接口，请检查连接后重试。",
    invalid_response: "管理接口返回了无法读取的响应。",
};
export class ApiFailure extends Error {
    constructor(
        public readonly code: string,
        public readonly status?: number,
        public readonly requestId?: string,
    ) {
        super(Object.hasOwn(messages, code) ? messages[code] : messages.invalid_response);
        this.name = "ApiFailure";
    }
}
export function mapFailure(error: unknown, response?: Response): ApiFailure {
    const detail =
        error && typeof error === "object" && "error" in error
            ? (error as ApiError).error
            : undefined;
    const code =
        response?.status === 401
            ? "unauthorized"
            : detail && typeof detail.code === "string" && Object.hasOwn(messages, detail.code)
              ? detail.code
              : response
                ? "invalid_response"
                : "network_error";
    const requestId =
        detail &&
        typeof detail.request_id === "string" &&
        /^[A-Za-z0-9_.:-]{1,160}$/.test(detail.request_id)
            ? detail.request_id
            : undefined;
    return new ApiFailure(code, response?.status, requestId);
}
