use super::{NativeContentError, Output, Result, synthetic::Origin};

/// Exact incremental grammar: whitespace, {, "text", :, JSON string, }, whitespace.
/// Retains lexical position, one partial escape and one UTF-16 high surrogate.
#[derive(Clone, Default)]
pub(super) struct Decoder {
    phase: u8,
    escape: bool,
    hex: Option<(u16, u8)>,
    high: Option<u16>,
    origin: Option<Origin>,
}

impl Decoder {
    pub(super) fn complete(&self) -> bool { self.phase == 11 }

    pub(super) fn pending_origin(&self) -> Option<Origin> { self.origin }

    pub(super) fn decode(
        &mut self,
        input: &str,
        origin: Origin,
        output: &mut Output<'_>,
    ) -> Result<Vec<(Origin, String)>> {
        let mut text = Vec::new();
        for (offset, ch) in input.chars().enumerate() {
            if offset % 1024 == 0 {
                output.check()?;
            }
            if self.phase != 9 {
                if matches!(self.phase, 0 | 1 | 7 | 8 | 10 | 11)
                    && matches!(ch, ' ' | '\t' | '\r' | '\n')
                {
                    continue;
                }
                if self.phase < 9
                    && ch == ['{', '"', 't', 'e', 'x', 't', '"', ':', '"'][usize::from(self.phase)]
                {
                    self.phase += 1;
                } else if self.phase == 10 && ch == '}' {
                    self.phase = 11;
                } else {
                    return Err(NativeContentError::InvalidInput);
                }
            } else if let Some((number, digits)) = self.hex {
                let digit = ch
                    .to_digit(16)
                    .filter(|_| ch.is_ascii())
                    .ok_or(NativeContentError::InvalidInput)?;
                let number = (number << 4) | u16::try_from(digit)?;
                if digits == 3 {
                    self.hex = None;
                    self.emit(number, &mut text, output)?;
                } else {
                    self.hex = Some((number, digits + 1));
                }
            } else if self.escape {
                self.escape = false;
                if ch == 'u' {
                    self.hex = Some((0, 0));
                } else {
                    let unit = match ch {
                        '"' => '"',
                        '\\' => '\\',
                        '/' => '/',
                        'b' => '\u{8}',
                        'f' => '\u{c}',
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        _ => return Err(NativeContentError::InvalidInput),
                    };
                    self.emit(unit as u16, &mut text, output)?;
                }
            } else if ch == '\\' {
                self.origin.get_or_insert(origin);
                self.escape = true;
            } else if ch == '"' {
                if self.high.is_some() {
                    return Err(NativeContentError::InvalidInput);
                }
                self.phase = 10;
            } else {
                if ch < ' ' || self.high.is_some() {
                    return Err(NativeContentError::InvalidInput);
                }
                append(&mut text, origin, ch, output)?;
            }
        }
        Ok(text)
    }

    fn emit(
        &mut self,
        unit: u16,
        text: &mut Vec<(Origin, String)>,
        output: &Output<'_>,
    ) -> Result<()> {
        if let Some(high) = self.high.take() {
            if !(0xDC00..=0xDFFF).contains(&unit) {
                return Err(NativeContentError::InvalidInput);
            }
            let code = 0x10000 + ((u32::from(high) - 0xD800) << 10) + u32::from(unit) - 0xDC00;
            append(
                text,
                self.origin.take().ok_or(NativeContentError::InvalidInput)?,
                char::from_u32(code).ok_or(NativeContentError::InvalidInput)?,
                output,
            )?;
        } else if (0xD800..=0xDBFF).contains(&unit) {
            self.high = Some(unit);
        } else {
            let ch = char::from_u32(u32::from(unit)).ok_or(NativeContentError::InvalidInput)?;
            append(
                text,
                self.origin.take().ok_or(NativeContentError::InvalidInput)?,
                ch,
                output,
            )?;
        }
        Ok(())
    }
}

fn append(
    text: &mut Vec<(Origin, String)>,
    origin: Origin,
    ch: char,
    output: &Output<'_>,
) -> Result<()> {
    let bytes = text.iter().fold(0_usize, |sum, (_, fragment)| {
        sum.saturating_add(fragment.len())
    });
    if bytes.saturating_add(ch.len_utf8())
        > output.limits.max_output_bytes.saturating_sub(output.bytes)
    {
        return Err(NativeContentError::Limit);
    }
    if text.last().is_none_or(|(last, _)| *last != origin) {
        text.push((origin, String::new()));
    }
    let (_, fragment) = text.last_mut().ok_or(NativeContentError::InvalidInput)?;
    output.append_char(fragment, ch)
}
