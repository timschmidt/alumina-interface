//! Bounded, exact, UI-only CNC geometry import.
//!
//! This is deliberately not a firmware protocol or a general RS-274
//! interpreter. It accepts one connected XY path made from explicitly modal
//! rapid positioning, linear interpolation, and I/J-centred circular arcs.
//! Every decimal is parsed into [`Rational`] before Hypercurve geometry exists;
//! unsupported process, compensation, feed, tool, spindle, Z-axis, or control
//! semantics fail closed.

use std::error::Error as StdError;
use std::fmt;

use alumina_protocol::Digest;
use alumina_storage::sha256;
use hypercurve::{
    CircularArc2, Curve2, CurveError, CurveGeometry2, CurvePath2, ExactCurveError, LineSeg2, Point2,
};
use hyperreal::{Problem, Rational, Real};

/// Result type for exact UI-only CNC geometry import.
pub type CncGeometryImportResult<T> = Result<T, CncGeometryImportError>;

/// Caller-owned admission limits for a CNC geometry source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CncGeometryImportLimits {
    maximum_source_bytes: usize,
    maximum_line_bytes: usize,
    maximum_lines: usize,
    maximum_words: usize,
    maximum_curves: usize,
    maximum_decimal_characters: usize,
}

impl CncGeometryImportLimits {
    /// Interactive browser policy for one imported connected path.
    pub const INTERACTIVE: Self = Self {
        maximum_source_bytes: 1024 * 1024,
        maximum_line_bytes: 4 * 1024,
        maximum_lines: 65_536,
        maximum_words: 262_144,
        maximum_curves: 4_096,
        maximum_decimal_characters: 128,
    };

    /// Construct a complete caller-owned import policy.
    pub const fn try_new(
        maximum_source_bytes: usize,
        maximum_line_bytes: usize,
        maximum_lines: usize,
        maximum_words: usize,
        maximum_curves: usize,
        maximum_decimal_characters: usize,
    ) -> CncGeometryImportResult<Self> {
        if maximum_source_bytes == 0
            || maximum_line_bytes == 0
            || maximum_lines == 0
            || maximum_words == 0
            || maximum_curves == 0
            || maximum_decimal_characters == 0
            || maximum_line_bytes > maximum_source_bytes
        {
            return Err(CncGeometryImportError::InvalidLimits);
        }
        Ok(Self {
            maximum_source_bytes,
            maximum_line_bytes,
            maximum_lines,
            maximum_words,
            maximum_curves,
            maximum_decimal_characters,
        })
    }

    /// Maximum admitted source bytes.
    pub const fn maximum_source_bytes(self) -> usize {
        self.maximum_source_bytes
    }

    /// Maximum bytes in one physical source line.
    pub const fn maximum_line_bytes(self) -> usize {
        self.maximum_line_bytes
    }

    /// Maximum physical source lines.
    pub const fn maximum_lines(self) -> usize {
        self.maximum_lines
    }

    /// Maximum parsed address words.
    pub const fn maximum_words(self) -> usize {
        self.maximum_words
    }

    /// Maximum emitted connected Hypercurve elements.
    pub const fn maximum_curves(self) -> usize {
        self.maximum_curves
    }

    /// Maximum characters in one exact decimal token.
    pub const fn maximum_decimal_characters(self) -> usize {
        self.maximum_decimal_characters
    }
}

/// Exact source unit mode retained at a generated curve.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CncUnitMode {
    /// `G21`: source coordinates are millimetres.
    Millimetres,
    /// `G20`: source coordinates are inches, converted exactly by `127/5`.
    Inches,
}

impl CncUnitMode {
    fn millimetres_per_source_unit(self) -> Rational {
        match self {
            Self::Millimetres => Rational::one(),
            Self::Inches => {
                Rational::fraction(127, 5).expect("127/5 is a valid static unit conversion")
            }
        }
    }
}

/// Endpoint-coordinate modal state retained at a generated curve.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CncDistanceMode {
    /// `G90`: X/Y endpoints are absolute.
    Absolute,
    /// `G91`: X/Y endpoints are incremental.
    Incremental,
}

/// I/J arc-centre modal state retained at a generated arc.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CncArcCenterMode {
    /// `G90.1`: I/J name an absolute centre.
    Absolute,
    /// `G91.1`: I/J are offsets from the arc start.
    Incremental,
}

/// Supported geometric motion associated with one retained curve.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CncMotionKind {
    /// `G1`: exact line segment.
    Linear,
    /// `G2`: exact clockwise circular arc.
    ClockwiseArc,
    /// `G3`: exact counter-clockwise circular arc.
    CounterClockwiseArc,
}

/// Source provenance for one retained Hypercurve element.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CncSourceSpan2 {
    curve_index: usize,
    source_line: usize,
    block_number: Option<u32>,
    motion: CncMotionKind,
    units: CncUnitMode,
    distance_mode: CncDistanceMode,
    arc_center_mode: Option<CncArcCenterMode>,
}

impl CncSourceSpan2 {
    /// Zero-based retained curve index.
    pub const fn curve_index(&self) -> usize {
        self.curve_index
    }

    /// One-based physical source line.
    pub const fn source_line(&self) -> usize {
        self.source_line
    }

    /// Optional non-negative `N` block number.
    pub const fn block_number(&self) -> Option<u32> {
        self.block_number
    }

    /// Exact supported motion family.
    pub const fn motion(&self) -> CncMotionKind {
        self.motion
    }

    /// Active source unit mode.
    pub const fn units(&self) -> CncUnitMode {
        self.units
    }

    /// Active endpoint-coordinate mode.
    pub const fn distance_mode(&self) -> CncDistanceMode {
        self.distance_mode
    }

    /// Active I/J mode for an arc, or `None` for a line.
    pub const fn arc_center_mode(&self) -> Option<CncArcCenterMode> {
        self.arc_center_mode
    }
}

