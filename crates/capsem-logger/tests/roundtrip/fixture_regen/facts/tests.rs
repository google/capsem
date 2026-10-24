use super::*;

#[test]
fn fixture_archive_facts_match_independent_original_index_and_bytes() {
    let recorded = std::fs::read(path()).expect("regenerate the fixture's bounded benchmark facts");
    assert!(recorded.len() <= MAX_TOTAL_BYTES + MAX_RECORDS * 1024);
    let decoded: Vec<Fact> = rmp_serde::from_slice(&recorded).unwrap();
    assert_eq!(decoded, from_source());
    assert_eq!(
        recorded,
        rmp_serde::to_vec_named(&decoded).unwrap(),
        "facts regeneration must be byte reproducible"
    );
}
