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
