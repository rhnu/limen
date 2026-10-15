//! Named, read-only access to process environment variables from the owning VM.

use std::env::VarError;

use rquickjs::{Ctx, Exception, Function};

#[derive(Debug, thiserror::Error)]
enum ReadError {
    #[error("environment variable name must be nonempty and exclude '=' and NUL")]
    InvalidName,
    #[error("environment variable name exceeds byte limit")]
    NameLimit,
    #[error("environment variable value exceeds byte limit")]
    ValueLimit,
    #[error("environment variable value is not Unicode")]
    NotUnicode(#[source] VarError),
}

/// Validate before lookup; preserve missing, empty and non-Unicode as distinct results.
fn read_with(
    name: &str,
    max_bytes: usize,
    lookup: impl FnOnce(&str) -> Result<String, VarError>,
) -> Result<Option<String>, ReadError> {
    if name.is_empty() || name.contains(['=', '\0']) {
        return Err(ReadError::InvalidName);
    }
    if name.len() > max_bytes {
        return Err(ReadError::NameLimit);
    }
    match lookup(name) {
        Ok(value) if value.len() > max_bytes => Err(ReadError::ValueLimit),
        Ok(value) => Ok(Some(value)),
        Err(VarError::NotPresent) => Ok(None),
        Err(error @ VarError::NotUnicode(_)) => Err(ReadError::NotUnicode(error)),
    }
}

/// Build the bounded lookup function; the host module owns namespace publication.
pub(super) fn function<'js>(ctx: &Ctx<'js>, max_bytes: usize) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, name: rquickjs::String<'_>| {
            let name = name.to_string()?;
            read_with(&name, max_bytes, |name| std::env::var(name)).map_err(|error| {
                // Only fixed messages enter JS; OS errors can contain the secret value.
                let message = error.to_string();
                match error {
                    ReadError::InvalidName => Exception::throw_type(&ctx, &message),
                    ReadError::NameLimit | ReadError::ValueLimit => {
                        Exception::throw_range(&ctx, &message)
                    },
                    ReadError::NotUnicode(_) => Exception::throw_message(&ctx, &message),
                }
            })
        },
    )
}

#[cfg(test)]
mod tests;
