//! Deterministic bounded text notation for exact graph-literal authoring.
//!
//! This is an editor notation, not a saved-document or transport format.
//! Canonical `ALGR`/`ALGW` bytes remain the persistence authority.

use core::fmt;

use hyperreal::Rational;

use super::{
    GraphSchema, GraphSchemaError, GraphTypeId, GraphValue, RecordValueField, TypeKind,
    TypedGraphValue,
};

/// First-release source/output ceiling for one interactive literal draft.
///
/// This admits the complete one-MiB graph byte-literal ceiling as lowercase
/// hexadecimal plus its `hex""` delimiters. Larger composite textual
/// expansions remain representable in canonical graph bytes but are not
/// admitted into an interactive text field.
pub const INTERACTIVE_GRAPH_LITERAL_TEXT_BYTES: usize = 2 * 1_024 * 1_024 + 6;

/// Caller-owned allocation policy for one exact graph-literal text operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphLiteralTextLimits {
    maximum_bytes: usize,
}

impl GraphLiteralTextLimits {
    /// Construct a text policy. Zero is rejected when an operation begins.
    pub const fn new(maximum_bytes: usize) -> Self {
        Self { maximum_bytes }
    }

    /// Return the bounded first-release browser/editor policy.
    pub const fn interactive() -> Self {
        Self::new(INTERACTIVE_GRAPH_LITERAL_TEXT_BYTES)
    }

    /// Return the maximum UTF-8 source or formatted byte length.
    pub const fn maximum_bytes(self) -> usize {
        self.maximum_bytes
    }

    fn validate(self) -> Result<(), GraphLiteralTextError> {
        if self.maximum_bytes == 0 {
            Err(GraphLiteralTextError::ZeroLimit)
        } else {
            Ok(())
        }
    }
}

impl Default for GraphLiteralTextLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

/// Rejection at the bounded exact graph-literal editor boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphLiteralTextError {
    /// The caller supplied a zero source/output ceiling.
    ZeroLimit,
    /// Input was larger than caller policy before parsing began.
    InputTooLarge {
        /// Observed UTF-8 bytes.
        actual: usize,
        /// Caller-owned ceiling.
        maximum: usize,
    },
    /// Deterministic formatting exceeded caller policy.
    OutputTooLarge {
        /// Caller-owned ceiling.
        maximum: usize,
    },
    /// Syntax contradicted the registered type at one UTF-8 byte offset.
    Syntax {
        /// Source byte offset at or immediately after the contradiction.
        offset: usize,
        /// Bounded diagnostic description.
        message: String,
    },
    /// An exact-rational token was malformed.
    InvalidRational(GraphTypeId),
    /// A signed canonical-integer token was malformed.
    InvalidSignedInteger(GraphTypeId),
    /// An unsigned canonical-integer token was malformed.
    InvalidUnsignedInteger(GraphTypeId),
    /// A byte literal was not an even sequence of hexadecimal digits.
    InvalidHex(GraphTypeId),
    /// Identity-bearing values require an authenticated selector, not text.
    IdentityRequiresSelector(GraphTypeId),
    /// Runtime Event/Stream types have no literal notation.
    RuntimeOnly(GraphTypeId),
    /// The parsed or formatted value contradicted the exact schema.
    Schema(GraphSchemaError),
}

impl fmt::Display for GraphLiteralTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => formatter.write_str("graph literal text limit is zero"),
            Self::InputTooLarge { actual, maximum } => write!(
                formatter,
                "graph literal text has {actual} bytes; policy admits at most {maximum}"
            ),
            Self::OutputTooLarge { maximum } => write!(
                formatter,
                "formatted graph literal exceeds the {maximum}-byte editor policy"
            ),
            Self::Syntax { offset, message } => {
                write!(
                    formatter,
                    "graph literal syntax at byte {offset}: {message}"
                )
            }
            Self::InvalidRational(value_type) => {
                write!(
                    formatter,
                    "type t{} requires an exact rational",
                    value_type.get()
                )
            }
            Self::InvalidSignedInteger(value_type) => write!(
                formatter,
                "type t{} requires a canonical signed decimal integer",
                value_type.get()
            ),
            Self::InvalidUnsignedInteger(value_type) => write!(
                formatter,
                "type t{} requires a canonical unsigned decimal integer",
                value_type.get()
            ),
            Self::InvalidHex(value_type) => write!(
                formatter,
                "type t{} requires hex\"...\" with complete byte pairs",
                value_type.get()
            ),
            Self::IdentityRequiresSelector(value_type) => write!(
                formatter,
                "identity-bearing type t{} requires an authenticated selector",
                value_type.get()
            ),
            Self::RuntimeOnly(value_type) => write!(
                formatter,
                "runtime type t{} has no graph-literal text",
                value_type.get()
            ),
            Self::Schema(error) => {
                write!(formatter, "graph literal failed schema validation: {error}")
            }
        }
    }
}

impl std::error::Error for GraphLiteralTextError {}

