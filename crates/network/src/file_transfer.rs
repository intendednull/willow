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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_request_serde_round_trip() {
        let req = ChunkRequest {
            hash: ContentHash::of(b"test data"),
        };
        let bytes = willow_transport::pack(&req).unwrap();
        let decoded: ChunkRequest = willow_transport::unpack(&bytes).unwrap();
        assert_eq!(decoded.hash, req.hash);
    }

    #[test]
    fn chunk_response_found_serde_round_trip() {
        let resp = ChunkResponse::Found {
            hash: ContentHash::of(b"chunk data"),
            data: b"chunk data".to_vec(),
        };
        let bytes = willow_transport::pack(&resp).unwrap();
        let decoded: ChunkResponse = willow_transport::unpack(&bytes).unwrap();
        assert!(matches!(decoded, ChunkResponse::Found { ref data, .. } if data == b"chunk data"));
    }

    #[test]
    fn chunk_response_not_found_serde_round_trip() {
        let hash = ContentHash::of(b"missing");
        let resp = ChunkResponse::NotFound { hash: hash.clone() };
        let bytes = willow_transport::pack(&resp).unwrap();
        let decoded: ChunkResponse = willow_transport::unpack(&bytes).unwrap();
        assert!(matches!(decoded, ChunkResponse::NotFound { hash: ref h } if *h == hash));
    }
}
