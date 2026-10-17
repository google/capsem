//! Secret lifetime and comparisons use the audited primitives owned here.
pub use zeroize::Zeroizing;

pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    bool::from(left.ct_eq(right))
}
