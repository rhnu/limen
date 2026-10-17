//! HTTP application dispatch, independent of connection transport and resource startup.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::{
    body::{Body, to_bytes},
    http::{HeaderValue, Method, Request, header},
    response::Response,
};
use limen_config::AuthConfig;
use limen_script::Cancellation;
use serde_json::Value;

use crate::{
    admin::Admin,
    gateway::{self, ErrorCode, Failure, Gateway},
    routes::{self, InferenceRoute, Route},
};

pub(crate) struct ApplicationSettings {
    pub body_timeout: Duration,
    pub max_output_bytes: usize,
}

pub(crate) struct Application {
    auth: AuthConfig,
    models: Value,
    settings: ApplicationSettings,
    run_id: String,
    pub(crate) generation: Arc<Gateway>,
    admin: Option<Admin>,
    requests: AtomicU64,
    managers: AtomicU64,
}

impl Application {
    pub(crate) fn new(
        auth: AuthConfig,
        models: Value,
        settings: ApplicationSettings,
        run_id: String,
        generation: Arc<Gateway>,
        admin: Option<Admin>,
    ) -> Arc<Self> {
        Arc::new(Self {
            auth,
            models,
            settings,
            run_id,
            generation,
            admin,
            requests: AtomicU64::new(1),
            managers: AtomicU64::new(1),
        })
    }

    pub(crate) async fn handle(
        self: Arc<Self>,
        request: Request<Body>,
        connection_cancel: Cancellation,
    ) -> Response {
        let route = routes::classify(request.uri().path());
        let manager = matches!(&route, Route::Management(_));
        let (counter, namespace) = if manager {
            (&self.managers, "m")
        } else {
            (&self.requests, "r")
        };
        let id = format!(
            "limen-{namespace}-{}-{}",
            self.run_id,
            counter.fetch_add(1, Ordering::Relaxed)
        );
        let mut response = self.dispatch(route, request, id, connection_cancel).await;
        if manager {
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        }
        response
    }

    async fn dispatch(
        &self,
        route: Route,
        request: Request<Body>,
        id: String,
        connection_cancel: Cancellation,
    ) -> Response {
        let expected = match &route {
            Route::Inference(_) => Some(self.auth.api_key.as_str()),
            Route::Management(_) => self.auth.manager_key.as_deref(),
            Route::Public => None,
        };
        if let Some(expected) = expected {
            if let Err(error) = gateway::authorize(expected, request.headers()) {
                return gateway::error_response(&error, &id);
            }
        } else {
            if matches!(&route, Route::Public)
                && let Some(response) =
                    crate::web::handle(request, self.settings.body_timeout).await
            {
                return response;
            }
            return gateway::error_response(&Failure::new(ErrorCode::NotFound, "endpoint"), &id);
        }
        match route {
            Route::Management(route) => match &self.admin {
                Some(admin) => {
                    admin
                        .handle(route, request, &id, self.generation.accepting())
                        .await
                },
                None => {
                    gateway::error_response(&Failure::new(ErrorCode::NotFound, "endpoint"), &id)
                },
            },
            Route::Inference(route) => {
                let method = match route {
                    InferenceRoute::Chat => Method::POST,
                    InferenceRoute::Models => Method::GET,
                    InferenceRoute::Unknown => {
                        return gateway::error_response(
                            &Failure::new(ErrorCode::NotFound, "endpoint"),
                            &id,
                        );
                    },
                };
                if request.method() != method {
                    let mut response = gateway::error_response(
                        &Failure::new(ErrorCode::MethodNotAllowed, "endpoint"),
                        &id,
                    );
                    response.headers_mut().insert(
                        header::ALLOW,
                        HeaderValue::from_static(if method == Method::GET {
                            "GET"
                        } else {
                            "POST"
                        }),
                    );
                    return response;
                }
                if route == InferenceRoute::Chat {
                    return Arc::clone(&self.generation)
                        .handle_generation(request, id, connection_cancel)
                        .await;
                }
                let body = tokio::time::timeout(
                    self.settings.body_timeout,
                    to_bytes(request.into_body(), 0),
                )
                .await;
                if !matches!(body, Ok(Ok(_))) {
                    return gateway::error_response(
                        &Failure::new(ErrorCode::InvalidRequest, "request_body"),
                        &id,
                    );
                }
                match gateway::encode_json(
                    &self.models,
                    self.settings.max_output_bytes,
                    ErrorCode::PayloadTooLarge,
                    "models_response",
                ) {
                    Ok(body) => gateway::json_response(200, http::HeaderMap::new(), body, &id),
                    Err(error) => gateway::error_response(&error, &id),
                }
            },
            Route::Public => unreachable!("public routes return before authentication"),
        }
    }

    pub(crate) async fn drain(&self) {
        self.generation.drain().await;
        if let Some(admin) = &self.admin {
            admin.drain().await;
        }
    }
}

#[cfg(test)]
mod tests;
