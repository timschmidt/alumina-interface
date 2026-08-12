//! Checked canonical storage ceilings for registered graph types.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};

use super::{GraphClockId, GraphSchema, GraphTypeId, TypeKind};

const TYPE_ID_BYTES: u64 = 4;
const LENGTH_BYTES: u64 = 4;
const VARIANT_TAG_BYTES: u64 = 1;

/// Exact canonical storage class and maximum byte count for one registered type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphTypeStorageKind {
    /// Complete typed literal, including its root type ID.
    Literal {
        /// Maximum bytes accepted by the canonical V1 value encoding.
        maximum_canonical_bytes: u64,
    },
    /// Runtime event payload. Timestamp/queue envelope bytes are not included.
    EventPayload {
        /// Explicit event clock.
        clock: GraphClockId,
        /// Maximum complete typed payload bytes.
        maximum_payload_bytes: u64,
    },
    /// Runtime stream sample. Timestamp/queue envelope bytes are not included.
    StreamSample {
        /// Explicit sample clock.
        clock: GraphClockId,
        /// Maximum complete typed sample bytes.
        maximum_sample_bytes: u64,
        /// Type-declared maximum retained samples.
        capacity: u32,
    },
}

/// Checked storage ceiling for one stable graph type identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphTypeStorageBound {
    value_type: GraphTypeId,
    kind: GraphTypeStorageKind,
}

impl GraphTypeStorageBound {
    /// Return the registered type identity.
    pub const fn value_type(self) -> GraphTypeId {
        self.value_type
    }

    /// Return the exact storage class and byte ceiling.
    pub const fn kind(self) -> GraphTypeStorageKind {
        self.kind
    }

    /// Return complete typed-literal bytes, or `None` for runtime-only types.
    pub const fn maximum_literal_bytes(self) -> Option<u64> {
        match self.kind {
            GraphTypeStorageKind::Literal {
                maximum_canonical_bytes,
            } => Some(maximum_canonical_bytes),
            GraphTypeStorageKind::EventPayload { .. }
            | GraphTypeStorageKind::StreamSample { .. } => None,
        }
    }
}

/// Failure while proving canonical type-storage ceilings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphStorageError {
    /// A referenced type was absent despite schema validation.
    UnknownType(GraphTypeId),
    /// A type reference recurred despite schema validation.
    RecursiveType(GraphTypeId),
    /// A runtime event/stream appeared inside a saved literal composite.
    RuntimeTypeNested {
        /// Composite requiring literal storage.
        container: GraphTypeId,
        /// Runtime-only referenced type.
        runtime_type: GraphTypeId,
    },
    /// A maximum byte calculation exceeded `u64`.
    SizeOverflow(GraphTypeId),
}

impl fmt::Display for GraphStorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownType(value_type) => {
                write!(
                    formatter,
                    "storage analysis found unknown type {value_type:?}"
                )
            }
            Self::RecursiveType(value_type) => {
                write!(
                    formatter,
                    "storage analysis found recursive type {value_type:?}"
                )
            }
            Self::RuntimeTypeNested {
                container,
                runtime_type,
            } => write!(
                formatter,
                "literal type {container:?} contains runtime-only type {runtime_type:?}"
            ),
            Self::SizeOverflow(value_type) => {
                write!(formatter, "type {value_type:?} storage ceiling exceeds u64")
            }
        }
    }
}

impl std::error::Error for GraphStorageError {}

#[derive(Clone, Copy)]
enum StorageShape {
    LiteralBody(u64),
    EventPayload {
        clock: GraphClockId,
        payload_body: u64,
    },
    StreamSample {
        clock: GraphClockId,
        sample_body: u64,
        capacity: u32,
    },
}

pub(super) fn analyze_type_storage(
    schema: &GraphSchema,
) -> Result<Vec<GraphTypeStorageBound>, GraphStorageError> {
    let mut memo = BTreeMap::new();
    let mut visiting = BTreeSet::new();
    let mut bounds = Vec::with_capacity(schema.types().len());
    for definition in schema.types() {
        let value_type = definition.id();
        let shape = storage_shape(schema, value_type, &mut memo, &mut visiting)?;
        let kind = match shape {
            StorageShape::LiteralBody(body) => GraphTypeStorageKind::Literal {
                maximum_canonical_bytes: checked_add(value_type, TYPE_ID_BYTES, body)?,
            },
            StorageShape::EventPayload {
                clock,
                payload_body,
            } => GraphTypeStorageKind::EventPayload {
                clock,
                maximum_payload_bytes: checked_add(value_type, TYPE_ID_BYTES, payload_body)?,
            },
            StorageShape::StreamSample {
                clock,
                sample_body,
                capacity,
            } => GraphTypeStorageKind::StreamSample {
                clock,
                maximum_sample_bytes: checked_add(value_type, TYPE_ID_BYTES, sample_body)?,
                capacity,
            },
        };
        bounds.push(GraphTypeStorageBound { value_type, kind });
    }
    Ok(bounds)
}

