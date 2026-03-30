//! # File Sharing
//!
//! File sharing via the [`BlobStore`](willow_network::BlobStore) trait.
//! Files are added to the blob store and their hash is broadcast over gossip.
//! Receivers download via `blobs().get(hash)`.

use anyhow::Result;
use willow_network::BlobStore;

/// Share a file by adding it to the blob store.
///
/// Returns the content hash. The caller should broadcast the hash
/// (along with filename and metadata) over gossip so peers can download.
pub async fn share_file(blobs: &dyn BlobStore, data: Vec<u8>) -> Result<iroh_blobs::Hash> {
    blobs.add(bytes::Bytes::from(data)).await
}

/// Download a file from the blob store by hash.
///
/// Returns `None` if the blob is not available.
pub async fn download_file(
    blobs: &dyn BlobStore,
    hash: iroh_blobs::Hash,
) -> Result<Option<Vec<u8>>> {
    Ok(blobs.get(hash).await?.map(|b| b.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_network::mem::{MemHub, MemNetwork};
    use willow_network::Network;

    #[tokio::test]
    async fn share_and_download_round_trip() {
        let hub = MemHub::new();
        let net = MemNetwork::new(&hub);

        let data = b"hello file sharing!".to_vec();
        let hash = share_file(net.blobs(), data.clone()).await.unwrap();

        let downloaded = download_file(net.blobs(), hash).await.unwrap().unwrap();
        assert_eq!(downloaded, data);
    }

    #[tokio::test]
    async fn download_missing_returns_none() {
        let hub = MemHub::new();
        let net = MemNetwork::new(&hub);

        let fake_hash = iroh_blobs::Hash::new(b"nonexistent");
        let result = download_file(net.blobs(), fake_hash).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn share_multiple_files() {
        let hub = MemHub::new();
        let net = MemNetwork::new(&hub);

        let h1 = share_file(net.blobs(), b"file one".to_vec()).await.unwrap();
        let h2 = share_file(net.blobs(), b"file two".to_vec()).await.unwrap();

        assert_ne!(h1, h2);
        assert_eq!(
            download_file(net.blobs(), h1).await.unwrap().unwrap(),
            b"file one"
        );
        assert_eq!(
            download_file(net.blobs(), h2).await.unwrap().unwrap(),
            b"file two"
        );
    }
}
