use super::*;

fn key(name: &str, qtype: u16) -> LookupKey {
    LookupKey {
        qname: name.to_string(),
        qtype,
        qclass: 1,
    }
}

#[tokio::test]
async fn the_first_query_leads_and_the_rest_follow_its_outcome() {
    let lookups = Arc::new(InFlightLookups::default());
    let Role::Lead(lease) = lookups.join_or_lead(key("a.example", 1)) else {
        panic!("first query must lead");
    };
    let mut followers = Vec::new();
    for _ in 0..50 {
        match lookups.join_or_lead(key("a.example", 1)) {
            Role::Follow(rx) => followers.push(rx),
            Role::Lead(_) => panic!("a second leader for the same key"),
        }
    }
    assert_eq!(lookups.len(), 1);
    let outcome = lease.finish(Ok((vec![1, 2, 3], Duration::from_millis(7))));
    assert!(outcome.is_ok());
    for rx in followers {
        let got = rx.await.expect("every follower is answered");
        assert_eq!(got.as_ref().as_ref().unwrap().0, vec![1, 2, 3]);
    }
    assert_eq!(lookups.len(), 0, "the key is released once settled");
}

#[tokio::test]
async fn different_type_or_class_or_name_is_a_different_lookup() {
    let lookups = Arc::new(InFlightLookups::default());
    let leases: Vec<LeaderLease> = [key("a.example", 1), key("a.example", 28), key("b.example", 1)]
        .into_iter()
        .chain(std::iter::once(LookupKey {
            qname: "a.example".into(),
            qtype: 1,
            qclass: 3,
        }))
        .map(|k| match lookups.join_or_lead(k) {
            Role::Lead(lease) => lease,
            Role::Follow(_) => panic!("distinct keys must each lead"),
        })
        .collect();
    assert_eq!(lookups.len(), 4);
    drop(leases);
    assert_eq!(lookups.len(), 0);
}

#[tokio::test]
async fn a_cancelled_leader_fails_its_followers_instead_of_hanging_them() {
    let lookups = Arc::new(InFlightLookups::default());
    let Role::Lead(lease) = lookups.join_or_lead(key("gone.example", 1)) else {
        panic!("first query must lead");
    };
    let Role::Follow(rx) = lookups.join_or_lead(key("gone.example", 1)) else {
        panic!("second query must follow");
    };
    drop(lease); // the leading task was cancelled before it could finish
    let got = tokio::time::timeout(Duration::from_secs(1), rx)
        .await
        .expect("follower must not hang")
        .expect("follower is told");
    assert!(got.is_err(), "an abandoned lookup is a failure, not an answer");
    assert_eq!(lookups.len(), 0);
    // And the key is free for the next query to lead.
    assert!(matches!(lookups.join_or_lead(key("gone.example", 1)), Role::Lead(_)));
}

#[tokio::test]
async fn a_follower_that_gave_up_does_not_break_the_others() {
    let lookups = Arc::new(InFlightLookups::default());
    let Role::Lead(lease) = lookups.join_or_lead(key("slow.example", 1)) else {
        panic!("lead");
    };
    let Role::Follow(quitter) = lookups.join_or_lead(key("slow.example", 1)) else {
        panic!("follow");
    };
    let Role::Follow(patient) = lookups.join_or_lead(key("slow.example", 1)) else {
        panic!("follow");
    };
    drop(quitter);
    lease.finish(Err("upstream timed out".into()));
    assert!(patient.await.unwrap().is_err());
}
