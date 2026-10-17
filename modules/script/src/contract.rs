//! Owned hook control data; protocol objects remain open JSON.

use serde_json::{Map, Value, json};

use crate::{ObservationBatch, ScriptError, ScriptErrorKind};

pub type ScriptState = Map<String, Value>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookPhase {
    OnRequest,
    BeforeSend,
    AfterReceive,
    OnResponse,
    OnContent,
    OnUpstreamEvent,
    OnStreamEvent,
    OnErrorResponse,
    OnError,
    OnFinish,
}

impl HookPhase {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OnRequest => "onRequest",
            Self::BeforeSend => "beforeSend",
            Self::AfterReceive => "afterReceive",
            Self::OnResponse => "onResponse",
            Self::OnContent => "onContent",
            Self::OnUpstreamEvent => "onUpstreamEvent",
            Self::OnStreamEvent => "onStreamEvent",
            Self::OnErrorResponse => "onErrorResponse",
            Self::OnError => "onError",
            Self::OnFinish => "onFinish",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct HookMeta {
    pub request_id: String,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub configured_upstream_model: Option<String>,
    pub client_stream: Option<bool>,
    pub upstream_status: Option<u16>,
    pub upstream_content_type: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ClientContext {
    pub headers: Vec<HeaderField>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeaderField {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug)]
pub struct RequestDraft {
    pub body: Value,
    pub url: String,
    pub headers: Vec<HeaderField>,
}

#[derive(Clone, Debug)]
pub struct ResponseHead {
    pub status: u16,
    pub headers: Vec<HeaderField>,
}

#[derive(Clone, Debug)]
pub struct SseContext {
    pub event: String,
    pub id: Option<String>,
    pub retry: Option<String>,
}

#[derive(Clone, Debug)]
pub struct EventInput {
    pub event: Value,
    pub sse: Option<SseContext>,
}

#[derive(Clone, Debug)]
pub struct ErrorResponseInput {
    pub error: Value,
    pub status: u16,
    pub headers: Vec<HeaderField>,
    pub body: Option<String>,
    pub content_type: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookOutcome {
    Success,
    Failed,
    Cancelled,
}

impl HookOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ObserverInput {
    pub outcome: HookOutcome,
    pub error: Option<Value>,
    pub stats: Value,
}

#[derive(Clone, Debug)]
pub enum HookInput {
    OnRequest { body: Value },
    BeforeSend(RequestDraft),
    AfterReceive(ResponseHead),
    OnResponse { body: Value },
    OnContent(EventInput),
    OnUpstreamEvent(EventInput),
    OnStreamEvent(EventInput),
    OnErrorResponse(ErrorResponseInput),
    OnError(ObserverInput),
    OnFinish(ObserverInput),
}

impl HookInput {
    #[must_use]
    pub const fn phase(&self) -> HookPhase {
        match self {
            Self::OnRequest { .. } => HookPhase::OnRequest,
            Self::BeforeSend(_) => HookPhase::BeforeSend,
            Self::AfterReceive(_) => HookPhase::AfterReceive,
            Self::OnResponse { .. } => HookPhase::OnResponse,
            Self::OnContent(_) => HookPhase::OnContent,
            Self::OnUpstreamEvent(_) => HookPhase::OnUpstreamEvent,
            Self::OnStreamEvent(_) => HookPhase::OnStreamEvent,
            Self::OnErrorResponse(_) => HookPhase::OnErrorResponse,
            Self::OnError(_) => HookPhase::OnError,
            Self::OnFinish(_) => HookPhase::OnFinish,
        }
    }
}

#[derive(Clone, Debug)]
pub struct HookContext {
    pub meta: HookMeta,
    pub client: ClientContext,
    pub state: ScriptState,
    pub input: HookInput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectionStatus {
    BadRequest,
    Forbidden,
    UnprocessableEntity,
}

impl RejectionStatus {
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        match self {
            Self::BadRequest => 400,
            Self::Forbidden => 403,
            Self::UnprocessableEntity => 422,
        }
    }
}

#[derive(Debug)]
pub struct HookEffects {
    pub state: ScriptState,
    pub observations: ObservationBatch,
}

#[derive(Debug)]
pub struct ErrorResponseDraft {
    pub status: u16,
    pub headers: Vec<HeaderField>,
    pub body: Option<String>,
    pub content_type: Option<String>,
}

#[derive(Debug)]
pub enum HookCandidate {
    OnRequest {
        body: Value,
        rejection: Option<RejectionStatus>,
    },
    BeforeSend {
        request: RequestDraft,
        rejection: Option<RejectionStatus>,
    },
    AfterReceive(ResponseHead),
    OnResponse {
        body: Value,
    },
    OnContent {
        events: Vec<Value>,
    },
    OnUpstreamEvent {
        events: Vec<Value>,
    },
    OnStreamEvent {
        events: Vec<Value>,
    },
    OnErrorResponse(ErrorResponseDraft),
    OnError,
    OnFinish,
}

/// Effects remain candidates until the caller completes its phase-specific checks.
#[derive(Debug)]
pub struct HookOutput {
    pub effects: HookEffects,
    pub candidate: HookCandidate,
}

pub(crate) fn headers_json(headers: &[HeaderField]) -> Value {
    Value::Array(
        headers
            .iter()
            .map(|field| json!({"name":field.name,"value":field.value}))
            .collect(),
    )
}

impl HookContext {
    /// Apply the same owned JSON budget used before a VM invocation.
    ///
    /// # Errors
    /// Rejects malformed phase payloads and values exceeding the JSON limits.
    pub fn validate(&self, limits: crate::JsonLimits) -> Result<(), ScriptError> {
        self.validate_shape()?;
        crate::json::encode_owned(&self.clone().into_json(), limits).map(|_| ())
    }

    pub(crate) fn validate_shape(&self) -> Result<(), ScriptError> {
        let valid = match &self.input {
            HookInput::OnRequest { body } | HookInput::OnResponse { body } => body.is_object(),
            HookInput::BeforeSend(input) => input.body.is_object(),
            HookInput::OnErrorResponse(input) => input.error.is_object(),
            HookInput::OnError(input) => {
                input.stats.is_object() && input.error.as_ref().is_some_and(Value::is_object)
            },
            HookInput::OnFinish(input) => {
                input.stats.is_object() && input.error.as_ref().is_none_or(Value::is_object)
            },
            _ => true,
        };
        if valid { Ok(()) } else { Err(invalid()) }
    }

    pub(crate) fn into_json(self) -> Value {
        let phase = self.input.phase();
        let meta = self.meta;
        let mut value = json!({"meta":{"requestId":meta.request_id,"phase":phase.as_str(),"model":meta.model,"provider":meta.provider,"configuredUpstreamModel":meta.configured_upstream_model,"clientStream":meta.client_stream,"upstreamStatus":meta.upstream_status,"upstreamContentType":meta.upstream_content_type},"client":{"headers":headers_json(&self.client.headers)},"state":self.state});
        match self.input {
            HookInput::OnRequest { body } | HookInput::OnResponse { body } => value["body"] = body,
            HookInput::BeforeSend(request) => {
                value["body"] = request.body;
                value["url"] = Value::String(request.url);
                value["headers"] = headers_json(&request.headers);
            },
            HookInput::AfterReceive(head) => {
                value["status"] = json!(head.status);
                value["headers"] = headers_json(&head.headers);
            },
            HookInput::OnContent(input)
            | HookInput::OnUpstreamEvent(input)
            | HookInput::OnStreamEvent(input) => {
                value["event"] = input.event;
                value["sse"] = input.sse.map_or(
                    Value::Null,
                    |sse| json!({"event":sse.event,"id":sse.id,"retry":sse.retry}),
                );
            },
            HookInput::OnErrorResponse(input) => {
                value["error"] = input.error;
                value["status"] = json!(input.status);
                value["headers"] = headers_json(&input.headers);
                value["body"] = json!(input.body);
                value["contentType"] = json!(input.content_type);
            },
            HookInput::OnError(input) | HookInput::OnFinish(input) => {
                value["outcome"] = json!(input.outcome.as_str());
                value["stats"] = input.stats;
                if let Some(error) = input.error {
                    value["error"] = error;
                }
            },
        }
        value
    }
}

fn invalid() -> ScriptError { ScriptError::new(ScriptErrorKind::Contract) }

fn take(map: &mut ScriptState, key: &str) -> Result<Value, ScriptError> {
    map.remove(key).ok_or_else(invalid)
}

fn body(map: &mut ScriptState) -> Result<Value, ScriptError> {
    let value = take(map, "body")?;
    if !value.is_object() {
        return Err(invalid());
    }
    Ok(value)
}

fn string(map: &mut ScriptState, key: &str) -> Result<String, ScriptError> {
    match take(map, key)? {
        Value::String(value) => Ok(value),
        _ => Err(invalid()),
    }
}

fn optional_string(map: &mut ScriptState, key: &str) -> Result<Option<String>, ScriptError> {
    match take(map, key)? {
        Value::Null => Ok(None),
        Value::String(value) => Ok(Some(value)),
        _ => Err(invalid()),
    }
}

fn status(map: &mut ScriptState) -> Result<u16, ScriptError> {
    take(map, "status")?
        .as_u64()
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(invalid)
}

fn headers(map: &mut ScriptState) -> Result<Vec<HeaderField>, ScriptError> {
    let Value::Array(fields) = take(map, "headers")? else {
        return Err(invalid());
    };
    fields
        .into_iter()
        .map(|field| {
            let Value::Object(mut field) = field else {
                return Err(invalid());
            };
            if field.len() != 2 {
                return Err(invalid());
            }
            Ok(HeaderField {
                name: string(&mut field, "name")?,
                value: string(&mut field, "value")?,
            })
        })
        .collect()
}

fn rejection(map: &mut ScriptState) -> Result<Option<RejectionStatus>, ScriptError> {
    let Some(value) = map.remove("rejection") else {
        return Ok(None);
    };
    let Value::Object(mut value) = value else {
        return Err(invalid());
    };
    if value.len() != 1 {
        return Err(invalid());
    }
    match status(&mut value)? {
        400 => Ok(Some(RejectionStatus::BadRequest)),
        403 => Ok(Some(RejectionStatus::Forbidden)),
        422 => Ok(Some(RejectionStatus::UnprocessableEntity)),
        _ => Err(invalid()),
    }
}

pub(crate) fn decode_candidate(
    context: Value,
    phase: HookPhase,
    observations: ObservationBatch,
) -> Result<HookOutput, ScriptError> {
    let Value::Object(mut context) = context else {
        return Err(invalid());
    };
    let Value::Object(state) = take(&mut context, "state")? else {
        return Err(invalid());
    };
    let candidate = match phase {
        HookPhase::OnRequest => HookCandidate::OnRequest {
            body: body(&mut context)?,
            rejection: rejection(&mut context)?,
        },
        HookPhase::BeforeSend => HookCandidate::BeforeSend {
            request: RequestDraft {
                body: body(&mut context)?,
                url: string(&mut context, "url")?,
                headers: headers(&mut context)?,
            },
            rejection: rejection(&mut context)?,
        },
        HookPhase::AfterReceive => HookCandidate::AfterReceive(ResponseHead {
            status: status(&mut context)?,
            headers: headers(&mut context)?,
        }),
        HookPhase::OnResponse => HookCandidate::OnResponse {
            body: body(&mut context)?,
        },
        HookPhase::OnContent | HookPhase::OnUpstreamEvent | HookPhase::OnStreamEvent => {
            let Value::Array(events) = take(&mut context, "events")? else {
                return Err(invalid());
            };
            match phase {
                HookPhase::OnContent => HookCandidate::OnContent { events },
                HookPhase::OnUpstreamEvent => HookCandidate::OnUpstreamEvent { events },
                _ => HookCandidate::OnStreamEvent { events },
            }
        },
        HookPhase::OnErrorResponse => HookCandidate::OnErrorResponse(ErrorResponseDraft {
            status: status(&mut context)?,
            headers: headers(&mut context)?,
            body: optional_string(&mut context, "body")?,
            content_type: optional_string(&mut context, "contentType")?,
        }),
        HookPhase::OnError => HookCandidate::OnError,
        HookPhase::OnFinish => HookCandidate::OnFinish,
    };
    if !matches!(phase, HookPhase::OnRequest | HookPhase::BeforeSend)
        && context.contains_key("rejection")
    {
        return Err(invalid());
    }
    Ok(HookOutput {
        effects: HookEffects {
            state,
            observations,
        },
        candidate,
    })
}
