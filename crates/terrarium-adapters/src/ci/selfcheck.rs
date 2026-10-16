//! An offline look at the TLS the GitHub client speaks, for the probe: the same rustls on ring
//! and the same root list, exercised without a socket (I-6). ring's C and assembly are built for
//! each target, and a probe on Windows is the first place they run there.

use std::sync::Arc;

use rustls::crypto::ring::default_provider;
use rustls::crypto::{CryptoProvider, SupportedKxGroup};
use rustls::{ClientConfig, RootCertStore};

/// What the check found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent checks, each passed or not"
)]
pub struct TlsCheck {
    /// ring's random source filled a buffer, and not with zeros.
    pub random: bool,
    /// Two key exchanges of the provider's first group (X25519) reached the same secret.
    pub exchange: bool,
    /// A client configuration was built from the provider and the root list.
    pub config: bool,
    /// How many roots the compiled-in list holds.
    pub roots: usize,
    /// Every check passed.
    pub passed: bool,
}

/// Runs the check. It opens nothing and asks no one.
pub fn tls_check() -> TlsCheck {
    let provider = default_provider();
    let random = random_fills(&provider);
    let exchange = provider
        .kx_groups
        .first()
        .is_some_and(|group| agrees(*group));
    let roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let count = roots.len();
    let config = ClientConfig::builder_with_provider(Arc::new(provider))
        .with_safe_default_protocol_versions()
        .map(|builder| builder.with_root_certificates(roots).with_no_client_auth())
        .is_ok();
    TlsCheck {
        random,
        exchange,
        config,
        roots: count,
        passed: random && exchange && config && count > 0,
    }
}

fn random_fills(provider: &CryptoProvider) -> bool {
    let mut bytes = [0_u8; 32];
    provider.secure_random.fill(&mut bytes).is_ok() && bytes.iter().any(|&b| b != 0)
}

/// Two parties of `group` exchange public keys and must reach one secret.
fn agrees(group: &dyn SupportedKxGroup) -> bool {
    let (Ok(ours), Ok(theirs)) = (group.start(), group.start()) else {
        return false;
    };
    let (our_public, their_public) = (ours.pub_key().to_vec(), theirs.pub_key().to_vec());
    match (ours.complete(&their_public), theirs.complete(&our_public)) {
        (Ok(a), Ok(b)) => a.secret_bytes() == b.secret_bytes() && !a.secret_bytes().is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tls_stack_starts_exchanges_keys_and_loads_its_roots_offline() {
        let check = tls_check();
        assert!(check.random && check.exchange && check.config, "{check:?}");
        assert!(check.roots > 100, "{check:?}");
        assert!(check.passed);
    }
}
