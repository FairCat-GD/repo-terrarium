//! The GitHub token, kept in the OS keychain and nowhere else: the macOS keychain or the
//! Windows credential manager, under the app's bundle id. It is read only to ask GitHub, and
//! never written to the settings, a log or an error.

use std::fmt;

/// A GitHub token. It has no `Display` and no serde, and its `Debug` says nothing of it.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// The token as pasted: surrounding space is ignored; empty, or holding space or a
    /// control character inside, it is no token.
    pub fn new(text: &str) -> Option<Token> {
        let text = text.trim();
        (!text.is_empty() && !text.chars().any(|c| c.is_whitespace() || c.is_control()))
            .then(|| Token(text.to_owned()))
    }

    pub(crate) fn secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(..)")
    }
}

/// Why the keychain could not be used. It carries nothing the keychain said: an error from it
/// can hold the secret's bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeychainError {
    /// The system has no keychain this build can use.
    Unavailable,
    /// The keychain is locked, or access to the item was denied.
    Refused,
    /// Anything else it reported.
    Failed,
}

/// Where the token is kept.
pub trait Keychain: Send + Sync {
    /// The token saved, if one is.
    fn token(&self) -> Result<Option<Token>, KeychainError>;
    fn save(&self, token: &Token) -> Result<(), KeychainError>;
    /// Removes the token; removing none is not an error.
    fn remove(&self) -> Result<(), KeychainError>;
}

/// The OS keychain's item for `service` and `account` (the service is the bundle id).
pub struct NativeKeychain {
    service: String,
    account: String,
}

impl NativeKeychain {
    pub fn new(service: &str, account: &str) -> NativeKeychain {
        NativeKeychain {
            service: service.to_owned(),
            account: account.to_owned(),
        }
    }

    fn entry(&self) -> Result<keyring::v1::Entry, KeychainError> {
        keyring::v1::Entry::new(&self.service, &self.account).map_err(|err| failure(&err))
    }
}

/// The keychain's error, reduced to what may be shown: never its payload.
fn failure(err: &keyring::v1::Error) -> KeychainError {
    match err {
        keyring::v1::Error::NoStorageAccess(_) => KeychainError::Refused,
        keyring::v1::Error::NoDefaultStore => KeychainError::Unavailable,
        _ => KeychainError::Failed,
    }
}

impl Keychain for NativeKeychain {
    fn token(&self) -> Result<Option<Token>, KeychainError> {
        match self.entry()?.get_password() {
            Ok(text) => Ok(Token::new(&text)),
            Err(keyring::v1::Error::NoEntry) => Ok(None),
            Err(err) => Err(failure(&err)),
        }
    }

    fn save(&self, token: &Token) -> Result<(), KeychainError> {
        self.entry()?
            .set_password(token.secret())
            .map_err(|err| failure(&err))
    }

    fn remove(&self) -> Result<(), KeychainError> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::v1::Error::NoEntry) => Ok(()),
            Err(err) => Err(failure(&err)),
        }
    }
}

/// A keychain in memory, for tests and harnesses: they never touch the person's keychain. It
/// counts its reads, so a test can say none happened.
#[cfg(any(test, feature = "fixture"))]
#[derive(Default)]
pub struct MemoryKeychain {
    token: std::sync::Mutex<Option<Token>>,
    reads: std::sync::atomic::AtomicUsize,
}

#[cfg(any(test, feature = "fixture"))]
impl MemoryKeychain {
    pub fn holding(token: &str) -> MemoryKeychain {
        let keychain = MemoryKeychain::default();
        *keychain.lock() = Token::new(token);
        keychain
    }

    pub fn reads(&self) -> usize {
        self.reads.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The token held, as a test would check it was saved.
    pub fn held(&self) -> Option<String> {
        self.lock().as_ref().map(|token| token.secret().to_owned())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Token>> {
        self.token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(any(test, feature = "fixture"))]
impl Keychain for MemoryKeychain {
    fn token(&self) -> Result<Option<Token>, KeychainError> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(self.lock().clone())
    }

    fn save(&self, token: &Token) -> Result<(), KeychainError> {
        *self.lock() = Some(token.clone());
        Ok(())
    }

    fn remove(&self) -> Result<(), KeychainError> {
        *self.lock() = None;
        Ok(())
    }
}
