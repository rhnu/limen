use super::{ExceptionDiagnostic, ExceptionKind, Location, locate};

#[test]
fn locates_runtime_and_parser_frames_in_registered_modules() {
    let modules = vec![
        "/private/gateway.js".into(),
        "/private/shared/module.mjs".into(),
    ];
    for (stack, expected) in [
        (
            "    at check (/private/gateway.js:4:17)\n",
            Location {
                module: 0,
                line: 4,
                column: Some(17),
            },
        ),
        (
            "    at /private/shared/module.mjs:2:9\n",
            Location {
                module: 1,
                line: 2,
                column: Some(9),
            },
        ),
        (
            "    at check (/private/gateway.js:8)\n",
            Location {
                module: 0,
                line: 8,
                column: None,
            },
        ),
        (
            "    at check (/private/gateway.js:8:0)\n",
            Location {
                module: 0,
                line: 8,
                column: None,
            },
        ),
    ] {
        assert_eq!(locate(stack, &modules), Some(expected));
    }
}

#[test]
fn accepts_windows_colons_unicode_and_spaces_without_exposing_paths() {
    let modules = vec![r"D:\private secret\配置.js".into()];
    let location = locate("    at f (D:\\private secret\\配置.js:12:3)\n", &modules).unwrap();
    let rendered = ExceptionDiagnostic {
        kind: ExceptionKind::TypeError,
        location: Some(location),
    }
    .to_string();
    assert_eq!(
        rendered,
        "exception=TypeError, module=gateway.js, line=12, column=3"
    );
}

#[test]
fn skips_native_and_unregistered_frames_before_selecting_a_location() {
    let modules = vec!["/private/gateway.js".into()];
    let stack = "    at native (native)\n    at f (/unknown/secret.js:1:2)\n    at f (/private/gateway.js:9:4)\n";
    assert_eq!(
        locate(stack, &modules),
        Some(Location {
            module: 0,
            line: 9,
            column: Some(4)
        })
    );
}

#[test]
fn rejects_unknown_paths_invalid_coordinates_and_oversized_stacks() {
    let modules = vec!["/private/gateway.js".into()];
    for stack in [
        "secret /private/gateway.js:4:2",
        "    at f (/private/gateway.js:0:2)",
        "    at f (/private/gateway.js:4294967296:2)",
        "    at f (/private/gateway.js:4:4294967296)",
        "    at f (/private/gateway.js:4:secret)",
        "    at f (/other/private/gateway.js:4:2)",
    ] {
        assert_eq!(locate(stack, &modules), None);
    }
    assert_eq!(
        locate(
            &format!("{}\n    at /private/gateway.js:4:2", "x".repeat(16_384)),
            &modules
        ),
        None
    );
}

#[test]
fn class_names_and_import_labels_cannot_copy_untrusted_text() {
    let diagnostic = ExceptionDiagnostic {
        kind: ExceptionKind::from_name("TypeError; synthetic-secret"),
        location: Some(Location {
            module: 2,
            line: 4,
            column: Some(7),
        }),
    };
    assert_eq!(
        diagnostic.to_string(),
        "exception=ThrownValue, module=import#2, line=4, column=7"
    );
}