impl From<GraphSchemaError> for GraphLiteralTextError {
    fn from(error: GraphSchemaError) -> Self {
        Self::Schema(error)
    }
}

/// Format one complete exact literal into deterministic schema-directed text.
///
/// Scalars use their exact notation. Text is double-quoted with explicit
/// escapes, bytes use lowercase `hex"..."`, arrays use `[a,b]`, records use
/// canonical schema field order `{name:value}`, options use `none` or
/// `some(value)`, and results use `ok(value)` or `error(value)`. Resource/job
/// handles are intentionally rejected because authoring their bytes would not
/// prove capability or cache authority.
pub fn format_graph_literal_text(
    schema: &GraphSchema,
    value: &TypedGraphValue,
    limits: GraphLiteralTextLimits,
) -> Result<String, GraphLiteralTextError> {
    limits.validate()?;
    schema.validate_typed_value(value)?;
    reject_non_text_authority(schema, value.value_type())?;
    let mut output = BoundedText::new(limits.maximum_bytes());
    format_value(schema, value.value_type(), value.value(), &mut output)?;
    Ok(output.finish())
}

/// Parse one complete schema-directed exact literal without partial mutation.
///
/// Insignificant whitespace is accepted around structural tokens. A successful
/// parse is revalidated as one complete [`TypedGraphValue`]; callers can format
/// that value again to obtain deterministic editor text.
pub fn parse_graph_literal_text(
    schema: &GraphSchema,
    value_type: GraphTypeId,
    source: &str,
    limits: GraphLiteralTextLimits,
) -> Result<TypedGraphValue, GraphLiteralTextError> {
    limits.validate()?;
    reject_non_text_authority(schema, value_type)?;
    if source.len() > limits.maximum_bytes() {
        return Err(GraphLiteralTextError::InputTooLarge {
            actual: source.len(),
            maximum: limits.maximum_bytes(),
        });
    }
    let mut parser = LiteralParser {
        schema,
        source,
        offset: 0,
        nodes: 0,
    };
    let value = parser.parse_value(value_type, 1)?;
    parser.skip_whitespace();
    if parser.offset != source.len() {
        return Err(parser.syntax("unexpected trailing input"));
    }
    TypedGraphValue::try_new(schema, value_type, value).map_err(Into::into)
}

fn reject_non_text_authority(
    schema: &GraphSchema,
    value_type: GraphTypeId,
) -> Result<(), GraphLiteralTextError> {
    let definition = schema
        .value_type(value_type)
        .ok_or(GraphSchemaError::UnknownType(value_type))?;
    match definition.kind() {
        TypeKind::ResourceHandle { .. } | TypeKind::JobHandle => {
            Err(GraphLiteralTextError::IdentityRequiresSelector(value_type))
        }
        TypeKind::Event { .. } | TypeKind::Stream { .. } => {
            Err(GraphLiteralTextError::RuntimeOnly(value_type))
        }
        _ => Ok(()),
    }
}

struct BoundedText {
    value: String,
    maximum_bytes: usize,
}

impl BoundedText {
    fn new(maximum_bytes: usize) -> Self {
        Self {
            value: String::with_capacity(maximum_bytes.min(4_096)),
            maximum_bytes,
        }
    }

    fn push_str(&mut self, value: &str) -> Result<(), GraphLiteralTextError> {
        let length = self.value.len().checked_add(value.len()).ok_or(
            GraphLiteralTextError::OutputTooLarge {
                maximum: self.maximum_bytes,
            },
        )?;
        if length > self.maximum_bytes {
            return Err(GraphLiteralTextError::OutputTooLarge {
                maximum: self.maximum_bytes,
            });
        }
        self.value.push_str(value);
        Ok(())
    }

    fn push_char(&mut self, value: char) -> Result<(), GraphLiteralTextError> {
        let length = self.value.len().checked_add(value.len_utf8()).ok_or(
            GraphLiteralTextError::OutputTooLarge {
                maximum: self.maximum_bytes,
            },
        )?;
        if length > self.maximum_bytes {
            return Err(GraphLiteralTextError::OutputTooLarge {
                maximum: self.maximum_bytes,
            });
        }
        self.value.push(value);
        Ok(())
    }

    fn finish(self) -> String {
        self.value
    }
}

