use super::*;

#[test]
fn messages_round_trip_binary_bodies_without_collector_authority() {
    let request = ProxyMetricRequest { body: vec![0, 0xff, 7] };
    let encoded = rmp_serde::to_vec_named(&request).unwrap();
    assert_eq!(rmp_serde::from_slice::<ProxyMetricRequest>(&encoded).unwrap(), request);

    let response = ProxyMetricResponse::Relayed {
        status: 202,
        content_type: Some("application/x-protobuf".into()),
        body: vec![9, 0, 8],
    };
    let encoded = rmp_serde::to_vec_named(&response).unwrap();
    assert_eq!(
        rmp_serde::from_slice::<ProxyMetricResponse>(&encoded).unwrap(),
        response
    );

    let hello = ProxyMetricBrokerMessage::Hello {
        session_id: "vm-a".into(),
    };
    let encoded = rmp_serde::to_vec_named(&hello).unwrap();
    assert_eq!(
        rmp_serde::from_slice::<ProxyMetricBrokerMessage>(&encoded).unwrap(),
        hello
    );
}
