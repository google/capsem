# OAuth signing fixtures

The DER key is a generated, public, test-only RSA signing key. It grants no
access to any account or service. Never use it outside hermetic tests.
The matching JSON file contains its public JWK. Credential tests sign actual
RS256 tokens and exercise exchange and key retrieval over temporary local TCP.