pub(super) fn literal_storage_bytes(
    bounds: &[GraphTypeStorageBound],
    value_type: GraphTypeId,
) -> Result<u64, GraphStorageError> {
    let bound = bounds
        .binary_search_by_key(&value_type, |bound| bound.value_type)
        .ok()
        .map(|index| bounds[index])
        .ok_or(GraphStorageError::UnknownType(value_type))?;
    bound
        .maximum_literal_bytes()
        .ok_or(GraphStorageError::RuntimeTypeNested {
            container: value_type,
            runtime_type: value_type,
        })
}

fn storage_shape(
    schema: &GraphSchema,
    value_type: GraphTypeId,
    memo: &mut BTreeMap<GraphTypeId, StorageShape>,
    visiting: &mut BTreeSet<GraphTypeId>,
) -> Result<StorageShape, GraphStorageError> {
    if let Some(shape) = memo.get(&value_type) {
        return Ok(*shape);
    }
    if !visiting.insert(value_type) {
        return Err(GraphStorageError::RecursiveType(value_type));
    }
    let definition = schema
        .value_type(value_type)
        .ok_or(GraphStorageError::UnknownType(value_type))?;
    let rational_body = || -> Result<u64, GraphStorageError> {
        let digits = u64::try_from(schema.limits().maximum_rational_digits)
            .map_err(|_| GraphStorageError::SizeOverflow(value_type))?;
        let one_magnitude = checked_add(value_type, LENGTH_BYTES, digits)?;
        checked_add(
            value_type,
            VARIANT_TAG_BYTES,
            checked_mul(value_type, one_magnitude, 2)?,
        )
    };
    let shape = match definition.kind() {
        TypeKind::Boolean => StorageShape::LiteralBody(1),
        TypeKind::ExactRational { .. } => StorageShape::LiteralBody(rational_body()?),
        TypeKind::MeasurementInterval { .. } => {
            StorageShape::LiteralBody(checked_mul(value_type, rational_body()?, 2)?)
        }
        TypeKind::CanonicalI64 { .. } | TypeKind::CanonicalU64 { .. } => {
            StorageShape::LiteralBody(8)
        }
        TypeKind::Text { maximum_bytes } | TypeKind::Bytes { maximum_bytes } => {
            StorageShape::LiteralBody(checked_add(
                value_type,
                LENGTH_BYTES,
                u64::from(*maximum_bytes),
            )?)
        }
        TypeKind::Array {
            element,
            maximum_items,
        } => {
            let element = require_literal_body(schema, value_type, *element, memo, visiting)?;
            StorageShape::LiteralBody(checked_add(
                value_type,
                LENGTH_BYTES,
                checked_mul(value_type, element, u64::from(*maximum_items))?,
            )?)
        }
        TypeKind::Record { fields } => {
            let mut bytes = LENGTH_BYTES;
            for field in fields {
                let field_body =
                    require_literal_body(schema, value_type, field.value_type(), memo, visiting)?;
                bytes = checked_add(value_type, bytes, checked_add(value_type, 4, field_body)?)?;
            }
            StorageShape::LiteralBody(bytes)
        }
        TypeKind::Option { value } => StorageShape::LiteralBody(checked_add(
            value_type,
            VARIANT_TAG_BYTES,
            require_literal_body(schema, value_type, *value, memo, visiting)?,
        )?),
        TypeKind::Result { ok, error } => {
            let ok = require_literal_body(schema, value_type, *ok, memo, visiting)?;
            let error = require_literal_body(schema, value_type, *error, memo, visiting)?;
            StorageShape::LiteralBody(checked_add(value_type, VARIANT_TAG_BYTES, ok.max(error))?)
        }
        TypeKind::Event { payload, clock } => StorageShape::EventPayload {
            clock: *clock,
            payload_body: require_literal_body(schema, value_type, *payload, memo, visiting)?,
        },
        TypeKind::Stream {
            sample,
            clock,
            capacity,
        } => StorageShape::StreamSample {
            clock: *clock,
            sample_body: require_literal_body(schema, value_type, *sample, memo, visiting)?,
            capacity: *capacity,
        },
        TypeKind::ResourceHandle { .. } => StorageShape::LiteralBody(16 + 32 + 4 + 4),
        TypeKind::JobHandle => StorageShape::LiteralBody(16 + 32 + 32),
    };
    visiting.remove(&value_type);
    memo.insert(value_type, shape);
    Ok(shape)
}