/// Non-canonical source facts retained beside imported exact geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct CncGeometryImportReport2 {
    raw_source_digest: Digest,
    source_bytes: usize,
    source_lines: usize,
    parsed_words: usize,
    positioning_blocks: usize,
    program_end_line: usize,
    start_mm: Point2,
    end_mm: Point2,
    spans: Vec<CncSourceSpan2>,
}

impl CncGeometryImportReport2 {
    /// SHA-256 of the exact imported bytes, including comments and whitespace.
    pub const fn raw_source_digest(&self) -> Digest {
        self.raw_source_digest
    }

    /// Number of admitted source bytes.
    pub const fn source_bytes(&self) -> usize {
        self.source_bytes
    }

    /// Number of physical source lines, including blank lines.
    pub const fn source_lines(&self) -> usize {
        self.source_lines
    }

    /// Number of parsed address words.
    pub const fn parsed_words(&self) -> usize {
        self.parsed_words
    }

    /// Number of pre-cut `G0` positioning blocks.
    pub const fn positioning_blocks(&self) -> usize {
        self.positioning_blocks
    }

    /// One-based line carrying the accepted `M2` or `M30` terminator.
    pub const fn program_end_line(&self) -> usize {
        self.program_end_line
    }

    /// Exact start of the retained connected path in millimetres.
    pub const fn start_mm(&self) -> &Point2 {
        &self.start_mm
    }

    /// Exact end of the retained connected path in millimetres.
    pub const fn end_mm(&self) -> &Point2 {
        &self.end_mm
    }

    /// Per-curve source provenance in retained path order.
    pub fn spans(&self) -> &[CncSourceSpan2] {
        &self.spans
    }
}

/// Exact imported path together with its non-canonical source report.
#[derive(Clone, Debug)]
pub struct ImportedCncGeometry2 {
    path: CurvePath2,
    report: CncGeometryImportReport2,
}

impl ImportedCncGeometry2 {
    /// Borrow the exact retained Hypercurve path.
    pub const fn path(&self) -> &CurvePath2 {
        &self.path
    }

    /// Borrow source/provenance facts that never become firmware input.
    pub const fn report(&self) -> &CncGeometryImportReport2 {
        &self.report
    }

