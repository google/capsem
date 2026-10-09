use super::*;

#[test]
fn requests_and_correlated_answers_are_bounded() {
    let address = ProxyPrivateNameRequest::AddressOf {
        request_id: 1,
        name: "box.dev.capsem.internal".to_string(),
    };
    let name = ProxyPrivateNameRequest::NameOf {
        request_id: 2,
        address: Ipv4Addr::new(10, 0, 0, 2),
    };
    address.validate().unwrap();
    name.validate().unwrap();
    assert_eq!(address.request_id(), 1);
    assert_eq!(name.request_id(), 2);
    assert_eq!(
        ProxyPrivateNameResponse::Address {
            request_id: 1,
            address: Ipv4Addr::new(10, 0, 0, 2),
        }
        .request_id(),
        1
    );
}

#[test]
fn malformed_authority_and_oversized_values_fail_closed() {
    assert!(ProxyPrivateNameRequest::AddressOf {
        request_id: 0,
        name: String::new(),
    }
    .validate()
    .is_err());
    assert!(ProxyPrivateNameRequest::AddressOf {
        request_id: 1,
        name: "x".repeat(MAX_PRIVATE_NAME_BYTES + 1),
    }
    .validate()
    .is_err());
    assert!(ProxyPrivateNameResponse::rejected(1, "x".repeat(MAX_PRIVATE_NAME_ERROR_BYTES + 1)).is_err());
}
