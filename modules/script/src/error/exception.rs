//! Only fixed exception classes and coordinates in registered modules leave the VM.

use std::fmt;

#[derive(Clone, Copy, Debug)]
pub(crate) enum ExceptionKind {
    Error,
    EvalError,
    RangeError,
    ReferenceError,
    SyntaxError,
    TypeError,
    UriError,
    AggregateError,
    InternalError,
    ThrownValue,
}

impl ExceptionKind {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Error" => Self::Error,
            "EvalError" => Self::EvalError,
            "RangeError" => Self::RangeError,
            "ReferenceError" => Self::ReferenceError,
            "SyntaxError" => Self::SyntaxError,
            "TypeError" => Self::TypeError,
            "URIError" => Self::UriError,
            "AggregateError" => Self::AggregateError,
            "InternalError" => Self::InternalError,
            _ => Self::ThrownValue,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::EvalError => "EvalError",
            Self::RangeError => "RangeError",
            Self::ReferenceError => "ReferenceError",
            Self::SyntaxError => "SyntaxError",
            Self::TypeError => "TypeError",
            Self::UriError => "URIError",
            Self::AggregateError => "AggregateError",
            Self::InternalError => "InternalError",
            Self::ThrownValue => "ThrownValue",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Location {
    pub module: usize,
    pub line: u32,
    pub column: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ExceptionDiagnostic {
    pub kind: ExceptionKind,
    pub location: Option<Location>,
}

impl fmt::Display for ExceptionDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "exception={}", self.kind.name())?;
        if let Some(location) = self.location {
            if location.module == 0 {
                f.write_str(", module=config.js")?;
            } else {
                write!(f, ", module=import#{}", location.module)?;
            }
            write!(f, ", line={}", location.line)?;
            if let Some(column) = location.column {
                write!(f, ", column={column}")?;
            }
        }
        Ok(())
    }
}

/// Parse bounded `QuickJS` frames without returning paths, function names or stack text.
pub(crate) fn locate(stack: &str, modules: &[String]) -> Option<Location> {
    if stack.len() > 16_384 {
        return None;
    }
    for frame in stack.lines().take(64) {
        let Some(frame) = frame.strip_prefix("    at ") else {
            continue;
        };
        let frame = frame.strip_suffix(')').unwrap_or(frame);
        let Some((prefix, last)) = frame.rsplit_once(':') else {
            continue;
        };
        let Ok(last) = last.parse::<u32>() else {
            continue;
        };
        let (path, line, column) = match prefix
            .rsplit_once(':')
            .and_then(|(path, line)| line.parse::<u32>().ok().map(|line| (path, line)))
        {
            Some((path, line)) => (path, line, (last != 0).then_some(last)),
            _ => (prefix, last, None),
        };
        if line == 0 {
            continue;
        }
        if let Some(module) = modules.iter().position(|name| {
            path == name
                || path
                    .strip_suffix(name.as_str())
                    .is_some_and(|s| s.ends_with(" ("))
        }) {
            return Some(Location {
                module,
                line,
                column,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests;
