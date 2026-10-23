//! Borrowed schema preflight bounds allocation and forbids hidden protobuf fields.
//!
//! Prost's normal forward-compatible unknown/duplicate handling is unsuitable
//! when the original bytes will be forwarded across a confined capability.
use prost::encoding::{decode_key, decode_varint, WireType};

use super::{AdmissionError, MAX_POINTS};

type Result<T> = std::result::Result<T, AdmissionError>;

#[derive(Clone, Copy)]
enum Node {
    Root,
    ResourceMetrics,
    Resource,
    ScopeMetrics,
    Scope,
    Metric,
    Sum,
    Histogram,
    NumberPoint,
    HistogramPoint,
    Attribute,
    Value,
}

#[derive(Clone, Copy)]
enum Field {
    Message(Node),
    Bytes(usize),
    Varint,
    Fixed64,
    Packed64(usize),
}

impl Node {
    // (wire shape, occurrence cap, oneof group). Tags not listed fail closed.
    fn field(self, tag: u32) -> Result<(Field, usize, bool)> {
        use Field::*;
        use Node::*;
        let entry = match (self, tag) {
            (Root, 1) => (Message(ResourceMetrics), 1, false),
            (ResourceMetrics, 1) => (Message(Resource), 1, false),
            (ResourceMetrics, 2) => (Message(ScopeMetrics), 1, false),
            (ResourceMetrics | ScopeMetrics, 3) => (Bytes(0), 1, false),
            (Resource, 1) => (Message(Attribute), 4, false),
            (Resource, 2) => (Varint, 1, false),
            (ScopeMetrics, 1) => (Message(Scope), 1, false),
            (ScopeMetrics, 2) => (Message(Metric), 4, false),
            (Scope, 1) => (Bytes(32), 1, false),
            (Scope, 2) => (Bytes(0), 1, false),
            (Scope, 4) => (Varint, 1, false),
            (Metric, 1) => (Bytes(64), 1, false),
            (Metric, 2) => (Bytes(256), 1, false),
            (Metric, 3) => (Bytes(16), 1, false),
            (Metric, 7) => (Message(Sum), 1, true),
            (Metric, 9) => (Message(Histogram), 1, true),
            (Sum, 1) => (Message(NumberPoint), MAX_POINTS, false),
            (Histogram, 1) => (Message(HistogramPoint), MAX_POINTS, false),
            (Sum | Histogram, 2) | (Sum, 3) => (Varint, 1, false),
            (NumberPoint, 7) | (HistogramPoint, 9) => (Message(Attribute), 5, false),
            (NumberPoint | HistogramPoint, 2 | 3) => (Fixed64, 1, false),
            (NumberPoint, 4 | 6) => (Fixed64, 1, true),
            (NumberPoint, 8) | (HistogramPoint, 10) => (Varint, 1, false),
            (HistogramPoint, 4 | 5 | 11 | 12) => (Fixed64, 1, false),
            (HistogramPoint, 6) => (Packed64(65), 1, false),
            (HistogramPoint, 7) => (Packed64(64), 1, false),
            (Attribute, 1) => (Bytes(32), 1, false),
            (Attribute, 2) => (Message(Value), 1, false),
            (Value, 1) => (Bytes(192), 1, false),
            _ => return Err(AdmissionError),
        };
        Ok(entry)
    }
}

pub(super) fn check(input: &[u8]) -> Result<()> {
    let mut budget = Budget { points: 0, fields: 0 };
    message(input, Node::Root, &mut budget)
}

struct Budget {
    points: usize,
    fields: usize,
}

fn message(mut input: &[u8], node: Node, budget: &mut Budget) -> Result<()> {
    if matches!(node, Node::NumberPoint | Node::HistogramPoint) {
        budget.points += 1;
        if budget.points > MAX_POINTS {
            return Err(AdmissionError);
        }
    }
    let mut counts = [0_usize; 16];
    while !input.is_empty() {
        budget.fields += 1;
        if budget.fields > 131_072 {
            return Err(AdmissionError);
        }
        let (tag, wire) = decode_key(&mut input).map_err(|_| AdmissionError)?;
        let (shape, cap, oneof) = node.field(tag)?;
        let slot = usize::try_from(tag).map_err(|_| AdmissionError)?;
        counts[slot] += 1;
        if counts[slot] > cap {
            return Err(AdmissionError);
        }
        if oneof {
            counts[15] += 1;
            if counts[15] > 1 {
                return Err(AdmissionError);
            }
        }
        match (shape, wire) {
            (Field::Varint, WireType::Varint) => {
                decode_varint(&mut input).map_err(|_| AdmissionError)?;
            }
            (Field::Fixed64, WireType::SixtyFourBit) => {
                take(&mut input, 8)?;
            }
            (Field::Message(child), WireType::LengthDelimited) => {
                let child_bytes = delimited(&mut input)?;
                message(child_bytes, child, budget)?;
            }
            (Field::Bytes(max), WireType::LengthDelimited) => {
                if delimited(&mut input)?.len() > max {
                    return Err(AdmissionError);
                }
            }
            (Field::Packed64(max), WireType::LengthDelimited) => {
                let bytes = delimited(&mut input)?;
                if bytes.len() % 8 != 0 || bytes.len() / 8 > max {
                    return Err(AdmissionError);
                }
            }
            _ => return Err(AdmissionError),
        }
    }
    Ok(())
}

fn delimited<'a>(input: &mut &'a [u8]) -> Result<&'a [u8]> {
    let length = usize::try_from(decode_varint(input).map_err(|_| AdmissionError)?).map_err(|_| AdmissionError)?;
    take(input, length)
}

fn take<'a>(input: &mut &'a [u8], length: usize) -> Result<&'a [u8]> {
    let (taken, rest) = input.split_at_checked(length).ok_or(AdmissionError)?;
    *input = rest;
    Ok(taken)
}