fn format_value(
    schema: &GraphSchema,
    value_type: GraphTypeId,
    value: &GraphValue,
    output: &mut BoundedText,
) -> Result<(), GraphLiteralTextError> {
    let definition = schema
        .value_type(value_type)
        .ok_or(GraphSchemaError::UnknownType(value_type))?;
    match (definition.kind(), value) {
        (TypeKind::Boolean, GraphValue::Boolean(value)) => {
            output.push_str(if *value { "true" } else { "false" })
        }
        (TypeKind::ExactRational { .. }, GraphValue::ExactRational(value)) => {
            output.push_str(&value.to_string())
        }
        (
            TypeKind::MeasurementInterval { .. },
            GraphValue::MeasurementInterval { lower, upper },
        ) => {
            output.push_str(&lower.to_string())?;
            output.push_str("..")?;
            output.push_str(&upper.to_string())
        }
        (TypeKind::CanonicalI64 { .. }, GraphValue::CanonicalI64(value)) => {
            output.push_str(&value.to_string())
        }
        (TypeKind::CanonicalU64 { .. }, GraphValue::CanonicalU64(value)) => {
            output.push_str(&value.to_string())
        }
        (TypeKind::Text { .. }, GraphValue::Text(value)) => format_text(value, output),
        (TypeKind::Bytes { .. }, GraphValue::Bytes(value)) => format_bytes(value, output),
        (TypeKind::Array { element, .. }, GraphValue::Array(values)) => {
            output.push_char('[')?;
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push_char(',')?;
                }
                format_value(schema, *element, value, output)?;
            }
            output.push_char(']')
        }
        (TypeKind::Record { fields }, GraphValue::Record(values)) => {
            output.push_char('{')?;
            for (index, (field, value)) in fields.iter().zip(values).enumerate() {
                if index != 0 {
                    output.push_char(',')?;
                }
                output.push_str(field.name())?;
                output.push_char(':')?;
                format_value(schema, field.value_type(), &value.value, output)?;
            }
            output.push_char('}')
        }
        (TypeKind::Option { .. }, GraphValue::OptionNone) => output.push_str("none"),
        (TypeKind::Option { value: inner }, GraphValue::OptionSome(value)) => {
            output.push_str("some(")?;
            format_value(schema, *inner, value, output)?;
            output.push_char(')')
        }
        (TypeKind::Result { ok, .. }, GraphValue::ResultOk(value)) => {
            output.push_str("ok(")?;
            format_value(schema, *ok, value, output)?;
            output.push_char(')')
        }
        (TypeKind::Result { error, .. }, GraphValue::ResultError(value)) => {
            output.push_str("error(")?;
            format_value(schema, *error, value, output)?;
            output.push_char(')')
        }
        (TypeKind::ResourceHandle { .. } | TypeKind::JobHandle, _) => {
            Err(GraphLiteralTextError::IdentityRequiresSelector(value_type))
        }
        (TypeKind::Event { .. } | TypeKind::Stream { .. }, _) => {
            Err(GraphLiteralTextError::RuntimeOnly(value_type))
        }
        _ => Err(GraphSchemaError::TypeMismatch {
            expected: value_type,
            received: value.kind(),
        }
        .into()),
    }
}

fn format_text(value: &str, output: &mut BoundedText) -> Result<(), GraphLiteralTextError> {
    output.push_char('"')?;
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\"")?,
            '\\' => output.push_str("\\\\")?,
            '\n' => output.push_str("\\n")?,
            '\r' => output.push_str("\\r")?,
            '\t' => output.push_str("\\t")?,
            '\0' => output.push_str("\\0")?,
            character if character.is_control() => {
                output.push_str(&format!("\\u{{{:x}}}", u32::from(character)))?;
            }
            character => output.push_char(character)?,
        }
    }
    output.push_char('"')
}

fn format_bytes(value: &[u8], output: &mut BoundedText) -> Result<(), GraphLiteralTextError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    output.push_str("hex\"")?;
    for byte in value {
        output.push_char(char::from(HEX[usize::from(byte >> 4)]))?;
        output.push_char(char::from(HEX[usize::from(byte & 0x0f)]))?;
    }
    output.push_char('"')
}

struct LiteralParser<'a> {
    schema: &'a GraphSchema,
    source: &'a str,
    offset: usize,
    nodes: usize,
}

