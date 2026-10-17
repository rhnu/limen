//! One classification preserves namespace authentication even for unknown endpoints.

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Route {
    Inference(InferenceRoute),
    Management(AdminRoute),
    Public,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum InferenceRoute {
    Chat,
    Models,
    Unknown,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum AdminRoute {
    Status,
    Config,
    Requests,
    Request(String),
    Usage,
    Logs,
    LogFiles,
    Unknown,
}

pub(crate) fn classify(path: &str) -> Route {
    if path == "/v1" || path.starts_with("/v1/") {
        return Route::Inference(match path {
            "/v1/chat/completions" => InferenceRoute::Chat,
            "/v1/models" => InferenceRoute::Models,
            _ => InferenceRoute::Unknown,
        });
    }
    if path == "/admin/v1" || path.starts_with("/admin/v1/") {
        return Route::Management(match path {
            "/admin/v1/status" => AdminRoute::Status,
            "/admin/v1/config" => AdminRoute::Config,
            "/admin/v1/requests" => AdminRoute::Requests,
            "/admin/v1/usage" => AdminRoute::Usage,
            "/admin/v1/logs" => AdminRoute::Logs,
            "/admin/v1/logs/files" => AdminRoute::LogFiles,
            _ => path
                .strip_prefix("/admin/v1/requests/")
                .filter(|id| !id.is_empty() && !id.contains('/'))
                .map_or(AdminRoute::Unknown, |id| AdminRoute::Request(id.to_owned())),
        });
    }
    Route::Public
}

#[cfg(test)]
mod tests;
