/// Match Serde's `default` decoder with omission of the same default on the
/// named MessagePack wire. The bound keeps the rule type-directed: an empty
/// value is omitted only where the field's declared default is actually empty.
pub(crate) fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    value == &T::default()
}

pub(crate) fn is_true(value: &bool) -> bool {
    *value
}
