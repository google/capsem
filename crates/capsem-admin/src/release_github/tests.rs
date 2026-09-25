use super::*;

const IDENTITY: &str = "profile-stable-code-0.6.2";
const EARLIER: &str = "2368c2a80280a6ae1cff5917a8f62fbdab09c7f2";
const MAIN: &str = "0da05c6bce4807bf7f73398d14ab7eee6b525198";

struct Existing(Option<&'static str>);

impl ReleaseLookup for Existing {
    fn release_commit(&mut self, tag: &str) -> Result<Option<String>> {
        assert_eq!(tag, IDENTITY);
        Ok(self.0.map(str::to_string))
    }
}

struct Unreachable;

impl ReleaseLookup for Unreachable {
    fn release_commit(&mut self, _tag: &str) -> Result<Option<String>> {
        Err(anyhow!("gh: could not resolve host"))
    }
}

fn commit(sha: &str) -> SourceCommit {
    sha.parse().unwrap()
}

#[test]
fn an_unpublished_identity_is_free() {
    ensure_publication_identity_is_free(&mut Existing(None), IDENTITY, &commit(MAIN)).unwrap();
}

#[test]
fn the_same_commit_resumes_its_own_publication() {
    ensure_publication_identity_is_free(&mut Existing(Some(MAIN)), IDENTITY, &commit(MAIN)).unwrap();
}

#[test]
fn an_identity_published_from_another_commit_is_refused_before_any_build() {
    let error = ensure_publication_identity_is_free(&mut Existing(Some(EARLIER)), IDENTITY, &commit(MAIN))
        .unwrap_err()
        .to_string();
    assert!(error.contains(EARLIER), "{error}");
    assert!(error.contains("advance the profile's `revision`"), "{error}");
}

#[test]
fn a_lookup_that_cannot_answer_is_not_taken_as_free() {
    assert!(ensure_publication_identity_is_free(&mut Unreachable, IDENTITY, &commit(MAIN)).is_err());
}
