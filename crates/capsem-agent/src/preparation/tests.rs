use super::*;

#[test]
fn startup_arguments_require_a_deadline_script_and_literal_entrypoint() {
    let args = ["1", "printf 'approved script'", "--", "/entrypoint", "a b", "--help"].map(String::from);
    let parsed = Input::parse(&args).unwrap();
    assert_eq!(parsed.startup, Some((Duration::from_secs(1), args[1].as_str())));
    assert_eq!(parsed.argv, &args[3..]);
    for invalid in [
        vec![],
        vec!["0", "x", "--", "/bin/true"],
        vec!["1", "x", "--"],
        vec!["1", "x", "other", "/bin/true"],
    ] {
        let invalid: Vec<String> = invalid.into_iter().map(String::from).collect();
        assert!(Input::parse(&invalid).is_err());
    }
}
