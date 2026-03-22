//! # Willow Files
//!
//! Content-addressed file chunking, hashing, and reassembly for the Willow
//! P2P network.
//!
//! Files are split into fixed-size [`Chunk`]s, each identified by its SHA-256
//! hash. A [`FileManifest`] describes the complete file: its name, size, MIME
//! type, and the ordered list of chunk hashes needed to reassemble it.
//!
//! ## Workflow
//!
//! **Sender:**
//! 1. Call [`split_file`] to chunk the file and get a manifest + chunks.
//! 2. Broadcast the [`FileManifest`] over gossipsub.
//! 3. Serve chunk requests via libp2p request-response.
//!
//! **Receiver:**
//! 1. Receive the manifest via gossipsub.
//! 2. Request missing chunks from peers.
//! 3. Call [`assemble_file`] to reconstruct the original file.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Default chunk size: 256 KiB.
pub const DEFAULT_CHUNK_SIZE: usize = 256 * 1024;

// ───── Types ────────────────────────────────────────────────────────────────

/// SHA-256 hash identifying a chunk or file.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentHash(pub [u8; 32]);

impl ContentHash {
    /// Compute the SHA-256 hash of arbitrary data.
    pub fn of(data: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(data);
        Self(hasher.finalize().into())
    }

    /// Hex-encoded hash string.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl std::fmt::Display for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Show first 8 bytes (16 hex chars) for readability.
        for b in &self.0[..8] {
            write!(f, "{b:02x}")?;
        }
        write!(f, "...")
    }
}

/// A single chunk of a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// SHA-256 hash of this chunk's data.
    pub hash: ContentHash,
    /// The chunk's raw bytes.
    pub data: Vec<u8>,
}

/// Metadata describing a complete file and the chunks needed to reconstruct it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileManifest {
    /// Original filename.
    pub filename: String,
    /// MIME type (e.g. `image/png`).
    pub mime_type: String,
    /// Total file size in bytes.
    pub total_size: u64,
    /// SHA-256 hash of the complete file.
    pub file_hash: ContentHash,
    /// Ordered list of chunk hashes. Concatenating the chunks in this order
    /// reconstructs the original file.
    pub chunk_hashes: Vec<ContentHash>,
    /// Chunk size used when splitting (needed for reassembly validation).
    pub chunk_size: usize,
}

// ───── Errors ───────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("missing chunk: {0}")]
    MissingChunk(ContentHash),

    #[error("chunk hash mismatch: expected {expected}, got {actual}")]
    ChunkHashMismatch {
        expected: ContentHash,
        actual: ContentHash,
    },

    #[error("reassembled file hash mismatch: expected {expected}, got {actual}")]
    FileHashMismatch {
        expected: ContentHash,
        actual: ContentHash,
    },
}

// ───── Splitting ────────────────────────────────────────────────────────────

/// Split a file into chunks and produce a manifest.
///
/// Each chunk is `chunk_size` bytes (the last chunk may be smaller).
/// Returns the manifest and the list of chunks.
pub fn split_file(
    data: &[u8],
    filename: impl Into<String>,
    mime_type: impl Into<String>,
    chunk_size: usize,
) -> (FileManifest, Vec<Chunk>) {
    let file_hash = ContentHash::of(data);
    let mut chunks = Vec::new();
    let mut chunk_hashes = Vec::new();

    for piece in data.chunks(chunk_size) {
        let hash = ContentHash::of(piece);
        chunk_hashes.push(hash.clone());
        chunks.push(Chunk {
            hash,
            data: piece.to_vec(),
        });
    }

    let manifest = FileManifest {
        filename: filename.into(),
        mime_type: mime_type.into(),
        total_size: data.len() as u64,
        file_hash,
        chunk_hashes,
        chunk_size,
    };

    (manifest, chunks)
}

/// Split a file using the default chunk size.
pub fn split_file_default(
    data: &[u8],
    filename: impl Into<String>,
    mime_type: impl Into<String>,
) -> (FileManifest, Vec<Chunk>) {
    split_file(data, filename, mime_type, DEFAULT_CHUNK_SIZE)
}

