use std::time::Duration;

use serde_json::json;

use super::*;

#[test]
fn completion_rechecks_cancellation_and_deadline_before_delivering_a_result() {
    for (cancelled, expired, expected) in [
        (false, false, None),
        (true, false, Some(ScriptErrorKind::Cancelled)),
        (false, true, Some(ScriptErrorKind::Deadline)),
        (true, true, Some(ScriptErrorKind::Cancelled)),
    ] {
        let cancellation = Cancellation::new();
        let (response, mut receive) = oneshot::channel();
        let mut completion = Completion {
            cancellation: cancellation.clone(),
            deadline: Instant::now() + Duration::from_secs(60),
            response,
        };
        if expired {
            completion.deadline = Instant::now();
        }
        if cancelled {
            cancellation.cancel();
        }
        completion.send(Ok(json!({"committed": true})));
        let result = receive.try_recv().unwrap();
        match expected {
            None => assert_eq!(result.unwrap(), json!({"committed": true})),
            Some(expected) => assert_eq!(result.unwrap_err().kind(), expected),
        }
    }
}

#[test]
fn closed_receiver_revokes_completion_and_live_receiver_keeps_hook_failure() {
    let (response, mut receive) = oneshot::channel();
    let completion = Completion {
        cancellation: Cancellation::new(),
        deadline: Instant::now() + Duration::from_secs(60),
        response,
    };
    receive.close();
    assert_eq!(
        completion.invalid().unwrap().kind(),
        ScriptErrorKind::Cancelled
    );
    completion.send(Ok(json!({"committed": true})));
    assert!(matches!(
        receive.try_recv(),
        Err(oneshot::error::TryRecvError::Closed)
    ));

    let (response, mut receive) = oneshot::channel();
    let completion = Completion {
        cancellation: Cancellation::new(),
        deadline: Instant::now() + Duration::from_secs(60),
        response,
    };
    completion.send(Err(ScriptError::new(ScriptErrorKind::Contract)));
    assert_eq!(
        receive.try_recv().unwrap().unwrap_err().kind(),
        ScriptErrorKind::Contract
    );
}
