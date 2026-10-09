//! JWT-library adapter to the ring verifier already shipped by the workspace.
//! No token signing or arbitrary key conversion is available in production.

use std::sync::OnceLock;

use jsonwebtoken::crypto::{CryptoProvider, JwtVerifier, KeyUtils};
use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{Algorithm, DecodingKeyKind};
use ring::signature::{RsaPublicKeyComponents, RSA_PKCS1_2048_8192_SHA256};
use signature::Verifier;

use super::OAuthIdentityError;

static READY: OnceLock<bool> = OnceLock::new();
static PROVIDER: CryptoProvider = CryptoProvider {
    signer_factory: |_, _| Err(ErrorKind::UnsupportedAlgorithm.into()),
    verifier_factory: |algorithm, key| {
        if *algorithm != Algorithm::RS256 {
            return Err(ErrorKind::UnsupportedAlgorithm.into());
        }
        match key.kind() {
            DecodingKeyKind::RsaModulusExponent { n, e } => Ok(Box::new(RingVerifier {
                n: n.clone(),
                e: e.clone(),
            })),
            _ => Err(ErrorKind::InvalidKeyFormat.into()),
        }
    },
    key_utils: KeyUtils {
        rsa_pub_components_from_private_key: |_| Err(ErrorKind::UnsupportedAlgorithm.into()),
        rsa_pub_components_from_public_key: |_| Err(ErrorKind::UnsupportedAlgorithm.into()),
        ec_pub_components_from_private_key: |_, _| Err(ErrorKind::UnsupportedAlgorithm.into()),
        ed_pub_components_from_private_key: |_, _| Err(ErrorKind::UnsupportedAlgorithm.into()),
        compute_digest: |_, _| Err(ErrorKind::UnsupportedAlgorithm.into()),
    },
};

pub(super) fn install() -> Result<(), OAuthIdentityError> {
    // Refuse a conflicting process-level provider; never silently inherit one.
    if *READY.get_or_init(|| PROVIDER.install_default().is_ok()) {
        Ok(())
    } else {
        Err(OAuthIdentityError::CryptoUnavailable)
    }
}

struct RingVerifier {
    n: Vec<u8>,
    e: Vec<u8>,
}
impl Verifier<Vec<u8>> for RingVerifier {
    fn verify(&self, message: &[u8], signature: &Vec<u8>) -> Result<(), signature::Error> {
        RsaPublicKeyComponents {
            n: self.n.as_slice(),
            e: self.e.as_slice(),
        }
        .verify(&RSA_PKCS1_2048_8192_SHA256, message, signature)
        .map_err(|_| signature::Error::new())
    }
}
impl JwtVerifier for RingVerifier {
    fn algorithm(&self) -> Algorithm {
        Algorithm::RS256
    }
}
