use std::{error::Error, fmt, sync::Arc};

/// Stable policy categories; diagnostic sources do not participate in classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptErrorKind {
    InvalidOptions,
    Module,
    JavaScript,
    Json,
    Contract,
    UnknownHook,
    Busy,
    Cancelled,
    Deadline,
    ExecutionBudget,
    MemoryLimit,
    Retired,
    Stopped,
    Thread,
}

/// Owned causes can cross threads; JavaScript exception values never leave the VM.
#[derive(Clone)]
pub struct ScriptError {
    kind: ScriptErrorKind,
    source: Option<Arc<dyn Error + Send + Sync>>,
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JavaScript runtime: {:?}", self.kind)
    }
}

impl Error for ScriptError {
    // Return the pointee: deriving source for Arc would expose the shared wrapper,
    // hiding the original type from downcast and cause-chain rendering.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

impl ScriptError {
    #[must_use]
    pub const fn new(kind: ScriptErrorKind) -> Self { Self { kind, source: None } }

    #[must_use]
    pub fn with_source<E>(kind: ScriptErrorKind, source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self {
            kind,
            source: Some(Arc::new(source)),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> ScriptErrorKind { self.kind }

    #[must_use]
    pub fn reclassify(mut self, kind: ScriptErrorKind) -> Self {
        self.kind = kind;
        self
    }

    /// Render only trusted typed metadata, never arbitrary source Display/Debug text.
    pub fn diagnostic(&self) -> impl fmt::Display + '_ { Diagnostic(self) }
}

impl fmt::Debug for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.diagnostic(), f)
    }
}

struct Diagnostic<'a>(&'a ScriptError);

impl fmt::Display for Diagnostic<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.0, f)?;
        let mut source = self.0.source();
        for _ in 0..8 {
            let Some(error) = source else {
                return Ok(());
            };
            f.write_str("; cause: ")?;
            safe_cause(error, f)?;
            source = error.source();
        }
        if source.is_some() {
            f.write_str("; further causes omitted")?;
        }
        Ok(())
    }
}

fn safe_cause(source: &(dyn Error + 'static), f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if let Some(error) = source.downcast_ref::<std::io::Error>() {
        write!(
            f,
            "io_kind={:?}, os_code={:?}",
            error.kind(),
            error.raw_os_error()
        )
    } else if let Some(error) = source.downcast_ref::<serde_json::Error>() {
        write!(
            f,
            "json_category={:?}, line={}, column={}",
            error.classify(),
            error.line(),
            error.column()
        )
    } else if let Some(error) = source.downcast_ref::<rquickjs::Error>() {
        write!(f, "quickjs={}", quickjs_kind(error))
    } else if let Some(error) = source.downcast_ref::<ThreadFailure>() {
        write!(f, "thread={error}")
    } else if let Some(error) =
        source.downcast_ref::<tokio::sync::mpsc::error::TrySendError<crate::Command>>()
    {
        match error {
            tokio::sync::mpsc::error::TrySendError::Full(_) => f.write_str("channel=mpsc_full"),
            tokio::sync::mpsc::error::TrySendError::Closed(_) => f.write_str("channel=mpsc_closed"),
        }
    } else if source.is::<tokio::sync::oneshot::error::RecvError>() {
        f.write_str("channel=oneshot_closed")
    } else if source.is::<tokio::sync::watch::error::RecvError>() {
        f.write_str("channel=watch_closed")
    } else {
        f.write_str("source=unclassified")
    }
}

fn quickjs_kind(error: &rquickjs::Error) -> &'static str {
    use rquickjs::Error as Js;
    match error {
        Js::Allocation => "Allocation",
        Js::Exception => "Exception",
        Js::DuplicateExports => "DuplicateExports",
        Js::InvalidExport => "InvalidExport",
        Js::InvalidClass { .. } => "InvalidClass",
        Js::InvalidString(_) => "InvalidString",
        Js::InvalidCStr(_) => "InvalidCStr",
        Js::Utf8(_) => "Utf8",
        Js::Io(_) => "Io",
        Js::ClassBorrow(_) => "ClassBorrow",
        Js::FunctionBorrow(_) => "FunctionBorrow",
        Js::FromJs { .. } => "FromJs",
        Js::IntoJs { .. } => "IntoJs",
        Js::MissingArgs { .. } => "MissingArgs",
        Js::TooManyArgs { .. } => "TooManyArgs",
        Js::Resolving { .. } => "Resolving",
        Js::Loading { .. } => "Loading",
        Js::AsSlice(_) => "AsSlice",
        Js::UnrelatedRuntime => "UnrelatedRuntime",
        Js::WouldBlock => "WouldBlock",
        Js::UserData(_) => "UserData",
        Js::Unknown => "Unknown",
        _ => "Other",
    }
}

/// Panic payloads are deliberately not retained: they can contain script secrets.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ThreadFailure {
    #[error("panic")]
    Panic,
}

#[cfg(test)]
mod tests;