    /// Move the exact path and report into an owning CAM transaction.
    pub fn into_parts(self) -> (CurvePath2, CncGeometryImportReport2) {
        (self.path, self.report)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MotionMode {
    Rapid,
    Linear,
    ClockwiseArc,
    CounterClockwiseArc,
}

impl MotionMode {
    fn retained_kind(self) -> Option<CncMotionKind> {
        match self {
            Self::Rapid => None,
            Self::Linear => Some(CncMotionKind::Linear),
            Self::ClockwiseArc => Some(CncMotionKind::ClockwiseArc),
            Self::CounterClockwiseArc => Some(CncMotionKind::CounterClockwiseArc),
        }
    }
}

#[derive(Clone, Debug)]
struct ParsedWord {
    letter: char,
    value: Rational,
}

#[derive(Default)]
struct ParsedBlock {
    block_number: Option<u32>,
    units: Option<CncUnitMode>,
    distance_mode: Option<CncDistanceMode>,
    plane_xy: bool,
    arc_center_mode: Option<CncArcCenterMode>,
    motion: Option<MotionMode>,
    program_end: bool,
    x: Option<Rational>,
    y: Option<Rational>,
    i: Option<Rational>,
    j: Option<Rational>,
}

impl ParsedBlock {
    fn has_coordinates(&self) -> bool {
        self.x.is_some() || self.y.is_some() || self.i.is_some() || self.j.is_some()
    }

    fn only_program_end(&self) -> bool {
        self.program_end
            && self.units.is_none()
            && self.distance_mode.is_none()
            && !self.plane_xy
            && self.arc_center_mode.is_none()
            && self.motion.is_none()
            && !self.has_coordinates()
    }
}

#[derive(Default)]
struct ModalState {
    units: Option<CncUnitMode>,
    distance_mode: Option<CncDistanceMode>,
    plane_xy: bool,
    arc_center_mode: Option<CncArcCenterMode>,
    motion: Option<MotionMode>,
    current_mm: Option<Point2>,
}

/// Parse one bounded CNC source into a connected exact XY Hypercurve path.
///
/// Supported modal words are `G0`, `G1`, `G2`, `G3`, `G17`, `G20`, `G21`,
/// `G90`, `G91`, `G90.1`, and `G91.1`, plus one terminal `M2` or `M30`.
/// The initial position must be established by an absolute `G0 X… Y…` before
/// retained motion. A later rapid is rejected because V1 represents one
/// connected path and must not silently discard non-cutting machine movement.
pub fn import_exact_cnc_geometry(
    source: &[u8],
    limits: CncGeometryImportLimits,
) -> CncGeometryImportResult<ImportedCncGeometry2> {
    if source.len() > limits.maximum_source_bytes {
        return Err(CncGeometryImportError::SourceTooLarge {
            actual: source.len(),
            maximum: limits.maximum_source_bytes,
        });
    }
    validate_source_bytes(source)?;

    let mut state = ModalState::default();
    let mut curves = Vec::new();
    let mut spans = Vec::new();
    let mut parsed_words = 0_usize;
    let mut source_lines = 0_usize;
    let mut positioning_blocks = 0_usize;
    let mut program_end_line = None;
    let mut envelope_open = false;

    for (line_index, raw_line) in source.split(|byte| *byte == b'\n').enumerate() {
        let line_number = line_index + 1;
        source_lines = line_number;
        if source_lines > limits.maximum_lines {
            return Err(CncGeometryImportError::TooManyLines {
                actual: source_lines,
                maximum: limits.maximum_lines,
            });
        }
        let raw_line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        if raw_line.len() > limits.maximum_line_bytes {
            return Err(CncGeometryImportError::LineTooLong {
                line: line_number,
                actual: raw_line.len(),
                maximum: limits.maximum_line_bytes,
            });
        }
        let uncommented = strip_comments(raw_line, line_number)?;
        let trimmed = trim_ascii_whitespace(&uncommented);
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == b"%" {
            if program_end_line.is_some() {
                if !envelope_open {
                    return Err(CncGeometryImportError::UnexpectedEnvelope { line: line_number });
                }
                envelope_open = false;
            } else if !envelope_open && curves.is_empty() && state.current_mm.is_none() {
                envelope_open = true;
            } else {
                return Err(CncGeometryImportError::UnexpectedEnvelope { line: line_number });
            }
            continue;
        }
        if program_end_line.is_some() {
            return Err(CncGeometryImportError::WordsAfterProgramEnd { line: line_number });
        }

        let words = lex_words(trimmed, line_number, limits.maximum_decimal_characters)?;
        parsed_words = parsed_words.checked_add(words.len()).ok_or(
            CncGeometryImportError::IntegerOverflow {
                domain: "parsed word count",
            },
        )?;
        if parsed_words > limits.maximum_words {
            return Err(CncGeometryImportError::TooManyWords {
                actual: parsed_words,
                maximum: limits.maximum_words,
            });
        }
        let block = parse_block(&words, line_number)?;
        if block.program_end {
            if !block.only_program_end() {
                return Err(CncGeometryImportError::ProgramEndMixed { line: line_number });
            }
            program_end_line = Some(line_number);
            continue;
        }

        if let Some(units) = block.units {
            state.units = Some(units);
        }
        if let Some(distance_mode) = block.distance_mode {
            state.distance_mode = Some(distance_mode);
        }
        if block.plane_xy {
            state.plane_xy = true;
        }
        if let Some(arc_center_mode) = block.arc_center_mode {
            state.arc_center_mode = Some(arc_center_mode);
        }
        if let Some(motion) = block.motion {
            state.motion = Some(motion);
        }
        if !block.has_coordinates() {
            continue;
        }

        let units = state
            .units
            .ok_or(CncGeometryImportError::MissingModalState {
                line: line_number,
                modal: "G20/G21 units",
            })?;
        let distance_mode =
            state
                .distance_mode
                .ok_or(CncGeometryImportError::MissingModalState {
                    line: line_number,
                    modal: "G90/G91 endpoint distance",
                })?;
        if !state.plane_xy {
            return Err(CncGeometryImportError::MissingModalState {
                line: line_number,
                modal: "G17 XY plane",
            });
        }
        let motion = state
            .motion
            .ok_or(CncGeometryImportError::MissingModalState {
                line: line_number,
                modal: "G0/G1/G2/G3 motion",
            })?;
        let scale = units.millimetres_per_source_unit();

        if state.current_mm.is_none() {
            if motion != MotionMode::Rapid || distance_mode != CncDistanceMode::Absolute {
                return Err(CncGeometryImportError::MissingInitialPosition { line: line_number });
            }
            let x = block
                .x
                .as_ref()
                .ok_or(CncGeometryImportError::IncompleteInitialPosition { line: line_number })?;
            let y = block
                .y
                .as_ref()
                .ok_or(CncGeometryImportError::IncompleteInitialPosition { line: line_number })?;
            if block.i.is_some() || block.j.is_some() {
                return Err(CncGeometryImportError::UnexpectedArcCenter { line: line_number });
            }
            state.current_mm = Some(Point2::new(Real::from(x * &scale), Real::from(y * &scale)));
            positioning_blocks = positioning_blocks.checked_add(1).ok_or(
                CncGeometryImportError::IntegerOverflow {
                    domain: "positioning block count",
                },
            )?;
            continue;
        }

        let start = state
            .current_mm
            .as_ref()
            .expect("the initial-position branch established a point")
            .clone();
        let endpoint = endpoint_from_block(&start, &block, distance_mode, &scale, line_number)?;
        if motion == MotionMode::Rapid {
            if !curves.is_empty() {
                return Err(CncGeometryImportError::DisconnectedRapid { line: line_number });
            }
            if block.i.is_some() || block.j.is_some() {
                return Err(CncGeometryImportError::UnexpectedArcCenter { line: line_number });
            }
            state.current_mm = Some(endpoint);
            positioning_blocks = positioning_blocks.checked_add(1).ok_or(
                CncGeometryImportError::IntegerOverflow {
                    domain: "positioning block count",
                },
            )?;
            continue;
        }

        let required_curves =
            curves
                .len()
                .checked_add(1)
                .ok_or(CncGeometryImportError::IntegerOverflow {
                    domain: "retained curve count",
                })?;
        if required_curves > limits.maximum_curves {
            return Err(CncGeometryImportError::TooManyCurves {
                required: required_curves,
                maximum: limits.maximum_curves,
            });
        }
        let (curve, arc_center_mode) = match motion {
            MotionMode::Rapid => unreachable!("rapid was handled before curve construction"),
            MotionMode::Linear => {
                if block.i.is_some() || block.j.is_some() {
                    return Err(CncGeometryImportError::UnexpectedArcCenter { line: line_number });
                }
                let line =
                    LineSeg2::try_new(start.clone(), endpoint.clone()).map_err(|source| {
                        CncGeometryImportError::Geometry {
                            line: line_number,
                            source,
                        }
                    })?;
                (Curve2::new(CurveGeometry2::Line(line)), None)
            }
            MotionMode::ClockwiseArc | MotionMode::CounterClockwiseArc => {
                if start == endpoint {
                    return Err(CncGeometryImportError::FullCircleUnsupported {
                        line: line_number,
                    });
                }
                let center_mode =
                    state
                        .arc_center_mode
                        .ok_or(CncGeometryImportError::MissingModalState {
                            line: line_number,
                            modal: "G90.1/G91.1 arc centre",
                        })?;
                let center =
                    arc_center_from_block(&start, &block, center_mode, &scale, line_number)?;
                let arc = CircularArc2::try_from_center(
                    start.clone(),
                    endpoint.clone(),
                    center,
                    motion == MotionMode::ClockwiseArc,
                )
                .map_err(|source| CncGeometryImportError::Geometry {
                    line: line_number,
                    source,
                })?;
                (
                    Curve2::new(CurveGeometry2::CircularArc(arc)),
                    Some(center_mode),
                )
            }
        };
        let curve_index = curves.len();
        curves
            .try_reserve(1)
            .map_err(|_| CncGeometryImportError::AllocationOverflow {
                domain: "retained CNC curves",
            })?;
        spans
            .try_reserve(1)
            .map_err(|_| CncGeometryImportError::AllocationOverflow {
                domain: "CNC source provenance",
            })?;
        curves.push(curve);
        spans.push(CncSourceSpan2 {
            curve_index,
            source_line: line_number,
            block_number: block.block_number,
            motion: motion
                .retained_kind()
                .expect("non-rapid modes have retained provenance"),
            units,
            distance_mode,
            arc_center_mode,
        });
        state.current_mm = Some(endpoint);
    }

    let program_end_line = program_end_line.ok_or(CncGeometryImportError::MissingProgramEnd)?;
    if envelope_open {
        return Err(CncGeometryImportError::UnclosedEnvelope);
    }
    if curves.is_empty() {
        return Err(CncGeometryImportError::EmptyPath);
    }
    let start_mm = curves
        .first()
        .expect("empty paths were rejected")
        .start()
        .clone();
    let end_mm = curves
        .last()
        .expect("empty paths were rejected")
        .end()
        .clone();
    let path = CurvePath2::try_new(curves).map_err(CncGeometryImportError::Path)?;

    Ok(ImportedCncGeometry2 {
        path,
        report: CncGeometryImportReport2 {
            raw_source_digest: sha256(source).digest,
            source_bytes: source.len(),
            source_lines,
            parsed_words,
            positioning_blocks,
            program_end_line,
            start_mm,
            end_mm,
            spans,
        },
    })
}

fn validate_source_bytes(source: &[u8]) -> CncGeometryImportResult<()> {
    for (offset, byte) in source.iter().copied().enumerate() {
        if !matches!(byte, b'\n' | b'\r' | b'\t' | 0x20..=0x7e) {
            return Err(CncGeometryImportError::InvalidSourceByte { offset, byte });
        }
    }
    Ok(())
}

fn strip_comments(line: &[u8], line_number: usize) -> CncGeometryImportResult<Vec<u8>> {
    let mut result = Vec::new();
    result
        .try_reserve(line.len())
        .map_err(|_| CncGeometryImportError::AllocationOverflow {
            domain: "uncommented source line",
        })?;
    let mut in_parentheses = false;
    for (column_index, byte) in line.iter().copied().enumerate() {
        match byte {
            b'(' if in_parentheses => {
                return Err(CncGeometryImportError::NestedComment {
                    line: line_number,
                    column: column_index + 1,
                });
            }
            b'(' => in_parentheses = true,
            b')' if !in_parentheses => {
                return Err(CncGeometryImportError::UnexpectedCommentEnd {
                    line: line_number,
                    column: column_index + 1,
                });
            }
            b')' => in_parentheses = false,
            b';' if !in_parentheses => break,
            _ if !in_parentheses => result.push(byte),
            _ => {}
        }
    }
    if in_parentheses {
        return Err(CncGeometryImportError::UnclosedComment { line: line_number });
    }
    Ok(result)
}

fn trim_ascii_whitespace(mut value: &[u8]) -> &[u8] {
    while value
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        value = &value[1..];
    }
    while value
        .last()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        value = &value[..value.len() - 1];
    }
    value
}

