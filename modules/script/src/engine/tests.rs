use std::{cell::Cell, error::Error, io};

use super::{Budget, conversion_error, fatal, module_error};
use crate::{ScriptError, ScriptErrorKind};

#[test]
fn context_reclassification_preserves_owned_causes_and_resource_failures() {
    for (convert, expected) in [
        (
            module_error as fn(ScriptError) -> ScriptError,
            ScriptErrorKind::Module,
        ),
        (conversion_error, ScriptErrorKind::Json),
    ] {
        let error = convert(ScriptError::with_source(
            ScriptErrorKind::JavaScript,
            rquickjs::Error::Exception,
        ));
        assert_eq!(error.kind(), expected);
        assert!(matches!(
            error.source().unwrap().downcast_ref::<rquickjs::Error>(),
            Some(rquickjs::Error::Exception)
        ));
        let error = convert(ScriptError::new(ScriptErrorKind::MemoryLimit));
        assert_eq!(error.kind(), ScriptErrorKind::MemoryLimit);
    }
}

#[test]
fn fatal_classification_uses_policy_kind() {
    for kind in [
        ScriptErrorKind::MemoryLimit,
        ScriptErrorKind::ExecutionBudget,
    ] {
        assert!(fatal(&ScriptError::new(kind)));
    }
    for kind in [
        ScriptErrorKind::JavaScript,
        ScriptErrorKind::Json,
        ScriptErrorKind::Cancelled,
        ScriptErrorKind::Deadline,
    ] {
        assert!(!fatal(&ScriptError::new(kind)));
    }
}

#[test]
fn budget_policy_keeps_the_failed_operation_cause() {
    let budget = Budget {
        deadline: Cell::new(None),
        interrupted: Cell::new(true),
    };
    let error = ScriptError::with_source(
        ScriptErrorKind::JavaScript,
        io::Error::from(io::ErrorKind::Interrupted),
    );
    let error = budget.check_result::<()>(Err(error)).unwrap_err();
    assert_eq!(error.kind(), ScriptErrorKind::ExecutionBudget);
    assert_eq!(
        error
            .source()
            .unwrap()
            .downcast_ref::<io::Error>()
            .unwrap()
            .kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(
        budget.check_result(Ok(1)).unwrap_err().kind(),
        ScriptErrorKind::ExecutionBudget
    );
}
