use super::*;

#[test]
fn data_header_preserves_flow_identity_and_connection_status() {
    let flow = FlowKey {
        generation: 0xabcd,
        id: 42,
    };
    for connected in [false, true] {
        let header = flow.data_header(connected);
        assert_eq!(header.len(), 17);
        assert_eq!(FlowKey::read_data_header(header).unwrap(), (flow, connected));
    }
}

#[test]
fn data_header_refuses_missing_identity_and_unknown_status() {
    for flow in [FlowKey { generation: 0, id: 1 }, FlowKey { generation: 1, id: 0 }] {
        assert!(!flow.is_valid());
        assert!(FlowKey::read_data_header(flow.data_header(true)).is_err());
    }
    let mut header = FlowKey { generation: 1, id: 1 }.data_header(true);
    header[16] = 2;
    assert!(FlowKey::read_data_header(header).is_err());
}