fn lex_words(
    line: &[u8],
    line_number: usize,
    maximum_decimal_characters: usize,
) -> CncGeometryImportResult<Vec<ParsedWord>> {
    let mut words = Vec::new();
    let mut cursor = 0_usize;
    while cursor < line.len() {
        while line
            .get(cursor)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            cursor += 1;
        }
        if cursor == line.len() {
            break;
        }
        let column = cursor + 1;
        let Some(letter) = line[cursor]
            .is_ascii_alphabetic()
            .then(|| line[cursor].to_ascii_uppercase() as char)
        else {
            return Err(CncGeometryImportError::MalformedWord {
                line: line_number,
                column,
            });
        };
        cursor += 1;
        let value_start = cursor;
        while cursor < line.len()
            && !line[cursor].is_ascii_alphabetic()
            && !matches!(line[cursor], b' ' | b'\t')
        {
            cursor += 1;
        }
        let raw_value = &line[value_start..cursor];
        if raw_value.len() > maximum_decimal_characters {
            return Err(CncGeometryImportError::DecimalTooLong {
                line: line_number,
                letter,
                actual: raw_value.len(),
                maximum: maximum_decimal_characters,
            });
        }
        let value = parse_exact_decimal(raw_value).map_err(|source| {
            CncGeometryImportError::InvalidDecimal {
                line: line_number,
                letter,
                source,
            }
        })?;
        words
            .try_reserve(1)
            .map_err(|_| CncGeometryImportError::AllocationOverflow {
                domain: "parsed source words",
            })?;
        words.push(ParsedWord { letter, value });
    }
    Ok(words)
}

