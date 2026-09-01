use super::*;

#[test]
fn protocol_aliases_are_normalized() {
    assert_eq!(ModelProtocol::try_from(" Claude ").unwrap(), ModelProtocol::Anthropic);
    assert_eq!(
        ModelProtocol::try_from("openai-compatible").unwrap(),
        ModelProtocol::OpenAi
    );
    assert_eq!(
        ModelProtocol::try_from("openai_compatible").unwrap(),
        ModelProtocol::OpenAi
    );
    assert_eq!(ModelProtocol::try_from("gemini").unwrap(), ModelProtocol::Google);
}

#[test]
fn unknown_protocol_is_rejected_without_guessing_provider_identity() {
    assert_eq!(
        ModelProtocol::try_from("private-vendor").unwrap_err(),
        "unknown model protocol 'private-vendor'"
    );
    assert_eq!(ProviderKind::from_provider_id("private-vendor"), ProviderKind::Unknown);
}

#[test]
fn compatible_protocol_does_not_change_endpoint_owner() {
    assert_eq!(
        ModelProtocol::try_from("openai-compatible").unwrap(),
        ModelProtocol::OpenAi
    );
    assert_eq!(ProviderKind::from_provider_id("ollama"), ProviderKind::Ollama);
}
