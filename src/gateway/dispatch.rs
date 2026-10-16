use axum::{
    body::{Body, to_bytes},
    http::{HeaderValue, Method, Request, header},
    response::Response,
};

use super::{
    Gateway,
    error::{ErrorCode, Failure},
    error_response, json_response, logic,
};

pub(super) enum Dispatch {
    Generation(Request<Body>),
    Response(Response),
}

impl Gateway {
    /// Authentication and management routing happen before generation admission or JS work.
    pub(super) async fn dispatch(&self, request: Request<Body>, id: &str) -> Dispatch {
        let path = request.uri().path();
        let admin = path == "/admin/v1" || path.starts_with("/admin/v1/");
        let expected = if admin {
            self.server.auth.manager_key.as_deref()
        } else if path == "/v1" || path.starts_with("/v1/") {
            Some(self.server.auth.api_key.as_str())
        } else {
            None
        };
        let Some(expected) = expected else {
            return Dispatch::Response(error_response(
                &Failure::new(ErrorCode::NotFound, "endpoint"),
                id,
            ));
        };
        if let Err(failure) = logic::authorize(expected, request.headers()) {
            return Dispatch::Response(error_response(&failure, id));
        }
        if admin {
            let Some(admin) = &self.admin else {
                return Dispatch::Response(error_response(
                    &Failure::new(ErrorCode::NotFound, "endpoint"),
                    id,
                ));
            };
            return Dispatch::Response(admin.handle(request, id, self.ready()).await);
        }
        let method = match path {
            "/v1/chat/completions" => Method::POST,
            "/v1/models" => Method::GET,
            _ => {
                return Dispatch::Response(error_response(
                    &Failure::new(ErrorCode::NotFound, "endpoint"),
                    id,
                ));
            },
        };
        if request.method() != method {
            let mut response =
                error_response(&Failure::new(ErrorCode::MethodNotAllowed, "endpoint"), id);
            response.headers_mut().insert(
                header::ALLOW,
                HeaderValue::from_static(if method == Method::GET { "GET" } else { "POST" }),
            );
            return Dispatch::Response(response);
        }
        if method == Method::GET {
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(self.server.limits.request_ms),
                to_bytes(request.into_body(), 0),
            )
            .await;
            if !matches!(result, Ok(Ok(_))) {
                return Dispatch::Response(error_response(
                    &Failure::new(ErrorCode::InvalidRequest, "request_body"),
                    id,
                ));
            }
            let body = logic::models_json(&self.business);
            let bytes = match super::json::encode(
                &body,
                self.server.limits.max_output_bytes,
                ErrorCode::PayloadTooLarge,
                "models_response",
            ) {
                Ok(bytes) => bytes,
                Err(failure) => return Dispatch::Response(error_response(&failure, id)),
            };
            return Dispatch::Response(json_response(200, http::HeaderMap::new(), bytes, id));
        }
        Dispatch::Generation(request)
    }
}
