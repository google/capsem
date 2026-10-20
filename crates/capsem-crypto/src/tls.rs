//! One TLS provider and certificate/key implementation. SNI policy, cache
//! capacity, and hostname normalization remain with the proxy's domain owner.
use rcgen::{CertificateParams, IsCa, KeyPair, SanType};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::sign::CertifiedKey;

pub fn provider() -> rustls::crypto::CryptoProvider {
    rustls::crypto::aws_lc_rs::default_provider()
}

pub struct CertificateIssuer {
    certificate: rcgen::Certificate,
    key: KeyPair,
    der: CertificateDer<'static>,
}

impl CertificateIssuer {
    pub fn load(key_pem: &str, cert_pem: &str) -> anyhow::Result<Self> {
        let key = KeyPair::from_pem(key_pem)?;
        let certificate = CertificateParams::from_ca_cert_pem(cert_pem)?.self_signed(&key)?;
        let der = CertificateDer::from(certificate.der().to_vec());
        Ok(Self { certificate, key, der })
    }

    pub fn certificate_der(&self) -> &CertificateDer<'static> {
        &self.der
    }

    /// Preserve the existing proxy's P-256, SAN, serverAuth and validity contract.
    pub fn mint_server_certificate(&self, domain: &str) -> anyhow::Result<CertifiedKey> {
        let key = KeyPair::generate()?;
        let mut params = CertificateParams::new(vec![domain.to_string()])?;
        params.distinguished_name.push(rcgen::DnType::CommonName, domain);
        params.subject_alt_names = vec![SanType::DnsName(domain.try_into()?)];
        params.not_before = time::Date::from_calendar_date(2026, time::Month::January, 1)
            .unwrap()
            .midnight()
            .assume_utc();
        params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(365);
        params.is_ca = IsCa::NoCa;
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        let certificate = params.signed_by(&key, &self.certificate, &self.key)?;
        let chain = vec![CertificateDer::from(certificate.der().to_vec()), self.der.clone()];
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
        let signing_key = rustls::crypto::aws_lc_rs::sign::any_supported_type(&key_der)?;
        Ok(CertifiedKey::new(chain, signing_key))
    }
}

/// Hermetic mock-server certificate; production authority uses CertificateIssuer.
pub fn self_signed(names: Vec<String>) -> anyhow::Result<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
    let certificate = rcgen::generate_simple_self_signed(names)?;
    Ok((
        CertificateDer::from(certificate.cert.der().to_vec()),
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(certificate.key_pair.serialize_der())),
    ))
}

#[cfg(test)]
mod tests;
