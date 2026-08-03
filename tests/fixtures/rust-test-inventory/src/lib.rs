//! Inventory fixture library.
//!
//! ```
//! assert_eq!(rust_test_inventory_fixture::answer(), 42);
//! ```

#[must_use]
pub const fn answer() -> u8 {
    42
}

#[cfg(test)]
mod tests;