impl LiteralParser<'_> {
    fn parse_value(
        &mut self,
        value_type: GraphTypeId,
        depth: usize,
    ) -> Result<GraphValue, GraphLiteralTextError> {
        if depth > self.schema.limits().maximum_value_depth {
            return Err(GraphSchemaError::ValueDepthExceeded.into());
        }
        self.nodes = self
            .nodes
            .checked_add(1)
            .ok_or(GraphSchemaError::ValueNodeLimitExceeded)?;
        if self.nodes > self.schema.limits().maximum_value_nodes {
            return Err(GraphSchemaError::ValueNodeLimitExceeded.into());
        }
        let definition = self
            .schema
            .value_type(value_type)
            .ok_or(GraphSchemaError::UnknownType(value_type))?;
        match definition.kind() {
            TypeKind::Boolean => match self.take_scalar_token() {
                "true" => Ok(GraphValue::Boolean(true)),
                "false" => Ok(GraphValue::Boolean(false)),
                _ => Err(self.syntax("expected true or false")),
            },
            TypeKind::ExactRational { .. } => self
                .take_scalar_token()
                .parse::<Rational>()
                .map(GraphValue::ExactRational)
                .map_err(|_| GraphLiteralTextError::InvalidRational(value_type)),
            TypeKind::MeasurementInterval { .. } => {
                let (lower, upper) = self.take_interval_tokens()?;
                Ok(GraphValue::MeasurementInterval {
                    lower: lower
                        .parse::<Rational>()
                        .map_err(|_| GraphLiteralTextError::InvalidRational(value_type))?,
                    upper: upper
                        .parse::<Rational>()
                        .map_err(|_| GraphLiteralTextError::InvalidRational(value_type))?,
                })
            }
            TypeKind::CanonicalI64 { .. } => self
                .take_scalar_token()
                .parse::<i64>()
                .map(GraphValue::CanonicalI64)
                .map_err(|_| GraphLiteralTextError::InvalidSignedInteger(value_type)),
            TypeKind::CanonicalU64 { .. } => self
                .take_scalar_token()
                .parse::<u64>()
                .map(GraphValue::CanonicalU64)
                .map_err(|_| GraphLiteralTextError::InvalidUnsignedInteger(value_type)),
            TypeKind::Text { .. } => self.parse_text().map(GraphValue::Text),
            TypeKind::Bytes { .. } => self.parse_bytes(value_type).map(GraphValue::Bytes),
            TypeKind::Array {
                element,
                maximum_items,
            } => self.parse_array(*element, *maximum_items, depth),
            TypeKind::Record { fields } => self.parse_record(fields, depth),
            TypeKind::Option { value } => self.parse_option(*value, depth),
            TypeKind::Result { ok, error } => self.parse_result(*ok, *error, depth),
            TypeKind::ResourceHandle { .. } | TypeKind::JobHandle => {
                Err(GraphLiteralTextError::IdentityRequiresSelector(value_type))
            }
            TypeKind::Event { .. } | TypeKind::Stream { .. } => {
                Err(GraphLiteralTextError::RuntimeOnly(value_type))
            }
        }
    }

    fn parse_array(
        &mut self,
        element: GraphTypeId,
        maximum_items: u32,
        depth: usize,
    ) -> Result<GraphValue, GraphLiteralTextError> {
        self.expect_char('[')?;
        self.skip_whitespace();
        let mut values = Vec::new();
        if self.consume_char(']') {
            return Ok(GraphValue::Array(values));
        }
        loop {
            if values.len() >= maximum_items as usize
                || values.len() >= self.schema.limits().maximum_array_items
            {
                return Err(GraphSchemaError::LimitExceeded("array literal").into());
            }
            values.push(self.parse_value(element, depth + 1)?);
            self.skip_whitespace();
            if self.consume_char(']') {
                break;
            }
            self.expect_char(',')?;
        }
        Ok(GraphValue::Array(values))
    }

    fn parse_record(
        &mut self,
        fields: &[super::RecordField],
        depth: usize,
    ) -> Result<GraphValue, GraphLiteralTextError> {
        self.expect_char('{')?;
        self.skip_whitespace();
        let mut values = Vec::with_capacity(fields.len());
        for (index, field) in fields.iter().enumerate() {
            if index != 0 {
                self.expect_char(',')?;
            }
            let name = self.take_record_field_name()?;
            if name != field.name() {
                return Err(self.syntax(&format!(
                    "expected canonical field {} [f{}]",
                    field.name(),
                    field.id().get()
                )));
            }
            self.expect_char(':')?;
            values.push(RecordValueField {
                field: field.id(),
                value: self.parse_value(field.value_type(), depth + 1)?,
            });
        }
        self.expect_char('}')?;
        Ok(GraphValue::Record(values))
    }

    fn parse_option(
        &mut self,
        inner: GraphTypeId,
        depth: usize,
    ) -> Result<GraphValue, GraphLiteralTextError> {
        self.skip_whitespace();
        if self.remaining().starts_with("none") {
            self.offset += 4;
            self.require_value_boundary()?;
            return Ok(GraphValue::OptionNone);
        }
        self.expect_keyword("some")?;
        self.expect_char('(')?;
        let value = self.parse_value(inner, depth + 1)?;
        self.expect_char(')')?;
        Ok(GraphValue::OptionSome(Box::new(value)))
    }

    fn parse_result(
        &mut self,
        ok: GraphTypeId,
        error: GraphTypeId,
        depth: usize,
    ) -> Result<GraphValue, GraphLiteralTextError> {
        self.skip_whitespace();
        let (inner, success) = if self.remaining().starts_with("ok") {
            self.offset += 2;
            (ok, true)
        } else if self.remaining().starts_with("error") {
            self.offset += 5;
            (error, false)
        } else {
            return Err(self.syntax("expected ok(value) or error(value)"));
        };
        self.expect_char('(')?;
        let value = Box::new(self.parse_value(inner, depth + 1)?);
        self.expect_char(')')?;
        if success {
            Ok(GraphValue::ResultOk(value))
        } else {
            Ok(GraphValue::ResultError(value))
        }
    }

    fn parse_text(&mut self) -> Result<String, GraphLiteralTextError> {
        self.expect_char('"')?;
        let mut value = String::new();
        loop {
            let character = self
                .next_char()
                .ok_or_else(|| self.syntax("unterminated quoted text"))?;
            match character {
                '"' => return Ok(value),
                '\\' => value.push(self.parse_escape()?),
                character if character.is_control() => {
                    return Err(self.syntax("raw control character must be escaped"));
                }
                character => value.push(character),
            }
        }
    }

    fn parse_escape(&mut self) -> Result<char, GraphLiteralTextError> {
        let escaped = self
            .next_char()
            .ok_or_else(|| self.syntax("unterminated text escape"))?;
        match escaped {
            '"' => Ok('"'),
            '\\' => Ok('\\'),
            'n' => Ok('\n'),
            'r' => Ok('\r'),
            't' => Ok('\t'),
            '0' => Ok('\0'),
            'u' => self.parse_unicode_escape(),
            _ => Err(self.syntax("unknown text escape")),
        }
    }

    fn parse_unicode_escape(&mut self) -> Result<char, GraphLiteralTextError> {
        if self.next_char() != Some('{') {
            return Err(self.syntax("Unicode escape must begin with \\u{"));
        }
        let start = self.offset;
        let mut digits = 0_usize;
        while let Some(character) = self.peek_char() {
            if character == '}' {
                break;
            }
            if !character.is_ascii_hexdigit() || digits == 6 {
                return Err(self.syntax("Unicode escape must contain one to six hex digits"));
            }
            self.next_char();
            digits += 1;
        }
        if digits == 0 || self.next_char() != Some('}') {
            return Err(self.syntax("unterminated Unicode escape"));
        }
        let scalar = u32::from_str_radix(&self.source[start..self.offset - 1], 16)
            .map_err(|_| self.syntax("invalid Unicode escape"))?;
        char::from_u32(scalar).ok_or_else(|| self.syntax("Unicode escape is not a scalar value"))
    }

    fn parse_bytes(&mut self, value_type: GraphTypeId) -> Result<Vec<u8>, GraphLiteralTextError> {
        self.expect_keyword("hex")?;
        self.skip_whitespace();
        if self.next_char() != Some('"') {
            return Err(GraphLiteralTextError::InvalidHex(value_type));
        }
        let start = self.offset;
        while let Some(character) = self.peek_char() {
            if character == '"' {
                break;
            }
            if !character.is_ascii_hexdigit() {
                return Err(GraphLiteralTextError::InvalidHex(value_type));
            }
            self.next_char();
        }
        let end = self.offset;
        if self.next_char() != Some('"') || !(end - start).is_multiple_of(2) {
            return Err(GraphLiteralTextError::InvalidHex(value_type));
        }
        let source = self.source.as_bytes();
        let mut bytes = Vec::with_capacity((end - start) / 2);
        let mut offset = start;
        while offset < end {
            let high =
                hex_nibble(source[offset]).ok_or(GraphLiteralTextError::InvalidHex(value_type))?;
            let low = hex_nibble(source[offset + 1])
                .ok_or(GraphLiteralTextError::InvalidHex(value_type))?;
            bytes.push((high << 4) | low);
            offset += 2;
        }
        Ok(bytes)
    }

    fn take_interval_tokens(&mut self) -> Result<(&str, &str), GraphLiteralTextError> {
        self.skip_whitespace();
        let start = self.offset;
        let mut separator = None;
        for (relative, character) in self.remaining().char_indices() {
            if is_value_terminator(character) {
                break;
            }
            if self.remaining()[relative..].starts_with("..") {
                separator = Some(start + relative);
                break;
            }
        }
        let separator = separator.ok_or_else(|| self.syntax("interval requires lower..upper"))?;
        let lower = self.source[start..separator].trim();
        if lower.is_empty() {
            return Err(self.syntax("interval lower endpoint cannot be empty"));
        }
        self.offset = separator + 2;
        let upper = self.take_scalar_token();
        Ok((lower, upper))
    }

    fn take_scalar_token(&mut self) -> &str {
        self.skip_whitespace();
        let start = self.offset;
        for (relative, character) in self.remaining().char_indices() {
            if is_value_terminator(character) {
                self.offset = start + relative;
                return self.source[start..self.offset].trim();
            }
        }
        self.offset = self.source.len();
        self.source[start..].trim()
    }

    fn take_record_field_name(&mut self) -> Result<&str, GraphLiteralTextError> {
        self.skip_whitespace();
        let start = self.offset;
        for (relative, character) in self.remaining().char_indices() {
            if character == ':' {
                self.offset = start + relative;
                let name = self.source[start..self.offset].trim();
                if name.is_empty() {
                    return Err(self.syntax("record field name is empty"));
                }
                return Ok(name);
            }
            if matches!(character, ',' | '}') {
                break;
            }
        }
        Err(self.syntax("record field requires name:value"))
    }

    fn expect_keyword(&mut self, keyword: &str) -> Result<(), GraphLiteralTextError> {
        self.skip_whitespace();
        if self.remaining().starts_with(keyword) {
            self.offset += keyword.len();
            Ok(())
        } else {
            Err(self.syntax(&format!("expected {keyword}")))
        }
    }

    fn expect_char(&mut self, expected: char) -> Result<(), GraphLiteralTextError> {
        self.skip_whitespace();
        if self.next_char() == Some(expected) {
            Ok(())
        } else {
            Err(self.syntax(&format!("expected '{expected}'")))
        }
    }

    fn consume_char(&mut self, expected: char) -> bool {
        self.skip_whitespace();
        if self.peek_char() == Some(expected) {
            self.next_char();
            true
        } else {
            false
        }
    }

    fn require_value_boundary(&mut self) -> Result<(), GraphLiteralTextError> {
        match self.peek_char() {
            None | Some(',' | ']' | '}' | ')') => Ok(()),
            Some(character) if character.is_whitespace() => Ok(()),
            Some(_) => Err(self.syntax("expected the end of the option value")),
        }
    }

    fn skip_whitespace(&mut self) {
        while self.peek_char().is_some_and(char::is_whitespace) {
            self.next_char();
        }
    }

    fn remaining(&self) -> &str {
        &self.source[self.offset..]
    }

    fn peek_char(&self) -> Option<char> {
        self.remaining().chars().next()
    }

    fn next_char(&mut self) -> Option<char> {
        let character = self.peek_char()?;
        self.offset += character.len_utf8();
        Some(character)
    }

    fn syntax(&self, message: &str) -> GraphLiteralTextError {
        GraphLiteralTextError::Syntax {
            offset: self.offset,
            message: message.to_owned(),
        }
    }
}

