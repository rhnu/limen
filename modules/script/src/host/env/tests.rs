use std::{env::VarError, error::Error, ffi::OsString};

use super::{ReadError, read_with};

#[test]
fn distinguishes_missing_empty_and_present_values_without_trimming() {
    assert_eq!(
        read_with("TOKEN", 16, false, |_| Err(VarError::NotPresent)).unwrap(),
        None
    );
    for value in ["", " token\n", "密钥"] {
        assert_eq!(
            read_with("TOKEN", 16, false, |_| Ok(value.into())).unwrap(),
            Some(value.into())
        );
    }
}

#[test]
fn required_values_reject_missing_and_empty_without_trimming() {
    for value in [Err(VarError::NotPresent), Ok(String::new())] {
        assert!(matches!(
            read_with("TOKEN", 16, true, |_| value),
            Err(ReadError::RequiredValue)
        ));
    }
    for value in ["token", " token\n", " \t", "密钥"] {
        assert_eq!(
            read_with("TOKEN", 16, true, |_| Ok(value.into())).unwrap(),
            Some(value.into())
        );
    }
}

#[test]
fn rejects_invalid_names_before_calling_the_os() {
    for required in [false, true] {
        for name in ["", "A=B", "A\0B"] {
            let error = read_with(name, 16, required, |_| {
                panic!("invalid name reached lookup")
            })
            .unwrap_err();
            assert!(matches!(error, ReadError::InvalidName));
        }
        assert!(matches!(
            read_with("LONG_NAME", 4, required, |_| panic!(
                "oversized name reached lookup"
            )),
            Err(ReadError::NameLimit)
        ));
    }
}

#[test]
fn bounds_names_and_values_by_utf8_bytes() {
    for required in [false, true] {
        assert_eq!(
            read_with("密钥", 6, required, |name| {
                assert_eq!(name, "密钥");
                Ok("令牌".into())
            })
            .unwrap(),
            Some("令牌".into())
        );
        assert!(matches!(
            read_with("密钥", 5, required, |_| panic!(
                "oversized name reached lookup"
            )),
            Err(ReadError::NameLimit)
        ));
        assert!(matches!(
            read_with("TOKEN", 5, required, |_| Ok("令牌".into())),
            Err(ReadError::ValueLimit)
        ));
    }
}

#[test]
fn retains_non_unicode_cause_without_exposing_value_in_message() {
    for required in [false, true] {
        let error = read_with("TOKEN", 128, required, |_| {
            Err(VarError::NotUnicode(OsString::from("synthetic-secret")))
        })
        .unwrap_err();
        assert!(matches!(error, ReadError::NotUnicode(_)));
        assert!(error.source().unwrap().is::<VarError>());
        assert!(!error.to_string().contains("synthetic-secret"));
    }
}
