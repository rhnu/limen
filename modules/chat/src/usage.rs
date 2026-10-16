use serde_json::{Value, json};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Completeness and trustworthiness of the upstream's explicit token report.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UsageQuality {
    #[default]
    Missing,
    Partial,
    Reported,
    Invalid,
    Conflicting,
}

impl UsageQuality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Partial => "partial",
            Self::Reported => "reported",
            Self::Invalid => "invalid",
            Self::Conflicting => "conflicting",
        }
    }
}

/// Owned counts reported by upstream, without estimation or transport completion claims.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UsageStats {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
    pub quality: UsageQuality,
}

impl UsageStats {
    /// Builds the fixed, bounded usage shape; the gateway supplies source and completion facts.
    pub fn to_json(self) -> Value {
        json!({
            "inputTokens": self.input_tokens,
            "outputTokens": self.output_tokens,
            "totalTokens": self.total_tokens,
            "cachedInputTokens": self.cached_input_tokens,
            "reasoningOutputTokens": self.reasoning_output_tokens,
            "quality": self.quality.as_str(),
        })
    }
}

/// Tracks five explicit Chat usage counts in constant space, independent of request success.
#[derive(Default)]
pub struct UsageTracker {
    counts: [Option<u64>; 5],
    invalid: bool,
    conflicting: bool,
}

impl UsageTracker {
    /// Observes a response's `usage` field. Missing and null values never erase prior counts.
    ///
    /// Reports are cumulative: identical values are accepted, distinct non-null values conflict.
    /// Malformed reports affect usage quality only and never return a request error.
    pub fn observe(&mut self, usage: Option<&Value>) {
        let Some(usage) = usage.filter(|value| !value.is_null()) else {
            return;
        };
        let Some(usage) = usage.as_object() else {
            self.invalid = true;
            return;
        };
        let reported = [
            count(usage.get("prompt_tokens")),
            count(usage.get("completion_tokens")),
            count(usage.get("total_tokens")),
            detail_count(usage.get("prompt_tokens_details"), "cached_tokens"),
            detail_count(usage.get("completion_tokens_details"), "reasoning_tokens"),
        ];
        for (previous, reported) in self.counts.iter_mut().zip(reported) {
            match reported {
                Ok(Some(value)) => {
                    if previous.is_some_and(|previous| previous != value) {
                        self.conflicting = true;
                    } else {
                        *previous = Some(value);
                    }
                },
                Ok(None) => {},
                Err(()) => self.invalid = true,
            }
        }
    }

    /// Returns a detached snapshot. Invalid input takes precedence over conflicting reports.
    /// Neither invalid nor conflicting reports expose any effective counts.
    pub fn snapshot(&self) -> UsageStats {
        let quality = if self.invalid {
            UsageQuality::Invalid
        } else if self.conflicting {
            UsageQuality::Conflicting
        } else if self.counts[..3].iter().all(Option::is_some) {
            UsageQuality::Reported
        } else if self.counts.iter().any(Option::is_some) {
            UsageQuality::Partial
        } else {
            UsageQuality::Missing
        };
        if matches!(quality, UsageQuality::Invalid | UsageQuality::Conflicting) {
            return UsageStats {
                quality,
                ..UsageStats::default()
            };
        }
        let [
            input_tokens,
            output_tokens,
            total_tokens,
            cached_input_tokens,
            reasoning_output_tokens,
        ] = self.counts;
        UsageStats {
            input_tokens,
            output_tokens,
            total_tokens,
            cached_input_tokens,
            reasoning_output_tokens,
            quality,
        }
    }
}

fn detail_count(details: Option<&Value>, key: &str) -> Result<Option<u64>, ()> {
    let Some(details) = details.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let details = details.as_object().ok_or(())?;
    count(details.get(key))
}

fn count(value: Option<&Value>) -> Result<Option<u64>, ()> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    if let Some(value) = value.as_u64() {
        return (value <= MAX_SAFE_INTEGER).then_some(Some(value)).ok_or(());
    }
    let number = value.as_number().ok_or(())?;
    let value = number.as_f64().ok_or(())?;
    if !number.is_f64() || !(0.0..=9_007_199_254_740_991.0).contains(&value) || value.fract() != 0.0
    {
        return Err(());
    }
    // The inclusive safe-integer bound and integral check make this conversion exact.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "The finite, integral value is within the exact nonnegative safe-integer range"
    )]
    Ok(Some(value as u64))
}

#[cfg(test)]
mod tests;
