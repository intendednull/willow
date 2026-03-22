//! # File Transfer Protocol
//!
//! libp2p request-response protocol for transferring file chunks between peers.
//!
//! Peers request chunks by hash and receive the chunk data in response.

use serde::{Deserialize, Serialize};
use willow_files::ContentHash;

/// Request a specific chunk by its content hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRequest {
    pub hash: ContentHash,
}

/// Response containing the requested chunk data, or an indication it's not available.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChunkResponse {
    /// The chunk was found and its data is included.
    Found { hash: ContentHash, data: Vec<u8> },
    /// The chunk is not available on this peer.
    NotFound { hash: ContentHash },
}