// ───── Reassembly ───────────────────────────────────────────────────────────

/// Reassemble a file from a manifest and a set of chunks.
///
/// Chunks are looked up by hash from the provided slice. Returns the
/// reconstructed file bytes.
///
/// # Errors
///
/// Returns an error if any chunk is missing, has a hash mismatch, or if
/// the final file hash doesn't match the manifest.
pub fn assemble_file(manifest: &FileManifest, chunks: &[Chunk]) -> Result<Vec<u8>, FileError> {
    let chunk_map: std::collections::HashMap<&ContentHash, &Chunk> =
        chunks.iter().map(|c| (&c.hash, c)).collect();

    let mut data = Vec::with_capacity(manifest.total_size as usize);

    for expected_hash in &manifest.chunk_hashes {
        let chunk = chunk_map
            .get(expected_hash)
            .ok_or_else(|| FileError::MissingChunk(expected_hash.clone()))?;

        // Verify chunk integrity.
        let actual_hash = ContentHash::of(&chunk.data);
        if actual_hash != *expected_hash {
            return Err(FileError::ChunkHashMismatch {
                expected: expected_hash.clone(),
                actual: actual_hash,
            });
        }

        data.extend_from_slice(&chunk.data);
    }

    // Verify complete file hash.
    let actual_file_hash = ContentHash::of(&data);
    if actual_file_hash != manifest.file_hash {
        return Err(FileError::FileHashMismatch {
            expected: manifest.file_hash.clone(),
            actual: actual_file_hash,
        });
    }

    Ok(data)
}

// ───── Chunk Store ──────────────────────────────────────────────────────────

/// In-memory store for received chunks, used during file reassembly.
///
/// Tracks which chunks have been received for each manifest and detects
/// when a file is complete.
#[derive(Debug, Default)]
pub struct ChunkStore {
    /// Pending file transfers: file_hash → (manifest, received chunks).
    pending: std::collections::HashMap<ContentHash, (FileManifest, Vec<Chunk>)>,
}

impl ChunkStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a file manifest. Call this when a file announcement is received.
    pub fn register_manifest(&mut self, manifest: FileManifest) {
        let hash = manifest.file_hash.clone();
        self.pending.entry(hash).or_insert((manifest, Vec::new()));
    }

    /// Add a received chunk. Returns `true` if this chunk was needed.
    pub fn add_chunk(&mut self, chunk: Chunk) -> bool {
        for (manifest, chunks) in self.pending.values_mut() {
            if manifest.chunk_hashes.contains(&chunk.hash)
                && !chunks.iter().any(|c| c.hash == chunk.hash)
            {
                chunks.push(chunk);
                return true;
            }
        }
        false
    }

    /// Check if all chunks for a file have been received.
    pub fn is_complete(&self, file_hash: &ContentHash) -> bool {
        self.pending
            .get(file_hash)
            .map(|(manifest, chunks)| chunks.len() == manifest.chunk_hashes.len())
            .unwrap_or(false)
    }

    /// How many chunks are still missing for a file.
    pub fn missing_count(&self, file_hash: &ContentHash) -> usize {
        self.pending
            .get(file_hash)
            .map(|(manifest, chunks)| manifest.chunk_hashes.len() - chunks.len())
            .unwrap_or(0)
    }

    /// Get the hashes of chunks we still need for a file.
    pub fn missing_hashes(&self, file_hash: &ContentHash) -> Vec<ContentHash> {
        let Some((manifest, chunks)) = self.pending.get(file_hash) else {
            return Vec::new();
        };
        let have: std::collections::HashSet<_> = chunks.iter().map(|c| &c.hash).collect();
        manifest
            .chunk_hashes
            .iter()
            .filter(|h| !have.contains(h))
            .cloned()
            .collect()
    }

    /// Attempt to assemble a complete file. Removes it from pending on success.
    pub fn try_assemble(&mut self, file_hash: &ContentHash) -> Result<Vec<u8>, FileError> {
        let Some((manifest, chunks)) = self.pending.get(file_hash) else {
            return Err(FileError::MissingChunk(file_hash.clone()));
        };

        let data = assemble_file(manifest, chunks)?;
        self.pending.remove(file_hash);
        Ok(data)
    }

    /// List all pending file transfers.
    pub fn pending_files(&self) -> Vec<&FileManifest> {
        self.pending.values().map(|(m, _)| m).collect()
    }
}