fn parse_exact_decimal(raw: &[u8]) -> Result<Rational, Problem> {
    if raw.is_empty() {
        return Err(Problem::BadDecimal);
    }
    let mut start = 0_usize;
    let mut negative = false;
    match raw[0] {
        b'+' => start = 1,
        b'-' => {
            start = 1;
            negative = true;
        }
        _ => {}
    }
    let magnitude = &raw[start..];
    if magnitude.is_empty()
        || magnitude.iter().filter(|byte| **byte == b'.').count() > 1
        || magnitude
            .iter()
            .any(|byte| !byte.is_ascii_digit() && *byte != b'.')
        || !magnitude.iter().any(u8::is_ascii_digit)
    {
        return Err(Problem::BadDecimal);
    }

    let needs_leading_zero = magnitude.first() == Some(&b'.');
    let omits_trailing_dot = magnitude.last() == Some(&b'.');
    let mut normalized = String::new();
    normalized
        .try_reserve(raw.len() + usize::from(needs_leading_zero))
        .map_err(|_| Problem::Exhausted)?;
    if negative {
        normalized.push('-');
    }
    if needs_leading_zero {
        normalized.push('0');
    }
    let magnitude = if omits_trailing_dot {
        &magnitude[..magnitude.len() - 1]
    } else {
        magnitude
    };
    normalized.push_str(std::str::from_utf8(magnitude).map_err(|_| Problem::BadDecimal)?);
    normalized.parse()
}

fn parse_block(words: &[ParsedWord], line: usize) -> CncGeometryImportResult<ParsedBlock> {
    let mut block = ParsedBlock::default();
    let mut m_code = None;
    for word in words {
        match word.letter {
            'N' => {
                if block.block_number.is_some() {
                    return Err(CncGeometryImportError::DuplicateWord { line, letter: 'N' });
                }
                block.block_number = Some(
                    u32::try_from(word.value.clone())
                        .map_err(|_| CncGeometryImportError::InvalidBlockNumber { line })?,
                );
            }
            'G' => apply_g_code(&mut block, &word.value, line)?,
            'M' => {
                if m_code.is_some() {
                    return Err(CncGeometryImportError::DuplicateWord { line, letter: 'M' });
                }
                m_code = Some(word.value.clone());
            }
            'X' => set_coordinate(&mut block.x, word.value.clone(), line, 'X')?,
            'Y' => set_coordinate(&mut block.y, word.value.clone(), line, 'Y')?,
            'I' => set_coordinate(&mut block.i, word.value.clone(), line, 'I')?,
            'J' => set_coordinate(&mut block.j, word.value.clone(), line, 'J')?,
            letter => return Err(CncGeometryImportError::UnsupportedWord { line, letter }),
        }
    }
    if let Some(code) = m_code {
        if code == Rational::from(2) || code == Rational::from(30) {
            block.program_end = true;
        } else {
            return Err(CncGeometryImportError::UnsupportedMCode { line, code });
        }
    }
    Ok(block)
}

fn apply_g_code(
    block: &mut ParsedBlock,
    code: &Rational,
    line: usize,
) -> CncGeometryImportResult<()> {
    if *code == Rational::zero() {
        set_modal(&mut block.motion, MotionMode::Rapid, line, "motion")
    } else if *code == Rational::one() {
        set_modal(&mut block.motion, MotionMode::Linear, line, "motion")
    } else if *code == Rational::from(2) {
        set_modal(&mut block.motion, MotionMode::ClockwiseArc, line, "motion")
    } else if *code == Rational::from(3) {
        set_modal(
            &mut block.motion,
            MotionMode::CounterClockwiseArc,
            line,
            "motion",
        )
    } else if *code == Rational::from(17) {
        if block.plane_xy {
            return Err(CncGeometryImportError::ConflictingModalGroup {
                line,
                group: "plane",
            });
        }
        block.plane_xy = true;
        Ok(())
    } else if *code == Rational::from(20) {
        set_modal(&mut block.units, CncUnitMode::Inches, line, "units")
    } else if *code == Rational::from(21) {
        set_modal(&mut block.units, CncUnitMode::Millimetres, line, "units")
    } else if *code == Rational::from(90) {
        set_modal(
            &mut block.distance_mode,
            CncDistanceMode::Absolute,
            line,
            "endpoint distance",
        )
    } else if *code == Rational::from(91) {
        set_modal(
            &mut block.distance_mode,
            CncDistanceMode::Incremental,
            line,
            "endpoint distance",
        )
    } else if *code == Rational::fraction(901, 10).expect("90.1 is valid") {
        set_modal(
            &mut block.arc_center_mode,
            CncArcCenterMode::Absolute,
            line,
            "arc centre",
        )
    } else if *code == Rational::fraction(911, 10).expect("91.1 is valid") {
        set_modal(
            &mut block.arc_center_mode,
            CncArcCenterMode::Incremental,
            line,
            "arc centre",
        )
    } else {
        Err(CncGeometryImportError::UnsupportedGCode {
            line,
            code: code.clone(),
        })
    }
}

fn set_modal<T: Copy>(
    target: &mut Option<T>,
    value: T,
    line: usize,
    group: &'static str,
) -> CncGeometryImportResult<()> {
    if target.is_some() {
        return Err(CncGeometryImportError::ConflictingModalGroup { line, group });
    }
    *target = Some(value);
    Ok(())
}

fn set_coordinate(
    target: &mut Option<Rational>,
    value: Rational,
    line: usize,
    letter: char,
) -> CncGeometryImportResult<()> {
    if target.is_some() {
        return Err(CncGeometryImportError::DuplicateWord { line, letter });
    }
    *target = Some(value);
    Ok(())
}

fn endpoint_from_block(
    start: &Point2,
    block: &ParsedBlock,
    distance_mode: CncDistanceMode,
    scale: &Rational,
    line: usize,
) -> CncGeometryImportResult<Point2> {
    if block.x.is_none() && block.y.is_none() {
        return Err(CncGeometryImportError::MissingEndpoint { line });
    }
    let x = coordinate_component(start.x(), block.x.as_ref(), distance_mode, scale);
    let y = coordinate_component(start.y(), block.y.as_ref(), distance_mode, scale);
    Ok(Point2::new(x, y))
}