const fn is_value_terminator(character: char) -> bool {
    matches!(character, ',' | ']' | '}' | ')')
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use alumina_protocol::{DeviceId, Digest};

    use super::super::{
        BaseDimensions, GraphClockId, GraphLimits, JobGraphHandle, RecordField, RecordFieldId,
        ResourceClassId, ResourceGraphHandle, TypeDefinition, UnitDefinition, UnitId,
    };
    use super::*;

    const UNIT: UnitId = UnitId::new(1);
    const BOOL: GraphTypeId = GraphTypeId::new(1);
    const EXACT: GraphTypeId = GraphTypeId::new(2);
    const INTERVAL: GraphTypeId = GraphTypeId::new(3);
    const SIGNED: GraphTypeId = GraphTypeId::new(4);
    const UNSIGNED: GraphTypeId = GraphTypeId::new(5);
    const TEXT: GraphTypeId = GraphTypeId::new(6);
    const BYTES: GraphTypeId = GraphTypeId::new(7);
    const ARRAY: GraphTypeId = GraphTypeId::new(8);
    const OPTION: GraphTypeId = GraphTypeId::new(9);
    const RESULT: GraphTypeId = GraphTypeId::new(10);
    const RECORD: GraphTypeId = GraphTypeId::new(11);
    const RESOURCE: GraphTypeId = GraphTypeId::new(12);
    const JOB: GraphTypeId = GraphTypeId::new(13);
    const EVENT: GraphTypeId = GraphTypeId::new(14);
    const OPTIONAL_RESOURCE: GraphTypeId = GraphTypeId::new(15);
    const CLASS: ResourceClassId = ResourceClassId::new(7);

    fn schema() -> GraphSchema {
        GraphSchema::try_new(
            GraphLimits::interactive(),
            vec![UnitDefinition::new(
                UNIT,
                "u",
                BaseDimensions::DIMENSIONLESS,
                Rational::from(1),
            )],
            vec![
                TypeDefinition::new(BOOL, "literal.bool", TypeKind::Boolean),
                TypeDefinition::new(
                    EXACT,
                    "literal.exact",
                    TypeKind::ExactRational { unit: UNIT },
                ),
                TypeDefinition::new(
                    INTERVAL,
                    "literal.interval",
                    TypeKind::MeasurementInterval { unit: UNIT },
                ),
                TypeDefinition::new(
                    SIGNED,
                    "literal.signed",
                    TypeKind::CanonicalI64 {
                        unit: UNIT,
                        quantum: Rational::from(1),
                    },
                ),
                TypeDefinition::new(
                    UNSIGNED,
                    "literal.unsigned",
                    TypeKind::CanonicalU64 {
                        unit: UNIT,
                        quantum: Rational::from(1),
                    },
                ),
                TypeDefinition::new(TEXT, "literal.text", TypeKind::Text { maximum_bytes: 64 }),
                TypeDefinition::new(BYTES, "literal.bytes", TypeKind::Bytes { maximum_bytes: 8 }),
                TypeDefinition::new(
                    ARRAY,
                    "literal.array",
                    TypeKind::Array {
                        element: TEXT,
                        maximum_items: 2,
                    },
                ),
                TypeDefinition::new(OPTION, "literal.option", TypeKind::Option { value: TEXT }),
                TypeDefinition::new(
                    RESULT,
                    "literal.result",
                    TypeKind::Result {
                        ok: EXACT,
                        error: BYTES,
                    },
                ),
                TypeDefinition::new(
                    RECORD,
                    "literal.record",
                    TypeKind::Record {
                        fields: vec![
                            RecordField::new(RecordFieldId::new(5), "outcome", RESULT),
                            RecordField::new(RecordFieldId::new(3), "samples", ARRAY),
                            RecordField::new(RecordFieldId::new(1), "enabled", BOOL),
                            RecordField::new(RecordFieldId::new(4), "choice", OPTION),
                            RecordField::new(RecordFieldId::new(2), "label", TEXT),
                        ],
                    },
                ),
                TypeDefinition::new(
                    RESOURCE,
                    "literal.resource",
                    TypeKind::ResourceHandle { class: CLASS },
                ),
                TypeDefinition::new(JOB, "literal.job", TypeKind::JobHandle),
                TypeDefinition::new(
                    EVENT,
                    "literal.event",
                    TypeKind::Event {
                        payload: BOOL,
                        clock: GraphClockId::new(1),
                    },
                ),
                TypeDefinition::new(
                    OPTIONAL_RESOURCE,
                    "literal.optional-resource",
                    TypeKind::Option { value: RESOURCE },
                ),
            ],
        )
        .unwrap()
    }

    fn typed(schema: &GraphSchema, value_type: GraphTypeId, value: GraphValue) -> TypedGraphValue {
        TypedGraphValue::try_new(schema, value_type, value).unwrap()
    }

    fn record_value() -> GraphValue {
        GraphValue::Record(vec![
            RecordValueField {
                field: RecordFieldId::new(1),
                value: GraphValue::Boolean(true),
            },
            RecordValueField {
                field: RecordFieldId::new(2),
                value: GraphValue::Text("ready\n\u{1}".to_owned()),
            },
            RecordValueField {
                field: RecordFieldId::new(3),
                value: GraphValue::Array(vec![
                    GraphValue::Text("α".to_owned()),
                    GraphValue::Text("quote\"".to_owned()),
                ]),
            },
            RecordValueField {
                field: RecordFieldId::new(4),
                value: GraphValue::OptionSome(Box::new(GraphValue::Text("armed".to_owned()))),
            },
            RecordValueField {
                field: RecordFieldId::new(5),
                value: GraphValue::ResultError(Box::new(GraphValue::Bytes(vec![0, 0xff]))),
            },
        ])
    }

    #[test]
    fn exact_composite_notation_round_trips_and_canonicalizes_whitespace() {
        let schema = schema();
        let expected = "{enabled:true,label:\"ready\\n\\u{1}\",samples:[\"α\",\"quote\\\"\"],choice:some(\"armed\"),outcome:error(hex\"00ff\")}";
        let value = typed(&schema, RECORD, record_value());
        assert_eq!(
            format_graph_literal_text(&schema, &value, GraphLiteralTextLimits::interactive())
                .unwrap(),
            expected
        );
        assert_eq!(
            parse_graph_literal_text(
                &schema,
                RECORD,
                " { enabled : true , label : \"ready\\n\\u{1}\" , samples : [ \"α\" , \"quote\\\"\" ] , choice : some ( \"armed\" ) , outcome : error ( hex \"00FF\" ) } ",
                GraphLiteralTextLimits::interactive(),
            )
            .unwrap(),
            value
        );
        for (end, _) in expected.char_indices().skip(1) {
            assert!(
                parse_graph_literal_text(
                    &schema,
                    RECORD,
                    &expected[..end],
                    GraphLiteralTextLimits::interactive(),
                )
                .is_err(),
                "accepted strict prefix ending at byte {end}"
            );
        }
    }

    #[test]
    fn every_scalar_branch_retains_exact_values() {
        let schema = schema();
        let cases = [
            (BOOL, " false ", GraphValue::Boolean(false)),
            (
                EXACT,
                " -7/9 ",
                GraphValue::ExactRational(Rational::fraction(-7, 9).unwrap()),
            ),
            (
                INTERVAL,
                " -1/3 .. 2/3 ",
                GraphValue::MeasurementInterval {
                    lower: Rational::fraction(-1, 3).unwrap(),
                    upper: Rational::fraction(2, 3).unwrap(),
                },
            ),
            (
                SIGNED,
                "-9223372036854775808",
                GraphValue::CanonicalI64(i64::MIN),
            ),
            (
                UNSIGNED,
                "18446744073709551615",
                GraphValue::CanonicalU64(u64::MAX),
            ),
            (
                TEXT,
                "\"line\\n\\0μ\"",
                GraphValue::Text("line\n\0μ".to_owned()),
            ),
            (
                BYTES,
                "hex\"DeAd00\"",
                GraphValue::Bytes(vec![0xde, 0xad, 0]),
            ),
        ];
        for (value_type, source, value) in cases {
            let parsed = parse_graph_literal_text(
                &schema,
                value_type,
                source,
                GraphLiteralTextLimits::interactive(),
            )
            .unwrap();
            assert_eq!(parsed, typed(&schema, value_type, value));
            let canonical =
                format_graph_literal_text(&schema, &parsed, GraphLiteralTextLimits::interactive())
                    .unwrap();
            assert_eq!(
                parse_graph_literal_text(
                    &schema,
                    value_type,
                    &canonical,
                    GraphLiteralTextLimits::interactive(),
                )
                .unwrap(),
                parsed
            );
        }
    }

    #[test]
    fn option_and_result_branches_have_unambiguous_tags() {
        let schema = schema();
        let cases = [
            (OPTION, "none", GraphValue::OptionNone),
            (
                OPTION,
                "some(\"ready\")",
                GraphValue::OptionSome(Box::new(GraphValue::Text("ready".to_owned()))),
            ),
            (
                RESULT,
                "ok(3/4)",
                GraphValue::ResultOk(Box::new(GraphValue::ExactRational(
                    Rational::fraction(3, 4).unwrap(),
                ))),
            ),
            (
                RESULT,
                "error(hex\"\")",
                GraphValue::ResultError(Box::new(GraphValue::Bytes(Vec::new()))),
            ),
        ];
        for (value_type, source, expected) in cases {
            let parsed = parse_graph_literal_text(
                &schema,
                value_type,
                source,
                GraphLiteralTextLimits::interactive(),
            )
            .unwrap();
            assert_eq!(parsed, typed(&schema, value_type, expected));
            assert_eq!(
                format_graph_literal_text(&schema, &parsed, GraphLiteralTextLimits::interactive())
                    .unwrap(),
                source
            );
        }
    }

    #[test]
    fn malformed_composites_fail_before_a_typed_value_exists() {
        let schema = schema();
        for source in [
            "{label:\"x\",enabled:true,samples:[],choice:none,outcome:ok(1)}",
            "{enabled:true,label:\"x\",samples:[\"a\",\"b\",\"c\"],choice:none,outcome:ok(1)}",
            "{enabled:true,label:\"x\",samples:[],choice:some(\"x\"),outcome:error(hex\"0\")}",
            "{enabled:true,label:\"\\u{110000}\",samples:[],choice:none,outcome:ok(1)}",
            "{enabled:true,label:\"x\",samples:[],choice:none,outcome:ok(1)} trailing",
        ] {
            assert!(
                parse_graph_literal_text(
                    &schema,
                    RECORD,
                    source,
                    GraphLiteralTextLimits::interactive(),
                )
                .is_err(),
                "unexpectedly accepted {source}"
            );
        }
        assert_eq!(
            parse_graph_literal_text(
                &schema,
                INTERVAL,
                "2..1",
                GraphLiteralTextLimits::interactive(),
            ),
            Err(GraphLiteralTextError::Schema(
                GraphSchemaError::ReversedMeasurement
            ))
        );
    }

    #[test]
    fn policy_and_identity_authority_fail_closed() {
        let schema = schema();
        let resource = typed(
            &schema,
            RESOURCE,
            GraphValue::ResourceHandle(ResourceGraphHandle {
                device_id: DeviceId([1; 16]),
                board_package_digest: Digest([2; 32]),
                class: CLASS,
                resource_selector: 3,
            }),
        );
        let job = typed(
            &schema,
            JOB,
            GraphValue::JobHandle(JobGraphHandle {
                device_id: DeviceId([3; 16]),
                global_job_digest: Digest([4; 32]),
                partition_digest: Digest([5; 32]),
            }),
        );
        let nested_resource = typed(
            &schema,
            OPTIONAL_RESOURCE,
            GraphValue::OptionSome(Box::new(resource.value().clone())),
        );
        let zero = GraphLiteralTextLimits::new(0);
        assert_eq!(
            parse_graph_literal_text(&schema, BOOL, "true", zero),
            Err(GraphLiteralTextError::ZeroLimit)
        );
        assert!(matches!(
            parse_graph_literal_text(&schema, BOOL, "true", GraphLiteralTextLimits::new(3)),
            Err(GraphLiteralTextError::InputTooLarge { .. })
        ));
        assert!(matches!(
            format_graph_literal_text(
                &schema,
                &typed(&schema, TEXT, GraphValue::Text("μ".into())),
                GraphLiteralTextLimits::new(3)
            ),
            Err(GraphLiteralTextError::OutputTooLarge { .. })
        ));
        for value in [&resource, &job, &nested_resource] {
            assert!(matches!(
                format_graph_literal_text(&schema, value, GraphLiteralTextLimits::interactive()),
                Err(GraphLiteralTextError::IdentityRequiresSelector(_))
            ));
        }
        assert!(matches!(
            parse_graph_literal_text(
                &schema,
                EVENT,
                "true",
                GraphLiteralTextLimits::interactive()
            ),
            Err(GraphLiteralTextError::RuntimeOnly(EVENT))
        ));
        assert!(matches!(
            parse_graph_literal_text(
                &schema,
                OPTIONAL_RESOURCE,
                "some(anything)",
                GraphLiteralTextLimits::interactive()
            ),
            Err(GraphLiteralTextError::IdentityRequiresSelector(RESOURCE))
        ));
    }
}