fn require_literal_body(
    schema: &GraphSchema,
    container: GraphTypeId,
    referenced: GraphTypeId,
    memo: &mut BTreeMap<GraphTypeId, StorageShape>,
    visiting: &mut BTreeSet<GraphTypeId>,
) -> Result<u64, GraphStorageError> {
    match storage_shape(schema, referenced, memo, visiting)? {
        StorageShape::LiteralBody(bytes) => Ok(bytes),
        StorageShape::EventPayload { .. } | StorageShape::StreamSample { .. } => {
            Err(GraphStorageError::RuntimeTypeNested {
                container,
                runtime_type: referenced,
            })
        }
    }
}

fn checked_add(owner: GraphTypeId, left: u64, right: u64) -> Result<u64, GraphStorageError> {
    left.checked_add(right)
        .ok_or(GraphStorageError::SizeOverflow(owner))
}

fn checked_mul(owner: GraphTypeId, left: u64, right: u64) -> Result<u64, GraphStorageError> {
    left.checked_mul(right)
        .ok_or(GraphStorageError::SizeOverflow(owner))
}

#[cfg(test)]
mod tests {
    use hyperreal::Rational;

    use super::*;
    use crate::graph::{
        BaseDimensions, GraphLimits, RecordField, RecordFieldId, TypeDefinition, UnitDefinition,
        UnitId,
    };

    const UNIT: UnitId = UnitId::new(1);
    const EXACT: GraphTypeId = GraphTypeId::new(1);
    const TEXT: GraphTypeId = GraphTypeId::new(2);
    const RECORD: GraphTypeId = GraphTypeId::new(3);
    const EVENT: GraphTypeId = GraphTypeId::new(4);
    const STREAM: GraphTypeId = GraphTypeId::new(5);

    fn schema() -> GraphSchema {
        let mut limits = GraphLimits::interactive();
        limits.maximum_rational_digits = 16;
        GraphSchema::try_new(
            limits,
            vec![UnitDefinition::new(
                UNIT,
                "mm",
                BaseDimensions::LENGTH,
                Rational::fraction(1, 1_000).unwrap(),
            )],
            vec![
                TypeDefinition::new(EXACT, "exact.mm", TypeKind::ExactRational { unit: UNIT }),
                TypeDefinition::new(TEXT, "core.text8", TypeKind::Text { maximum_bytes: 8 }),
                TypeDefinition::new(
                    RECORD,
                    "core.record",
                    TypeKind::Record {
                        fields: vec![
                            RecordField::new(RecordFieldId::new(1), "position", EXACT),
                            RecordField::new(RecordFieldId::new(2), "label", TEXT),
                        ],
                    },
                ),
                TypeDefinition::new(
                    EVENT,
                    "event.record",
                    TypeKind::Event {
                        payload: RECORD,
                        clock: GraphClockId::new(1),
                    },
                ),
                TypeDefinition::new(
                    STREAM,
                    "stream.record",
                    TypeKind::Stream {
                        sample: RECORD,
                        clock: GraphClockId::new(1),
                        capacity: 7,
                    },
                ),
            ],
        )
        .unwrap()
    }

    #[test]
    fn exact_composite_event_and_stream_bounds_match_v1_encoding() {
        let bounds = analyze_type_storage(&schema()).unwrap();
        assert_eq!(bounds[0].maximum_literal_bytes(), Some(45));
        assert_eq!(bounds[1].maximum_literal_bytes(), Some(16));
        assert_eq!(bounds[2].maximum_literal_bytes(), Some(69));
        assert_eq!(
            bounds[3].kind(),
            GraphTypeStorageKind::EventPayload {
                clock: GraphClockId::new(1),
                maximum_payload_bytes: 69,
            }
        );
        assert_eq!(
            bounds[4].kind(),
            GraphTypeStorageKind::StreamSample {
                clock: GraphClockId::new(1),
                maximum_sample_bytes: 69,
                capacity: 7,
            }
        );
    }

    #[test]
    fn runtime_types_cannot_hide_inside_literal_composites() {
        let invalid = GraphSchema::try_new(
            GraphLimits::interactive(),
            Vec::new(),
            vec![
                TypeDefinition::new(
                    GraphTypeId::new(1),
                    "event.bool",
                    TypeKind::Event {
                        payload: GraphTypeId::new(2),
                        clock: GraphClockId::new(1),
                    },
                ),
                TypeDefinition::new(GraphTypeId::new(2), "core.bool", TypeKind::Boolean),
                TypeDefinition::new(
                    GraphTypeId::new(3),
                    "bad.optional-event",
                    TypeKind::Option {
                        value: GraphTypeId::new(1),
                    },
                ),
            ],
        )
        .unwrap();
        assert_eq!(
            analyze_type_storage(&invalid),
            Err(GraphStorageError::RuntimeTypeNested {
                container: GraphTypeId::new(3),
                runtime_type: GraphTypeId::new(1),
            })
        );
    }
}