// ───── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_deterministic() {
        let h1 = ContentHash::of(b"hello");
        let h2 = ContentHash::of(b"hello");
        assert_eq!(h1, h2);
    }

    #[test]
    fn content_hash_different_for_different_data() {
        let h1 = ContentHash::of(b"hello");
        let h2 = ContentHash::of(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn content_hash_to_hex() {
        let h = ContentHash::of(b"test");
        let hex = h.to_hex();
        assert_eq!(hex.len(), 64); // SHA-256 = 32 bytes = 64 hex chars
    }

    #[test]
    fn split_and_assemble_small_file() {
        let data = b"hello, willow!";
        let (manifest, chunks) = split_file(data, "hello.txt", "text/plain", 8);

        assert_eq!(manifest.filename, "hello.txt");
        assert_eq!(manifest.mime_type, "text/plain");
        assert_eq!(manifest.total_size, 14);
        assert_eq!(manifest.chunk_hashes.len(), 2); // 8 + 6
        assert_eq!(chunks.len(), 2);

        let reassembled = assemble_file(&manifest, &chunks).unwrap();
        assert_eq!(reassembled, data);
    }

    #[test]
    fn split_and_assemble_exact_chunk_boundary() {
        let data = vec![0xABu8; 16];
        let (manifest, chunks) = split_file(&data, "exact.bin", "application/octet-stream", 8);

        assert_eq!(chunks.len(), 2);
        let reassembled = assemble_file(&manifest, &chunks).unwrap();
        assert_eq!(reassembled, data);
    }

    #[test]
    fn split_and_assemble_single_chunk() {
        let data = b"tiny";
        let (manifest, chunks) = split_file_default(data, "tiny.txt", "text/plain");

        assert_eq!(chunks.len(), 1);
        let reassembled = assemble_file(&manifest, &chunks).unwrap();
        assert_eq!(reassembled, data);
    }

    #[test]
    fn split_and_assemble_empty_file() {
        let data = b"";
        let (manifest, chunks) = split_file(data, "empty.txt", "text/plain", 8);

        assert_eq!(chunks.len(), 0);
        assert_eq!(manifest.total_size, 0);
        let reassembled = assemble_file(&manifest, &chunks).unwrap();
        assert!(reassembled.is_empty());
    }

    #[test]
    fn assemble_missing_chunk_fails() {
        let data = b"hello, willow!";
        let (manifest, mut chunks) = split_file(data, "test.txt", "text/plain", 8);
        chunks.pop(); // Remove last chunk.

        let result = assemble_file(&manifest, &chunks);
        assert!(matches!(result, Err(FileError::MissingChunk(_))));
    }

    #[test]
    fn assemble_tampered_chunk_fails() {
        let data = b"hello, willow!";
        let (manifest, mut chunks) = split_file(data, "test.txt", "text/plain", 8);

        // Tamper with a chunk's data.
        chunks[0].data[0] ^= 0xFF;

        let result = assemble_file(&manifest, &chunks);
        assert!(matches!(result, Err(FileError::ChunkHashMismatch { .. })));
    }

    #[test]
    fn large_file_round_trip() {
        // 1 MiB of random-ish data.
        let data: Vec<u8> = (0..1_048_576u32).map(|i| (i % 251) as u8).collect();
        let (manifest, chunks) = split_file_default(&data, "large.bin", "application/octet-stream");

        assert_eq!(chunks.len(), 4); // 1 MiB / 256 KiB = 4 chunks
        let reassembled = assemble_file(&manifest, &chunks).unwrap();
        assert_eq!(reassembled, data);
    }

    #[test]
    fn manifest_serde_round_trip() {
        let data = b"serde test";
        let (manifest, _) = split_file_default(data, "test.txt", "text/plain");

        let bytes = willow_transport::pack(&manifest).unwrap();
        let decoded: FileManifest = willow_transport::unpack(&bytes).unwrap();

        assert_eq!(decoded.filename, manifest.filename);
        assert_eq!(decoded.file_hash, manifest.file_hash);
        assert_eq!(decoded.chunk_hashes.len(), manifest.chunk_hashes.len());
    }

    #[test]
    fn chunk_serde_round_trip() {
        let chunk = Chunk {
            hash: ContentHash::of(b"data"),
            data: b"data".to_vec(),
        };

        let bytes = willow_transport::pack(&chunk).unwrap();
        let decoded: Chunk = willow_transport::unpack(&bytes).unwrap();

        assert_eq!(decoded.hash, chunk.hash);
        assert_eq!(decoded.data, chunk.data);
    }

    // ── ChunkStore Tests ─────────────────────────────────────────────────

    #[test]
    fn chunk_store_register_and_complete() {
        let data = b"hello, chunks!";
        let (manifest, chunks) = split_file(data, "test.txt", "text/plain", 8);
        let file_hash = manifest.file_hash.clone();

        let mut store = ChunkStore::new();
        store.register_manifest(manifest);

        assert!(!store.is_complete(&file_hash));
        assert_eq!(store.missing_count(&file_hash), 2);

        for chunk in chunks {
            store.add_chunk(chunk);
        }

        assert!(store.is_complete(&file_hash));
        assert_eq!(store.missing_count(&file_hash), 0);

        let reassembled = store.try_assemble(&file_hash).unwrap();
        assert_eq!(reassembled, data);
    }

    #[test]
    fn chunk_store_partial_then_complete() {
        let data = b"partial test data!!";
        let (manifest, chunks) = split_file(data, "partial.txt", "text/plain", 8);
        let file_hash = manifest.file_hash.clone();

        let mut store = ChunkStore::new();
        store.register_manifest(manifest);

        // Add only first chunk.
        store.add_chunk(chunks[0].clone());
        assert!(!store.is_complete(&file_hash));
        assert_eq!(store.missing_count(&file_hash), 2);

        // missing_hashes returns the ones we need.
        let missing = store.missing_hashes(&file_hash);
        assert_eq!(missing.len(), 2);

        // Add remaining.
        for chunk in &chunks[1..] {
            store.add_chunk(chunk.clone());
        }
        assert!(store.is_complete(&file_hash));
    }

    #[test]
    fn chunk_store_duplicate_chunk_ignored() {
        let data = b"dedup test";
        let (manifest, chunks) = split_file(data, "dedup.txt", "text/plain", 64);
        let file_hash = manifest.file_hash.clone();

        let mut store = ChunkStore::new();
        store.register_manifest(manifest);

        assert!(store.add_chunk(chunks[0].clone()));
        assert!(!store.add_chunk(chunks[0].clone())); // duplicate
        assert!(store.is_complete(&file_hash));
    }

    #[test]
    fn chunk_store_unknown_chunk_rejected() {
        let mut store = ChunkStore::new();
        let chunk = Chunk {
            hash: ContentHash::of(b"unknown"),
            data: b"unknown".to_vec(),
        };
        assert!(!store.add_chunk(chunk));
    }

    #[test]
    fn chunk_store_pending_files() {
        let (m1, _) = split_file_default(b"file1", "a.txt", "text/plain");
        let (m2, _) = split_file_default(b"file2", "b.txt", "text/plain");

        let mut store = ChunkStore::new();
        store.register_manifest(m1);
        store.register_manifest(m2);

        assert_eq!(store.pending_files().len(), 2);
    }

    #[test]
    fn chunk_hashes_are_content_addressed() {
        let data = b"same content";
        let (_, chunks1) = split_file(data, "a.txt", "text/plain", 64);
        let (_, chunks2) = split_file(data, "b.txt", "text/plain", 64);

        // Same content → same chunk hashes regardless of filename.
        assert_eq!(chunks1[0].hash, chunks2[0].hash);
    }
}
