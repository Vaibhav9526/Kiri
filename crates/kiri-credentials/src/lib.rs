use async_trait::async_trait;
use std::{collections::HashMap, sync::Arc};
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("credential store error: {0}")]
    Store(String),
}

#[async_trait]
pub trait CredentialStore: Send + Sync {
    async fn set(&self, key: &str, secret: &str) -> Result<(), CredentialError>;
    async fn get(&self, key: &str) -> Result<Option<String>, CredentialError>;
    async fn delete(&self, key: &str) -> Result<(), CredentialError>;
}

#[derive(Clone, Default)]
pub struct MemoryCredentialStore {
    values: Arc<RwLock<HashMap<String, String>>>,
}

#[async_trait]
impl CredentialStore for MemoryCredentialStore {
    async fn set(&self, key: &str, secret: &str) -> Result<(), CredentialError> {
        self.values.write().await.insert(key.into(), secret.into());
        Ok(())
    }
    async fn get(&self, key: &str) -> Result<Option<String>, CredentialError> {
        Ok(self.values.read().await.get(key).cloned())
    }
    async fn delete(&self, key: &str) -> Result<(), CredentialError> {
        self.values.write().await.remove(key);
        Ok(())
    }
}

pub struct WindowsCredentialStore {
    service: String,
}
impl WindowsCredentialStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }
    fn entry(&self, key: &str) -> Result<keyring::Entry, CredentialError> {
        keyring::Entry::new(&self.service, key).map_err(|e| CredentialError::Store(e.to_string()))
    }
}

#[async_trait]
impl CredentialStore for WindowsCredentialStore {
    async fn set(&self, key: &str, secret: &str) -> Result<(), CredentialError> {
        self.entry(key)?
            .set_password(secret)
            .map_err(|e| CredentialError::Store(e.to_string()))
    }
    async fn get(&self, key: &str) -> Result<Option<String>, CredentialError> {
        match self.entry(key)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(CredentialError::Store(e.to_string())),
        }
    }
    async fn delete(&self, key: &str) -> Result<(), CredentialError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(CredentialError::Store(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn memory_store_round_trip() {
        let store = MemoryCredentialStore::default();
        store.set("provider", "secret").await.unwrap();
        assert_eq!(
            store.get("provider").await.unwrap().as_deref(),
            Some("secret")
        );
        store.delete("provider").await.unwrap();
        assert!(store.get("provider").await.unwrap().is_none());
    }
}