fn coordinate_component(
    current: &Real,
    source: Option<&Rational>,
    distance_mode: CncDistanceMode,
    scale: &Rational,
) -> Real {
    let Some(source) = source else {
        return current.clone();
    };
    let value = Real::from(source * scale);
    match distance_mode {
        CncDistanceMode::Absolute => value,
        CncDistanceMode::Incremental => current + value,
    }
}

fn arc_center_from_block(
    start: &Point2,
    block: &ParsedBlock,
    center_mode: CncArcCenterMode,
    scale: &Rational,
    line: usize,
) -> CncGeometryImportResult<Point2> {
    let i = block
        .i
        .as_ref()
        .ok_or(CncGeometryImportError::MissingArcCenter { line })?;
    let j = block
        .j
        .as_ref()
        .ok_or(CncGeometryImportError::MissingArcCenter { line })?;
    let i = Real::from(i * scale);
    let j = Real::from(j * scale);
    Ok(match center_mode {
        CncArcCenterMode::Absolute => Point2::new(i, j),
        CncArcCenterMode::Incremental => Point2::new(start.x() + i, start.y() + j),
    })
}

/// Failure to admit or construct exact UI-only CNC geometry.
#[derive(Debug)]
pub enum CncGeometryImportError {
    /// At least one configured admission limit was zero or inconsistent.
    InvalidLimits,
    /// Source byte length exceeded policy before parsing.
    SourceTooLarge {
        /// Observed byte length.
        actual: usize,
        /// Caller-owned maximum.
        maximum: usize,
    },
    /// A non-printable or non-ASCII source byte was found.
    InvalidSourceByte {
        /// Zero-based source byte offset.
        offset: usize,
        /// Rejected byte.
        byte: u8,
    },
    /// Physical line count exceeded policy.
    TooManyLines {
        /// Observed line count.
        actual: usize,
        /// Caller-owned maximum.
        maximum: usize,
    },
    /// One physical line exceeded policy.
    LineTooLong {
        /// One-based source line.
        line: usize,
        /// Observed byte length.
        actual: usize,
        /// Caller-owned maximum.
        maximum: usize,
    },
    /// A parenthesized comment contained another opening parenthesis.
    NestedComment {
        /// One-based source line.
        line: usize,
        /// One-based byte column.
        column: usize,
    },
    /// A comment close appeared without a matching open.
    UnexpectedCommentEnd {
        /// One-based source line.
        line: usize,
        /// One-based byte column.
        column: usize,
    },
    /// A parenthesized comment was not closed on its physical line.
    UnclosedComment {
        /// One-based source line.
        line: usize,
    },
    /// A `%` program envelope marker appeared in an unsupported position.
    UnexpectedEnvelope {
        /// One-based source line.
        line: usize,
    },
    /// An opening `%` envelope lacked a closing marker after program end.
    UnclosedEnvelope,
    /// Address-word syntax was malformed.
    MalformedWord {
        /// One-based source line.
        line: usize,
        /// One-based byte column.
        column: usize,
    },
    /// An exact decimal token exceeded policy.
    DecimalTooLong {
        /// One-based source line.
        line: usize,
        /// Address letter.
        letter: char,
        /// Observed token characters.
        actual: usize,
        /// Caller-owned maximum.
        maximum: usize,
    },
    /// An exact decimal token was invalid.
    InvalidDecimal {
        /// One-based source line.
        line: usize,
        /// Address letter.
        letter: char,
        /// Hyperreal parse failure.
        source: Problem,
    },
    /// Parsed word count exceeded policy.
    TooManyWords {
        /// Observed parsed words.
        actual: usize,
        /// Caller-owned maximum.
        maximum: usize,
    },
    /// One non-modal address appeared twice in a block.
    DuplicateWord {
        /// One-based source line.
        line: usize,
        /// Duplicate address letter.
        letter: char,
    },
    /// Two words from one modal group appeared in a block.
    ConflictingModalGroup {
        /// One-based source line.
        line: usize,
        /// Modal group name.
        group: &'static str,
    },
    /// A word outside the selected geometry subset was present.
    UnsupportedWord {
        /// One-based source line.
        line: usize,
        /// Rejected address letter.
        letter: char,
    },
    /// A G code outside the selected geometry subset was present.
    UnsupportedGCode {
        /// One-based source line.
        line: usize,
        /// Rejected exact code.
        code: Rational,
    },
    /// An M code other than terminal M2/M30 was present.
    UnsupportedMCode {
        /// One-based source line.
        line: usize,
        /// Rejected exact code.
        code: Rational,
    },
    /// An N block number was negative, fractional, or wider than `u32`.
    InvalidBlockNumber {
        /// One-based source line.
        line: usize,
    },
    /// A geometry block lacked an explicitly established modal state.
    MissingModalState {
        /// One-based source line.
        line: usize,
        /// Required modal group.
        modal: &'static str,
    },
    /// The path origin was not established by absolute G0 X/Y.
    MissingInitialPosition {
        /// One-based source line.
        line: usize,
    },
    /// The first absolute rapid lacked either X or Y.
    IncompleteInitialPosition {
        /// One-based source line.
        line: usize,
    },
    /// A motion block lacked both X and Y endpoint words.
    MissingEndpoint {
        /// One-based source line.
        line: usize,
    },
    /// An arc lacked either explicit I or J.
    MissingArcCenter {
        /// One-based source line.
        line: usize,
    },
    /// I/J appeared on a rapid or line block.
    UnexpectedArcCenter {
        /// One-based source line.
        line: usize,
    },
    /// G0 appeared after retained path motion began.
    DisconnectedRapid {
        /// One-based source line.
        line: usize,
    },
    /// A start-equals-end full circle is outside V1 selected semantics.
    FullCircleUnsupported {
        /// One-based source line.
        line: usize,
    },
    /// M2/M30 shared a block with another semantic word.
    ProgramEndMixed {
        /// One-based source line.
        line: usize,
    },
    /// A non-comment word followed M2/M30.
    WordsAfterProgramEnd {
        /// One-based source line.
        line: usize,
    },
    /// The source lacked terminal M2/M30.
    MissingProgramEnd,
    /// No retained line or arc was present.
    EmptyPath,
    /// Retained curve count exceeded policy before construction.
    TooManyCurves {
        /// Curve count required by the next block.
        required: usize,
        /// Caller-owned maximum.
        maximum: usize,
    },
    /// Hypercurve rejected one exact primitive.
    Geometry {
        /// One-based source line.
        line: usize,
        /// Exact primitive construction failure.
        source: CurveError,
    },
    /// Hypercurve rejected final path topology or connectivity.
    Path(ExactCurveError),
    /// Checked counter arithmetic overflowed.
    IntegerOverflow {
        /// Counter domain.
        domain: &'static str,
    },
    /// A bounded temporary allocation failed.
    AllocationOverflow {
        /// Allocation domain.
        domain: &'static str,
    },
}

