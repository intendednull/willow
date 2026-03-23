//! # File Manager
//!
//! Manages file sharing and downloading. Lives alongside the network task
//! and handles chunk requests automatically.

use std::collections::HashMap;

use willow_files::{split_file_default, Chunk, ChunkStore, ContentHash, FileManifest};
use willow_transport::{pack_envelope, MessageType};

/// Manages locally shared files and incoming downloads.
pub struct FileManager {
    /// Chunks we're serving (from files we've shared).
    local_chunks: HashMap<ContentHash, Vec<u8>>,
    /// Active downloads from other peers.
    downloads: ChunkStore,
    /// Manifests of files we've shared (for metadata lookup).
    shared_manifests: Vec<FileManifest>,
}

/// Events emitted by the FileManager to the UI.
#[derive(Debug, Clone)]
pub enum FileEvent {
    /// A file manifest was received from a peer.
    FileAnnounced {
        manifest: FileManifest,
        from: String,
    },
    /// A file download completed.
    FileDownloaded {
        manifest: FileManifest,
        data: Vec<u8>,
    },
    /// Download progress update.
    DownloadProgress {
        file_hash: ContentHash,
        received: usize,
        total: usize,
    },
}

impl FileManager {
    pub fn new() -> Self {
        Self {
            local_chunks: HashMap::new(),
            downloads: ChunkStore::new(),
            shared_manifests: Vec::new(),
        }
    }

    /// Share a file: split it into chunks, store locally, return the manifest
    /// (serialized as an envelope for gossipsub broadcast).
    pub fn share_file(
        &mut self,
        data: &[u8],
        filename: String,
        mime_type: String,
    ) -> Option<(FileManifest, Vec<u8>)> {
        let (manifest, chunks) = split_file_default(data, &filename, &mime_type);

        // Store all chunks locally so we can serve requests.
        for chunk in &chunks {
            self.local_chunks
                .insert(chunk.hash.clone(), chunk.data.clone());
        }

        self.shared_manifests.push(manifest.clone());

        // Serialize the manifest for gossipsub broadcast.
        let envelope = pack_envelope(MessageType::File, &manifest).ok()?;
        Some((manifest, envelope))
    }

    /// Look up a chunk by hash (for responding to chunk requests).
    pub fn get_chunk(&self, hash: &ContentHash) -> Option<&[u8]> {
        self.local_chunks.get(hash).map(|v| v.as_slice())
    }

    /// Register a received file manifest and start tracking the download.
    pub fn register_manifest(&mut self, manifest: FileManifest) {
        self.downloads.register_manifest(manifest);
    }

    /// Add a received chunk. Returns true if the chunk was needed.
    pub fn add_chunk(&mut self, hash: ContentHash, data: Vec<u8>) -> bool {
        let chunk = Chunk {
            hash: hash.clone(),
            data: data.clone(),
        };
        let needed = self.downloads.add_chunk(chunk);
        if needed {
            // Also store locally so we can seed to other peers.
            self.local_chunks.insert(hash, data);
        }
        needed
    }

    /// Check if a file download is complete.
    pub fn is_download_complete(&self, file_hash: &ContentHash) -> bool {
        self.downloads.is_complete(file_hash)
    }

    /// Try to assemble a completed download.
    pub fn try_assemble(&mut self, file_hash: &ContentHash) -> Option<Vec<u8>> {
        self.downloads.try_assemble(file_hash).ok()
    }

    /// Get the hashes of chunks we still need for a file.
    pub fn missing_chunks(&self, file_hash: &ContentHash) -> Vec<ContentHash> {
        self.downloads.missing_hashes(file_hash)
    }

    /// Get download progress (received, total) for a file.
    pub fn download_progress(&self, file_hash: &ContentHash) -> Option<(usize, usize)> {
        let missing = self.downloads.missing_count(file_hash);
        let pending = self.downloads.pending_files();
        pending
            .iter()
            .find(|m| m.file_hash == *file_hash)
            .map(|m| (m.chunk_hashes.len() - missing, m.chunk_hashes.len()))
    }
}

