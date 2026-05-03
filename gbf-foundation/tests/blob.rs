use std::mem;

use gbf_foundation::{BlobCodec, BlobRef, Hash256};

mod blob {
    use super::*;

    #[test]
    fn blob_ref_serde_round_trip() {
        let blob_ref = BlobRef {
            hash: Hash256::from_bytes([0xab; 32]),
            len: 1234,
            codec: BlobCodec::Zstd,
        };

        let encoded = serde_json::to_string(&blob_ref).expect("blob ref serializes");
        assert_eq!(
            encoded,
            "{\"hash\":\"abababababababababababababababababababababababababababababababab\",\"len\":1234,\"codec\":\"zstd\"}"
        );
        let decoded: BlobRef = serde_json::from_str(&encoded).expect("blob ref deserializes");

        assert_eq!(decoded, blob_ref);
    }

    #[test]
    fn blob_codec_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&BlobCodec::Raw).expect("raw serializes"),
            "\"raw\""
        );
        assert_eq!(
            serde_json::to_string(&BlobCodec::Zstd).expect("zstd serializes"),
            "\"zstd\""
        );
    }

    #[test]
    fn blob_ref_size_is_compact() {
        assert!(mem::size_of::<BlobRef>() <= 40);
    }
}