impl fmt::Display for CncGeometryImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits => formatter.write_str("CNC import limits are invalid"),
            Self::SourceTooLarge { actual, maximum } => write!(
                formatter,
                "CNC source has {actual} bytes; policy permits {maximum}"
            ),
            Self::InvalidSourceByte { offset, byte } => write!(
                formatter,
                "CNC source byte {offset} is unsupported ASCII/control value 0x{byte:02x}"
            ),
            Self::TooManyLines { actual, maximum } => write!(
                formatter,
                "CNC source has {actual} lines; policy permits {maximum}"
            ),
            Self::LineTooLong {
                line,
                actual,
                maximum,
            } => write!(
                formatter,
                "CNC line {line} has {actual} bytes; policy permits {maximum}"
            ),
            Self::NestedComment { line, column } => {
                write!(
                    formatter,
                    "nested CNC comment at line {line}, column {column}"
                )
            }
            Self::UnexpectedCommentEnd { line, column } => write!(
                formatter,
                "unmatched CNC comment close at line {line}, column {column}"
            ),
            Self::UnclosedComment { line } => {
                write!(formatter, "unclosed CNC comment on line {line}")
            }
            Self::UnexpectedEnvelope { line } => {
                write!(formatter, "unexpected CNC % envelope marker on line {line}")
            }
            Self::UnclosedEnvelope => formatter.write_str("CNC % envelope was not closed"),
            Self::MalformedWord { line, column } => {
                write!(
                    formatter,
                    "malformed CNC word at line {line}, column {column}"
                )
            }
            Self::DecimalTooLong {
                line,
                letter,
                actual,
                maximum,
            } => write!(
                formatter,
                "CNC {letter} decimal on line {line} has {actual} characters; policy permits {maximum}"
            ),
            Self::InvalidDecimal {
                line,
                letter,
                source,
            } => write!(
                formatter,
                "CNC {letter} decimal on line {line} is invalid: {source}"
            ),
            Self::TooManyWords { actual, maximum } => write!(
                formatter,
                "CNC source has {actual} words; policy permits {maximum}"
            ),
            Self::DuplicateWord { line, letter } => {
                write!(formatter, "duplicate CNC {letter} word on line {line}")
            }
            Self::ConflictingModalGroup { line, group } => write!(
                formatter,
                "multiple CNC {group} modal words occur on line {line}"
            ),
            Self::UnsupportedWord { line, letter } => write!(
                formatter,
                "CNC {letter} word on line {line} is outside the UI geometry subset"
            ),
            Self::UnsupportedGCode { line, code } => write!(
                formatter,
                "CNC G{code} on line {line} is outside the UI geometry subset"
            ),
            Self::UnsupportedMCode { line, code } => write!(
                formatter,
                "CNC M{code} on line {line} is outside terminal M2/M30 semantics"
            ),
            Self::InvalidBlockNumber { line } => {
                write!(
                    formatter,
                    "CNC N block number on line {line} is not a u32 integer"
                )
            }
            Self::MissingModalState { line, modal } => {
                write!(formatter, "CNC line {line} lacks explicit {modal} state")
            }
            Self::MissingInitialPosition { line } => write!(
                formatter,
                "CNC line {line} begins motion before absolute G0 establishes X and Y"
            ),
            Self::IncompleteInitialPosition { line } => write!(
                formatter,
                "initial CNC rapid on line {line} must specify both X and Y"
            ),
            Self::MissingEndpoint { line } => {
                write!(formatter, "CNC motion on line {line} has no X/Y endpoint")
            }
            Self::MissingArcCenter { line } => write!(
                formatter,
                "CNC arc on line {line} must specify both I and J exactly"
            ),
            Self::UnexpectedArcCenter { line } => write!(
                formatter,
                "CNC I/J centre words are invalid for motion on line {line}"
            ),
            Self::DisconnectedRapid { line } => write!(
                formatter,
                "CNC rapid on line {line} would disconnect the single V1 path"
            ),
            Self::FullCircleUnsupported { line } => write!(
                formatter,
                "CNC start-equals-end full circle on line {line} is outside V1 semantics"
            ),
            Self::ProgramEndMixed { line } => write!(
                formatter,
                "CNC program end on line {line} must be its only semantic word"
            ),
            Self::WordsAfterProgramEnd { line } => {
                write!(
                    formatter,
                    "CNC word appears after program end on line {line}"
                )
            }
            Self::MissingProgramEnd => formatter.write_str("CNC source lacks terminal M2 or M30"),
            Self::EmptyPath => formatter.write_str("CNC source contains no retained line or arc"),
            Self::TooManyCurves { required, maximum } => write!(
                formatter,
                "CNC path requires {required} curves; policy permits {maximum}"
            ),
            Self::Geometry { line, source } => write!(
                formatter,
                "exact CNC geometry on line {line} was rejected: {source}"
            ),
            Self::Path(source) => write!(formatter, "exact CNC path was rejected: {source}"),
            Self::IntegerOverflow { domain } => {
                write!(formatter, "CNC {domain} exceeded integer representation")
            }
            Self::AllocationOverflow { domain } => {
                write!(formatter, "bounded CNC allocation failed for {domain}")
            }
        }
    }
}