impl Default for FileManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_file_stores_chunks_and_returns_manifest() {
        let mut mgr = FileManager::new();
        let data = b"hello file sharing!";

        let (manifest, envelope) = mgr
            .share_file(data, "test.txt".into(), "text/plain".into())
            .expect("share should succeed");

        assert_eq!(manifest.filename, "test.txt");
        assert_eq!(manifest.mime_type, "text/plain");
        assert_eq!(manifest.total_size, data.len() as u64);
        assert!(!envelope.is_empty());

        // All chunks should be stored locally.
        for hash in &manifest.chunk_hashes {
            assert!(mgr.get_chunk(hash).is_some());
        }
    }

    #[test]
    fn get_chunk_returns_none_for_unknown() {
        let mgr = FileManager::new();
        let hash = ContentHash::of(b"nonexistent");
        assert!(mgr.get_chunk(&hash).is_none());
    }

    #[test]
    fn download_flow_register_add_assemble() {
        let data = b"download me!";
        let (manifest, chunks) = willow_files::split_file(data, "dl.txt", "text/plain", 8);
        let file_hash = manifest.file_hash.clone();

        let mut mgr = FileManager::new();
        mgr.register_manifest(manifest);

        assert!(!mgr.is_download_complete(&file_hash));
        assert_eq!(mgr.download_progress(&file_hash), Some((0, 2)));
        assert_eq!(mgr.missing_chunks(&file_hash).len(), 2);

        // Add chunks.
        for chunk in chunks {
            assert!(mgr.add_chunk(chunk.hash, chunk.data));
        }

        assert!(mgr.is_download_complete(&file_hash));
        assert_eq!(mgr.download_progress(&file_hash), Some((2, 2)));

        let assembled = mgr.try_assemble(&file_hash).expect("should assemble");
        assert_eq!(assembled, data);
    }

    #[test]
    fn add_chunk_returns_false_for_unneeded() {
        let mut mgr = FileManager::new();
        let hash = ContentHash::of(b"random");
        assert!(!mgr.add_chunk(hash, b"random".to_vec()));
    }

    #[test]
    fn received_chunks_stored_for_seeding() {
        let data = b"seed me!";
        let (manifest, chunks) = willow_files::split_file_default(data, "seed.txt", "text/plain");

        let mut mgr = FileManager::new();
        mgr.register_manifest(manifest);

        let hash = chunks[0].hash.clone();
        mgr.add_chunk(hash.clone(), chunks[0].data.clone());

        // The received chunk should be available for serving to other peers.
        assert!(mgr.get_chunk(&hash).is_some());
    }

    #[test]
    fn share_multiple_files() {
        let mut mgr = FileManager::new();

        let (m1, _) = mgr
            .share_file(b"file one", "one.txt".into(), "text/plain".into())
            .unwrap();
        let (m2, _) = mgr
            .share_file(b"file two", "two.txt".into(), "text/plain".into())
            .unwrap();

        assert_ne!(m1.file_hash, m2.file_hash);

        // Both files' chunks should be accessible.
        for hash in &m1.chunk_hashes {
            assert!(mgr.get_chunk(hash).is_some());
        }
        for hash in &m2.chunk_hashes {
            assert!(mgr.get_chunk(hash).is_some());
        }
    }

    #[test]
    fn shared_manifest_envelope_is_deserializable() {
        let mut mgr = FileManager::new();
        let (manifest, envelope) = mgr
            .share_file(b"envelope test", "env.txt".into(), "text/plain".into())
            .unwrap();

        let (decoded, msg_type) =
            willow_transport::unpack_envelope::<FileManifest>(&envelope).unwrap();
        assert_eq!(msg_type, MessageType::File);
        assert_eq!(decoded.filename, manifest.filename);
        assert_eq!(decoded.file_hash, manifest.file_hash);
    }
}
