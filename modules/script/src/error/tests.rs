use std::{error::Error, fmt, io};

use super::{
    ScriptError, ScriptErrorKind, ThreadFailure,
    exception::{ExceptionDiagnostic, ExceptionKind, Location},
};

#[test]
fn exception_metadata_survives_reclassification_without_replacing_the_source() {
    let error = ScriptError::with_source(ScriptErrorKind::JavaScript, rquickjs::Error::Exception)
        .with_exception(ExceptionDiagnostic {
            kind: ExceptionKind::ReferenceError,
            location: Some(Location {
                module: 1,
                line: 4,
                column: Some(3),
            }),
        });
    let copy = error.clone().reclassify(ScriptErrorKind::Module);
    assert!(std::ptr::eq(
        error.source().unwrap(),
        copy.source().unwrap()
    ));
    assert!(copy.source().unwrap().is::<rquickjs::Error>());
    let diagnostic = copy.diagnostic().to_string();
    assert!(diagnostic.contains("exception=ReferenceError, module=import#1, line=4, column=3"));
    assert!(diagnostic.contains("quickjs=Exception"));
}

#[test]
fn classification_and_clones_keep_the_original_cause() {
    let source = io::Error::new(io::ErrorKind::PermissionDenied, "synthetic-secret");
    let error = ScriptError::with_source(ScriptErrorKind::JavaScript, source);
    let copy = error.clone().reclassify(ScriptErrorKind::Module);
    assert_eq!(error.kind(), ScriptErrorKind::JavaScript);
    assert_eq!(copy.kind(), ScriptErrorKind::Module);
    assert!(std::ptr::eq(
        error.source().unwrap(),
        copy.source().unwrap()
    ));
    let source = copy.source().unwrap().downcast_ref::<io::Error>().unwrap();
    assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(source.to_string(), "synthetic-secret");
}

#[test]
fn synthetic_policy_errors_have_no_fabricated_source() {
    for kind in [
        ScriptErrorKind::Cancelled,
        ScriptErrorKind::Deadline,
        ScriptErrorKind::Busy,
    ] {
        let error = ScriptError::new(kind);
        assert_eq!(error.kind(), kind);
        assert!(error.source().is_none());
    }
}

#[derive(Debug)]
struct Untrusted;

impl fmt::Display for Untrusted {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result { Err(fmt::Error) }
}

impl Error for Untrusted {}

#[test]
fn safe_renderers_use_only_typed_metadata() {
    let errors = [
        ScriptError::with_source(
            ScriptErrorKind::Module,
            io::Error::new(io::ErrorKind::NotFound, "synthetic-secret"),
        ),
        ScriptError::with_source(
            ScriptErrorKind::Module,
            rquickjs::Error::new_resolving_message(
                "synthetic-secret",
                "synthetic-secret",
                "synthetic-secret",
            ),
        ),
        ScriptError::with_source(
            ScriptErrorKind::JavaScript,
            rquickjs::Error::new_from_js_message("string", "number", "synthetic-secret"),
        ),
        ScriptError::with_source(
            ScriptErrorKind::Json,
            serde_json::from_str::<serde_json::Value>("{synthetic-secret").unwrap_err(),
        ),
        ScriptError::with_source(ScriptErrorKind::Thread, ThreadFailure::Panic),
        ScriptError::with_source(ScriptErrorKind::Thread, Untrusted),
    ];
    for error in errors {
        for rendered in [
            error.to_string(),
            format!("{error:?}"),
            error.diagnostic().to_string(),
        ] {
            assert!(!rendered.contains("synthetic-secret"));
        }
    }
    let error = ScriptError::with_source(ScriptErrorKind::Module, io::Error::from_raw_os_error(2));
    let diagnostic = error.diagnostic().to_string();
    assert!(diagnostic.contains("NotFound"));
    assert!(diagnostic.contains("Some(2)"));
}

#[test]
fn owned_error_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ScriptError>();
}

#[test]
fn safe_diagnostics_include_nested_types_without_source_text() {
    #[derive(Debug, thiserror::Error)]
    #[error("private wrapper text")]
    struct Wrapped {
        #[source]
        source: io::Error,
    }
    let error = ScriptError::with_source(
        ScriptErrorKind::Module,
        Wrapped {
            source: io::Error::new(io::ErrorKind::PermissionDenied, "private IO text"),
        },
    );
    assert!(error.source().unwrap().source().unwrap().is::<io::Error>());
    let diagnostic = error.diagnostic().to_string();
    assert!(diagnostic.contains("PermissionDenied"));
    assert!(!diagnostic.contains("private wrapper text"));
    assert!(!diagnostic.contains("private IO text"));
}

#[test]
fn cyclic_cause_chain_has_a_bounded_report() {
    #[derive(Debug)]
    struct Cycle;

    impl fmt::Display for Cycle {
        fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result { Err(fmt::Error) }
    }

    impl Error for Cycle {
        fn source(&self) -> Option<&(dyn Error + 'static)> { Some(self) }
    }
    let error = ScriptError::with_source(ScriptErrorKind::Thread, Cycle);
    assert!(
        error
            .diagnostic()
            .to_string()
            .contains("further causes omitted")
    );
}
