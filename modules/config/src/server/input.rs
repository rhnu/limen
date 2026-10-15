use serde::Deserialize;
use serde_saphyr::{Location, Spanned};

use crate::ConfigError;

/// One schema uses each parser's native spans without retaining configuration text.
pub(super) struct Input<T, const TOML: bool> {
    pub value: T,
    yaml: Location,
    offset: Option<usize>,
}

impl<T, const TOML: bool> Input<T, TOML> {
    pub fn unlocated(value: T) -> Self {
        Self {
            value,
            yaml: Location::UNKNOWN,
            offset: None,
        }
    }

    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> Input<U, TOML> {
        Input {
            value: map(self.value),
            yaml: self.yaml,
            offset: self.offset,
        }
    }

    pub fn locate(&self, error: ConfigError, source: &str) -> ConfigError {
        if let Some(offset) = self.offset {
            error.at_offset(source, offset)
        } else {
            error.at(self.yaml)
        }
    }
}

impl<'de, T: Deserialize<'de>, const TOML: bool> Deserialize<'de> for Input<T, TOML> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if TOML {
            let value = toml::Spanned::<T>::deserialize(deserializer)?;
            let offset = Some(value.span().start);
            Ok(Self {
                value: value.into_inner(),
                yaml: Location::UNKNOWN,
                offset,
            })
        } else {
            let value = Spanned::<T>::deserialize(deserializer)?;
            Ok(Self {
                value: value.value,
                yaml: value.referenced,
                offset: None,
            })
        }
    }
}