impl StdError for CncGeometryImportError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::InvalidDecimal { source, .. } => Some(source),
            Self::Geometry { source, .. } => Some(source),
            Self::Path(source) => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPRESENTATIVE: &[u8] = br"%
(all modal state is explicit)
G21 G90 G17 G91.1
N10 G0 X0 Y0
N20 G1 X4 Y0
N30 G2 X8 Y0 I2 J0
M30
%
";

    #[test]
    fn selected_line_arc_program_retains_exact_geometry_and_provenance() {
        let imported =
            import_exact_cnc_geometry(REPRESENTATIVE, CncGeometryImportLimits::INTERACTIVE)
                .unwrap();
        assert_eq!(imported.path().curves().len(), 2);
        assert_eq!(imported.report().source_bytes(), REPRESENTATIVE.len());
        assert_eq!(imported.report().parsed_words(), 19);
        assert_eq!(imported.report().positioning_blocks(), 1);
        assert_eq!(imported.report().program_end_line(), 7);
        assert_eq!(imported.report().start_mm(), &Point2::from_values(0, 0));
        assert_eq!(imported.report().end_mm(), &Point2::from_values(8, 0));
        assert_eq!(imported.report().spans().len(), 2);
        assert_eq!(imported.report().spans()[0].block_number(), Some(20));
        assert_eq!(imported.report().spans()[0].motion(), CncMotionKind::Linear);
        assert_eq!(
            imported.report().spans()[1].motion(),
            CncMotionKind::ClockwiseArc
        );
        assert_eq!(
            imported.report().spans()[1].arc_center_mode(),
            Some(CncArcCenterMode::Incremental)
        );

        let CurveGeometry2::CircularArc(arc) = imported.path().curves()[1].geometry() else {
            panic!("second imported curve must remain a native arc");
        };
        assert_eq!(arc.center(), &Point2::from_values(6, 0));
        assert!(arc.is_clockwise());
        assert_eq!(
            imported.report().raw_source_digest(),
            sha256(REPRESENTATIVE).digest
        );
    }

    #[test]
    fn inch_and_incremental_coordinates_convert_to_exact_millimetres() {
        let source = b"G20 G90 G17 G91.1\nG0 X0 Y0\nG1 X1 Y0\nG91\nG1 X.5 Y.25\nM2\n";
        let imported =
            import_exact_cnc_geometry(source, CncGeometryImportLimits::INTERACTIVE).unwrap();
        assert_eq!(
            imported.report().end_mm(),
            &Point2::new(
                Real::from(Rational::fraction(381, 10).unwrap()),
                Real::from(Rational::fraction(127, 20).unwrap()),
            )
        );
        assert_eq!(
            imported.report().spans()[1].distance_mode(),
            CncDistanceMode::Incremental
        );
        assert_eq!(imported.report().spans()[1].units(), CncUnitMode::Inches);
    }

    #[test]
    fn selected_subset_rejects_ambiguous_or_process_semantics() {
        for (source, expected) in [
            (
                b"G21 G90 G17 G91.1\nG1 X1 Y0\nM2\n".as_slice(),
                "absolute G0",
            ),
            (
                b"G21 G90 G17 G91.1\nG0 X0 Y0\nG1 X1 Y0 F10\nM2\n".as_slice(),
                "F word",
            ),
            (
                b"G21 G90 G17 G91.1\nG0 X0 Y0\nG1 X1 Y0\nG0 X2 Y0\nM2\n".as_slice(),
                "disconnect",
            ),
            (
                b"G21 G90 G17 G91.1\nG0 X0 Y0\nG2 X1 Y0 I0 J1\nM2\n".as_slice(),
                "do not share the supplied radius",
            ),
        ] {
            let error = import_exact_cnc_geometry(source, CncGeometryImportLimits::INTERACTIVE)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(expected),
                "expected {expected:?} in {error:?}"
            );
        }
    }

    #[test]
    fn source_limits_fail_before_unbounded_geometry_growth() {
        let tiny = CncGeometryImportLimits::try_new(64, 32, 8, 20, 1, 8).unwrap();
        assert!(matches!(
            import_exact_cnc_geometry(REPRESENTATIVE, tiny),
            Err(CncGeometryImportError::SourceTooLarge { maximum: 64, .. })
        ));
        let curve_limited = CncGeometryImportLimits::try_new(1_024, 128, 16, 64, 1, 16).unwrap();
        assert!(matches!(
            import_exact_cnc_geometry(
                b"G21 G90 G17 G91.1\nG0 X0 Y0\nG1 X1 Y0\nG1 X2 Y0\nM2\n",
                curve_limited,
            ),
            Err(CncGeometryImportError::TooManyCurves {
                required: 2,
                maximum: 1,
            })
        ));
        assert!(matches!(
            CncGeometryImportLimits::try_new(8, 9, 1, 1, 1, 1),
            Err(CncGeometryImportError::InvalidLimits)
        ));
    }

    #[test]
    fn every_representative_prefix_fails_without_panicking_or_partial_output() {
        for length in 0..REPRESENTATIVE.len() - 1 {
            assert!(
                import_exact_cnc_geometry(
                    &REPRESENTATIVE[..length],
                    CncGeometryImportLimits::INTERACTIVE,
                )
                .is_err()
            );
        }
        assert!(
            import_exact_cnc_geometry(REPRESENTATIVE, CncGeometryImportLimits::INTERACTIVE).is_ok()
        );
    }
}
