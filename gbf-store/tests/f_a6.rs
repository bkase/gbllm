use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Cursor, Read};
use std::thread;
use std::time::Duration;

use gbf_foundation::{BlobCodec, BlobRef, Hash256, SemVer};
use gbf_store::archive::{
    ARCHIVE_HEADER_LEN, ARCHIVE_MAGIC, ARCHIVE_VERSION, ArchiveError, MAX_ARCHIVE_BLOB_BODY_LEN,
    create_archive, extract_archive, list_archive,
};
use gbf_store::blob::{BlobStore, BlobStoreError};
use gbf_store::gc::{
    BlobReferenceError, BlobReferenceReader, BlobReferencesRegistry, GcOptions,
    UnknownReferencePolicy, run_gc,
};
use gbf_store::integrity::{IntegrityError, verify_all, verify_integrity, verify_reachable};
use gbf_store::pinset::{BlobReferences, Pinset, PinsetName};
use gbf_store::stage_cache::{
    ComponentDigestSet, ComponentId, FeatureFlag, StageCache, StageCacheKey, StageId, StageKey,
    compose_key, try_compose_key,
};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

fn store() -> (tempfile::TempDir, BlobStore) {
    let dir = tempdir().expect("tempdir");
    let store = BlobStore::open(dir.path().join("store")).expect("store opens");
    (dir, store)
}

fn pinset(name: &str, roots: impl IntoIterator<Item = Hash256>) -> Pinset {
    Pinset {
        name: PinsetName::new(name).expect("valid pinset name"),
        roots: roots.into_iter().collect(),
        annotation: None,
    }
}

fn key_with(component: &str, component_hash: Hash256) -> StageKey {
    let mut components = BTreeMap::new();
    components.insert(ComponentId::from(component), component_hash);
    let mut flags = BTreeSet::new();
    flags.insert(FeatureFlag::from("trace"));
    StageKey {
        stage_id: StageId::from("codegen.range"),
        shard_local: ComponentDigestSet { components },
        global: Hash256::from_bytes([9; 32]),
        feature_flags: flags,
        pass_version: SemVer::new(1, 2, 3),
    }
}

#[derive(Clone)]
struct MapRefs {
    refs: BTreeMap<Hash256, Vec<Hash256>>,
}

impl BlobReferenceReader for MapRefs {
    fn referenced_blobs(
        &self,
        hash: Hash256,
        _bytes: &[u8],
    ) -> Result<Option<Vec<Hash256>>, BlobReferenceError> {
        Ok(self.refs.get(&hash).cloned())
    }
}

fn registry_with(refs: BTreeMap<Hash256, Vec<Hash256>>) -> BlobReferencesRegistry {
    let mut registry = BlobReferencesRegistry::new();
    registry.register(MapRefs { refs });
    registry
}

fn digest(bytes: &[u8]) -> Hash256 {
    Hash256::from_bytes(Sha256::digest(bytes).into())
}

mod blob {
    use super::*;

    #[test]
    fn round_trip() {
        let (_dir, store) = store();
        let hash = store.put(b"hello").expect("put");

        assert_eq!(store.get(hash).expect("get"), b"hello");
    }

    #[test]
    fn content_addressed_idempotent() {
        let (_dir, store) = store();
        let first = store.put(b"same").expect("first put");
        let second = store.put(b"same").expect("second put");

        assert_eq!(first, second);
        assert_eq!(store.list_blobs().expect("list").len(), 1);
    }

    #[test]
    fn atomic_writes() {
        // Simulate a crash mid-write: bytes land in tmp/ but rename never
        // happens. The canonical path for those bytes must not appear, and
        // the store reopens cleanly with the orphan tmp untouched (open does
        // not sweep tmp; that's the explicit `cleanup_tmp` operation).
        let (dir, _store) = store();
        let store_root = dir.path().join("store");
        let claimed = digest(b"partial");
        let tmp = store_root.join("tmp").join("orphan.tmp");
        fs::write(&tmp, b"partial").expect("write tmp");

        let reopened = BlobStore::open(store_root.clone()).expect("reopen");

        assert!(!reopened.exists(claimed));
        assert!(!reopened.path_for(claimed).exists());
        assert!(tmp.exists());
    }

    #[test]
    fn streaming_round_trip() {
        let (_dir, store) = store();
        let bytes = vec![42_u8; 96 * 1024];
        let hash = store
            .put_streaming(Cursor::new(bytes.clone()))
            .expect("stream put");
        let mut out = Vec::new();
        store
            .get_streaming(hash)
            .expect("stream get")
            .read_to_end(&mut out)
            .expect("read stream");

        assert_eq!(out, bytes);
    }

    #[test]
    fn streaming_hash_matches_inline() {
        let (_dir, store) = store();
        let bytes = vec![7_u8; 70 * 1024];

        assert_eq!(
            store.put(&bytes).expect("put"),
            store
                .put_streaming(Cursor::new(bytes))
                .expect("streaming put")
        );
    }

    #[test]
    fn two_char_prefix_layout() {
        let (_dir, store) = store();
        let hash = store.put(b"path").expect("put");
        let path = store.path_for(hash);
        let hex = hash.to_hex();

        assert!(path.ends_with(format!("{}/{}", &hex[..2], hex)));
    }

    #[test]
    fn open_preserves_tmp_files() {
        let (dir, _store) = store();
        let tmp = dir.path().join("store").join("tmp").join("inflight.tmp");
        fs::write(&tmp, b"in flight").expect("write tmp");

        let _reopened = BlobStore::open(dir.path().join("store")).expect("reopen");

        assert!(tmp.exists());
    }

    #[test]
    fn cleanup_tmp_removes_old_orphans() {
        let (dir, store) = store();
        let tmp = dir.path().join("store").join("tmp").join("old.tmp");
        fs::write(&tmp, b"old").expect("write tmp");

        assert_eq!(
            store
                .cleanup_tmp(Duration::ZERO)
                .expect("cleanup tmp removes"),
            1
        );
        assert!(!tmp.exists());
    }

    #[test]
    fn open_creates_dirs() {
        let dir = tempdir().expect("tempdir");
        let _store = BlobStore::open(dir.path().join("store")).expect("open");

        assert!(
            dir.path()
                .join("store")
                .join("blobs")
                .join("sha256")
                .is_dir()
        );
        assert!(dir.path().join("store").join("tmp").is_dir());
    }

    #[test]
    fn put_as_returns_blob_ref() {
        let (_dir, store) = store();
        let blob_ref = store.put_as(b"encoded", BlobCodec::Zstd).expect("put_as");

        assert_eq!(blob_ref.len, 7);
        assert_eq!(blob_ref.codec, BlobCodec::Zstd);
        assert_eq!(store.get(blob_ref.hash).expect("get"), b"encoded");
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn put_as_rejects_oversize() {
        let err = gbf_store::blob::checked_blob_len(u32::MAX as usize + 1)
            .expect_err("oversize len rejected");

        assert!(matches!(err, BlobStoreError::BlobTooLarge { .. }));
    }

    #[test]
    fn put_expect_hash_match() {
        let (_dir, store) = store();
        let expected = store.put(b"expected").expect("put");
        store.remove(expected).expect("remove");

        assert_eq!(
            store.put_expect(expected, b"expected").expect("put_expect"),
            expected
        );
    }

    #[test]
    fn put_expect_hash_mismatch() {
        let (_dir, store) = store();
        let expected = store.put(b"expected").expect("put");
        store.remove(expected).expect("remove");

        let err = store
            .put_expect(expected, b"different")
            .expect_err("hash mismatch");

        assert!(matches!(err, BlobStoreError::HashMismatch { .. }));
        assert!(!store.exists(expected));
    }

    #[test]
    fn idempotent_verifies_existing() {
        let (_dir, store) = store();
        let hash = store.put(b"clean").expect("put");
        fs::write(store.path_for(hash), b"corrupt").expect("corrupt canonical");

        let err = store.put(b"clean").expect_err("corruption detected");

        assert!(matches!(err, BlobStoreError::ExistingBlobCorrupt { .. }));
    }

    #[test]
    fn get_ref_validates_len() {
        let (_dir, store) = store();
        let hash = store.put(b"len").expect("put");

        let err = store
            .get_ref(BlobRef {
                hash,
                len: 99,
                codec: BlobCodec::Raw,
            })
            .expect_err("len mismatch");

        assert!(matches!(err, BlobStoreError::LenMismatch { .. }));
    }

    #[test]
    fn list_blobs_walks_all() {
        let (_dir, store) = store();
        let a = store.put(b"a").expect("put a");
        let b = store.put(b"b").expect("put b");

        assert_eq!(store.list_blobs().expect("list"), vec![a.min(b), a.max(b)]);
    }

    #[test]
    fn remove_then_exists_false() {
        let (_dir, store) = store();
        let hash = store.put(b"gone").expect("put");

        store.remove(hash).expect("remove");

        assert!(!store.exists(hash));
    }

    #[test]
    fn concurrent_writes_idempotent() {
        let (_dir, store) = store();
        let mut handles = Vec::new();
        for _ in 0..4 {
            let store = store.clone();
            handles.push(thread::spawn(move || store.put(b"parallel").expect("put")));
        }
        let mut hashes = Vec::new();
        for handle in handles {
            hashes.push(handle.join().expect("thread"));
        }

        assert!(hashes.iter().all(|hash| *hash == hashes[0]));
        assert_eq!(store.list_blobs().expect("list").len(), 1);
    }
}

mod integrity {
    use super::*;

    #[test]
    fn detects_corruption() {
        let (_dir, store) = store();
        let hash = store.put(b"clean").expect("put");
        fs::write(store.path_for(hash), b"dirty").expect("corrupt");

        let err = verify_integrity(&store, hash).expect_err("corruption");

        assert!(matches!(err, IntegrityError::HashMismatch { .. }));
    }

    #[test]
    fn detects_missing() {
        let (_dir, store) = store();

        let err = verify_integrity(&store, Hash256::from_bytes([3; 32])).expect_err("missing hash");

        assert!(matches!(err, IntegrityError::NotFound { .. }));
    }

    #[test]
    fn verify_all_no_mismatches() {
        let (_dir, store) = store();
        store.put(b"a").expect("put");

        let report = verify_all(&store).expect("verify all");

        assert!(report.mismatches.is_empty());
        assert!(report.missing.is_empty());
    }

    #[test]
    fn verify_all_counts_blobs() {
        let (_dir, store) = store();
        store.put(b"a").expect("put a");
        store.put(b"b").expect("put b");

        assert_eq!(verify_all(&store).expect("verify all").blobs_checked, 2);
    }

    #[test]
    fn verify_reachable_finds_missing() {
        let (_dir, store) = store();
        let parent = store.put(b"parent").expect("put parent");
        let missing = Hash256::from_bytes([8; 32]);
        let registry = registry_with(BTreeMap::from([(parent, vec![missing])]));

        let report =
            verify_reachable(&store, [parent], &registry).expect("verify reachable reports");

        assert_eq!(report.missing, vec![missing]);
    }

    #[test]
    fn report_serde_round_trip() {
        let report = gbf_store::integrity::IntegrityReport {
            blobs_checked: 1,
            mismatches: vec![Hash256::from_bytes([1; 32])],
            missing: vec![Hash256::from_bytes([2; 32])],
        };

        let encoded = serde_json::to_string(&report).expect("report serializes");
        let decoded = serde_json::from_str(&encoded).expect("report deserializes");

        assert_eq!(report, decoded);
    }
}

mod stage_cache {
    use super::*;

    #[test]
    fn deterministic_keys() {
        let mut left_components = BTreeMap::new();
        left_components.insert(ComponentId::from("b"), Hash256::from_bytes([2; 32]));
        left_components.insert(ComponentId::from("a"), Hash256::from_bytes([1; 32]));
        let mut right_components = BTreeMap::new();
        right_components.insert(ComponentId::from("a"), Hash256::from_bytes([1; 32]));
        right_components.insert(ComponentId::from("b"), Hash256::from_bytes([2; 32]));

        let left = StageKey {
            stage_id: StageId::from("stage"),
            shard_local: ComponentDigestSet {
                components: left_components,
            },
            global: Hash256::from_bytes([3; 32]),
            feature_flags: BTreeSet::from([FeatureFlag::from("x"), FeatureFlag::from("y")]),
            pass_version: SemVer::new(1, 0, 0),
        };
        let right = StageKey {
            shard_local: ComponentDigestSet {
                components: right_components,
            },
            feature_flags: BTreeSet::from([FeatureFlag::from("y"), FeatureFlag::from("x")]),
            ..left.clone()
        };

        assert_eq!(compose_key(&left), compose_key(&right));
    }

    #[test]
    fn shard_invalidation() {
        let left = key_with("component", Hash256::from_bytes([1; 32]));
        let right = key_with("component", Hash256::from_bytes([2; 32]));

        assert_ne!(compose_key(&left), compose_key(&right));
    }

    #[test]
    fn feature_flag_sensitivity() {
        let left = key_with("component", Hash256::from_bytes([1; 32]));
        let mut right = left.clone();
        right.feature_flags.insert(FeatureFlag::from("new-flag"));

        assert_ne!(compose_key(&left), compose_key(&right));
    }

    #[test]
    fn pass_version_sensitivity() {
        let left = key_with("component", Hash256::from_bytes([1; 32]));
        let mut right = left.clone();
        right.pass_version = SemVer::new(1, 2, 4);

        assert_ne!(compose_key(&left), compose_key(&right));
    }

    #[test]
    fn round_trip() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let key = key_with("component", Hash256::from_bytes([1; 32]));

        cache.put(&key, b"payload").expect("cache put");

        assert_eq!(
            cache.get(&key).expect("cache get"),
            Some(b"payload".to_vec())
        );
    }

    #[test]
    fn put_returns_entry() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let key = key_with("component", Hash256::from_bytes([1; 32]));

        let entry = cache.put(&key, b"payload").expect("cache put");

        assert_eq!(entry.key, compose_key(&key));
        assert_eq!(store.get(entry.payload_hash).expect("payload"), b"payload");
    }

    #[test]
    fn miss_returns_none() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);

        assert_eq!(
            cache
                .get(&key_with("component", Hash256::from_bytes([1; 32])))
                .expect("cache get"),
            None
        );
    }

    #[test]
    fn stale_index_treated_as_miss() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let key = key_with("component", Hash256::from_bytes([1; 32]));
        let entry = cache.put(&key, b"payload").expect("cache put");
        store.remove(entry.payload_hash).expect("remove payload");

        assert_eq!(cache.get(&key).expect("stale miss"), None);
    }

    #[test]
    fn missing_index_after_put_treated_as_miss() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let key = key_with("component", Hash256::from_bytes([1; 32]));
        let entry = cache.put(&key, b"payload").expect("cache put");
        fs::remove_file(cache.index_path_for(entry.key)).expect("remove index");

        assert_eq!(cache.get(&key).expect("missing index miss"), None);
    }

    #[test]
    fn repeated_put_updates_index() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let key = key_with("component", Hash256::from_bytes([1; 32]));

        cache.put(&key, b"old").expect("first put");
        let entry = cache.put(&key, b"new").expect("second put");

        assert_eq!(store.get(entry.payload_hash).expect("payload"), b"new");
        assert_eq!(cache.get(&key).expect("cache get"), Some(b"new".to_vec()));
    }

    #[test]
    fn invalid_index_returns_typed_error() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let key = key_with("component", Hash256::from_bytes([1; 32]));
        let index_path = cache.index_path_for(compose_key(&key));
        fs::create_dir_all(index_path.parent().expect("index parent")).expect("index parent");
        fs::write(&index_path, b"not-a-hash").expect("write bad index");

        let err = cache.get(&key).expect_err("invalid index error");

        assert!(matches!(
            err,
            gbf_store::stage_cache::StageCacheError::InvalidIndex { .. }
        ));
    }

    #[test]
    fn compose_key_length_prefix_safe() {
        let left = StageKey {
            stage_id: StageId::from("ab"),
            shard_local: ComponentDigestSet {
                components: BTreeMap::from([(
                    ComponentId::from("cd"),
                    Hash256::from_bytes([1; 32]),
                )]),
            },
            global: Hash256::ZERO,
            feature_flags: BTreeSet::new(),
            pass_version: SemVer::new(0, 0, 0),
        };
        let right = StageKey {
            stage_id: StageId::from("a"),
            shard_local: ComponentDigestSet {
                components: BTreeMap::from([(
                    ComponentId::from("bcd"),
                    Hash256::from_bytes([1; 32]),
                )]),
            },
            ..left.clone()
        };

        assert_ne!(compose_key(&left), compose_key(&right));
    }

    #[test]
    fn component_digest_btreemap_canonical() {
        deterministic_keys();
    }

    #[test]
    fn stage_cache_key_distinct_from_hash() {
        let key: StageCacheKey = compose_key(&key_with("component", Hash256::from_bytes([1; 32])));

        assert_ne!(key.to_string(), Hash256::ZERO.to_string());
    }

    #[test]
    fn pass_version_component_overflow_rejected() {
        let mut key = key_with("component", Hash256::from_bytes([1; 32]));
        key.pass_version = SemVer::new(u32::MAX as u64 + 1, 0, 0);

        assert!(try_compose_key(&key).is_err());
    }
}

mod pinset {
    use super::*;

    #[test]
    fn serde_round_trip() {
        let hash = Hash256::from_bytes([1; 32]);
        let pinset = Pinset {
            name: PinsetName::new("latest").expect("name"),
            roots: BTreeSet::from([hash]),
            annotation: Some("build".to_owned()),
        };

        let encoded = serde_json::to_string(&pinset).expect("pinset serializes");
        let decoded = serde_json::from_str(&encoded).expect("pinset deserializes");

        assert_eq!(pinset, decoded);
    }

    #[test]
    fn name_validation_rejects_empty() {
        assert!(PinsetName::new("").is_err());
    }

    #[test]
    fn name_validation_rejects_path_separator() {
        assert!(PinsetName::new("bad/name").is_err());
        assert!(PinsetName::new("bad\\name").is_err());
    }

    #[test]
    fn name_validation_rejects_parent_segment() {
        assert!(PinsetName::new("..").is_err());
        assert!(PinsetName::new("bad..name").is_err());
    }

    #[test]
    fn name_validation_rejects_nul() {
        assert!(PinsetName::new("bad\0name").is_err());
    }

    #[test]
    fn name_validation_rejects_leading_dot() {
        assert!(PinsetName::new(".hidden").is_err());
    }

    #[test]
    fn deserialize_runs_validation() {
        let err = serde_json::from_str::<Pinset>(
            "{
            \"name\":\"../bad\",
            \"roots\":[],
            \"annotation\":null
        }",
        )
        .expect_err("invalid name rejected");

        assert!(err.to_string().contains("path separators"));
    }

    #[test]
    fn roots_dedup_via_btreeset() {
        let hash = Hash256::from_bytes([1; 32]);
        let pinset = pinset("latest", [hash, hash]);

        assert_eq!(pinset.roots.len(), 1);
    }

    #[test]
    fn annotation_optional_round_trip() {
        let pinset = pinset("latest", []);
        let encoded = serde_json::to_string(&pinset).expect("serialize");
        let decoded: Pinset = serde_json::from_str(&encoded).expect("deserialize");

        assert_eq!(decoded.annotation, None);
    }

    #[test]
    fn blob_references_trait_compiles() {
        struct Typed(Hash256);
        impl BlobReferences for Typed {
            fn referenced_blobs(&self) -> Vec<Hash256> {
                vec![self.0]
            }
        }

        let typed: Box<dyn BlobReferences> = Box::new(Typed(Hash256::from_bytes([1; 32])));

        assert_eq!(typed.referenced_blobs(), vec![Hash256::from_bytes([1; 32])]);
    }
}

mod gc {
    use super::*;

    fn leaf_options() -> GcOptions {
        GcOptions {
            unknown_reference_policy: UnknownReferencePolicy::TreatAsLeaf,
            ..GcOptions::default()
        }
    }

    #[test]
    fn pinset_protection() {
        let (_dir, store) = store();
        let pinned = store.put(b"pinned").expect("put pinned");

        let report = run_gc(
            &store,
            &[pinset("latest", [pinned])],
            &BlobReferencesRegistry::empty(),
            &leaf_options(),
        )
        .expect("gc");

        assert!(store.exists(pinned));
        assert_eq!(report.blobs_kept, 1);
    }

    #[test]
    fn unpinned_blob_removed() {
        let (_dir, store) = store();
        let unpinned = store.put(b"unpinned").expect("put unpinned");

        let report = run_gc(
            &store,
            &[],
            &BlobReferencesRegistry::empty(),
            &leaf_options(),
        )
        .expect("gc");

        assert!(!store.exists(unpinned));
        assert_eq!(report.removed, vec![unpinned]);
    }

    #[test]
    fn transitive_refs_via_registry() {
        let (_dir, store) = store();
        let parent = store.put(b"parent").expect("put parent");
        let child = store.put(b"child").expect("put child");
        let grandchild = store.put(b"grandchild").expect("put grandchild");
        let unrelated = store.put(b"unrelated").expect("put unrelated");
        let registry = registry_with(BTreeMap::from([
            (parent, vec![child]),
            (child, vec![grandchild]),
            (grandchild, vec![]),
        ]));

        run_gc(
            &store,
            &[pinset("latest", [parent])],
            &registry,
            &GcOptions::default(),
        )
        .expect("gc");

        assert!(store.exists(parent));
        assert!(store.exists(child));
        assert!(store.exists(grandchild));
        assert!(!store.exists(unrelated));
    }

    #[test]
    fn unknown_reference_policy_abort() {
        let (_dir, store) = store();
        let pinned = store.put(b"unknown").expect("put pinned");

        let err = run_gc(
            &store,
            &[pinset("latest", [pinned])],
            &BlobReferencesRegistry::empty(),
            &GcOptions::default(),
        )
        .expect_err("abort");

        assert!(matches!(
            err,
            gbf_store::gc::GcError::UndecodableReferenceBearingBlob { .. }
        ));
    }

    #[test]
    fn unknown_reference_policy_treat_as_leaf() {
        let (_dir, store) = store();
        let parent = store.put(b"parent").expect("put parent");
        let child = store.put(b"child").expect("put child");
        let registry = registry_with(BTreeMap::from([(parent, vec![child]), (child, vec![])]));

        run_gc(
            &store,
            &[pinset("latest", [parent])],
            &registry,
            &GcOptions::default(),
        )
        .expect("registered references keep child");
        assert!(store.exists(child));

        run_gc(
            &store,
            &[pinset("latest", [parent])],
            &BlobReferencesRegistry::empty(),
            &leaf_options(),
        )
        .expect("gc");

        assert!(store.exists(parent));
        assert!(!store.exists(child));
    }

    #[test]
    fn dry_run_populates_candidates() {
        let (_dir, store) = store();
        let hash = store.put(b"candidate").expect("put");
        let opts = GcOptions {
            dry_run: true,
            ..leaf_options()
        };

        let report = run_gc(&store, &[], &BlobReferencesRegistry::empty(), &opts).expect("gc");

        assert_eq!(report.candidate_blobs, 1);
        assert_eq!(report.blobs_removed, 0);
        assert!(report.removed.is_empty());
        assert!(store.exists(hash));
    }

    #[test]
    fn max_remove_per_run_honored() {
        let (_dir, store) = store();
        store.put(b"a").expect("put a");
        store.put(b"b").expect("put b");

        let opts = GcOptions {
            max_remove_per_run: Some(1),
            ..leaf_options()
        };
        let report = run_gc(&store, &[], &BlobReferencesRegistry::empty(), &opts).expect("gc");

        assert_eq!(report.blobs_removed, 1);
        assert_eq!(store.list_blobs().expect("list").len(), 1);
    }

    #[test]
    fn removal_order_is_hash_ascending() {
        let (_dir, store) = store();
        let a = store.put(b"a").expect("put a");
        let b = store.put(b"b").expect("put b");
        let c = store.put(b"c").expect("put c");
        let mut all = [a, b, c];
        all.sort();

        let opts = GcOptions {
            max_remove_per_run: Some(2),
            ..leaf_options()
        };
        let report = run_gc(&store, &[], &BlobReferencesRegistry::empty(), &opts).expect("gc");

        assert_eq!(report.removed, all[..2]);
    }

    #[test]
    fn report_counts_correct() {
        let (_dir, store) = store();
        let kept = store.put(b"kept").expect("put kept");
        store.put(b"removed").expect("put removed");

        let opts = GcOptions {
            dry_run: true,
            max_remove_per_run: Some(0),
            ..leaf_options()
        };
        let report = run_gc(
            &store,
            &[pinset("latest", [kept])],
            &BlobReferencesRegistry::empty(),
            &opts,
        )
        .expect("gc");

        assert_eq!(report.blobs_kept + report.candidate_blobs, 2);
        assert_eq!(report.blobs_removed, 0);
    }

    #[test]
    fn bytes_freed_matches_sum_of_lens() {
        let (_dir, store) = store();
        store.put(b"1234").expect("put");
        store.put(b"12").expect("put");

        let report = run_gc(
            &store,
            &[],
            &BlobReferencesRegistry::empty(),
            &leaf_options(),
        )
        .expect("gc");

        assert_eq!(report.bytes_freed, 6);
    }

    #[test]
    fn sweep_stage_cache_indexes_removes_stale() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let key = key_with("component", Hash256::from_bytes([1; 32]));
        let entry = cache.put(&key, b"payload").expect("put");
        let index_path = cache.index_path_for(entry.key);

        let opts = GcOptions {
            sweep_stage_cache_indexes: true,
            ..leaf_options()
        };
        run_gc(&store, &[], &BlobReferencesRegistry::empty(), &opts).expect("gc");

        assert!(!store.exists(entry.payload_hash));
        assert!(!index_path.exists());
    }

    #[test]
    fn sweep_stage_cache_indexes_dry_run_removes_nothing() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let key = key_with("component", Hash256::from_bytes([1; 32]));
        let entry = cache.put(&key, b"payload").expect("put");
        let index_path = cache.index_path_for(entry.key);
        let opts = GcOptions {
            dry_run: true,
            sweep_stage_cache_indexes: true,
            ..leaf_options()
        };

        run_gc(&store, &[], &BlobReferencesRegistry::empty(), &opts).expect("gc");

        assert!(store.exists(entry.payload_hash));
        assert!(index_path.exists());
    }

    #[test]
    fn sweep_stage_cache_indexes_respects_remove_limit() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let key_a = key_with("a", Hash256::from_bytes([1; 32]));
        let key_b = key_with("b", Hash256::from_bytes([2; 32]));
        let entry_a = cache.put(&key_a, b"a").expect("put a");
        let entry_b = cache.put(&key_b, b"b").expect("put b");
        let path_a = cache.index_path_for(entry_a.key);
        let path_b = cache.index_path_for(entry_b.key);
        let opts = GcOptions {
            max_remove_per_run: Some(1),
            sweep_stage_cache_indexes: true,
            ..leaf_options()
        };

        run_gc(&store, &[], &BlobReferencesRegistry::empty(), &opts).expect("gc");

        assert_eq!(
            path_a.exists(),
            store.exists(entry_a.payload_hash),
            "index for payload a should match whether payload a survived the limited sweep"
        );
        assert_eq!(
            path_b.exists(),
            store.exists(entry_b.payload_hash),
            "index for payload b should match whether payload b survived the limited sweep"
        );
        assert_eq!(store.list_blobs().expect("remaining").len(), 1);
    }

    #[test]
    fn report_serde_round_trip() {
        let report = gbf_store::gc::GcReport {
            pinsets_walked: 1,
            blobs_kept: 2,
            candidate_blobs: 3,
            candidate_bytes: 4,
            blobs_removed: 5,
            bytes_freed: 6,
            removed: vec![Hash256::from_bytes([1; 32])],
        };

        let encoded = serde_json::to_string(&report).expect("serialize");
        let decoded = serde_json::from_str(&encoded).expect("deserialize");

        assert_eq!(report, decoded);
    }

    #[test]
    fn two_pinsets_union() {
        let (_dir, store) = store();
        let a = store.put(b"a").expect("put a");
        let b = store.put(b"b").expect("put b");

        run_gc(
            &store,
            &[pinset("a", [a]), pinset("b", [b])],
            &BlobReferencesRegistry::empty(),
            &leaf_options(),
        )
        .expect("gc");

        assert!(store.exists(a));
        assert!(store.exists(b));
    }
}

mod archive {
    use super::*;

    fn archive_fixture() -> (tempfile::TempDir, BlobStore, Vec<Pinset>) {
        let (dir, store) = store();
        let a = store.put(b"alpha").expect("put alpha");
        let b = store.put(b"beta").expect("put beta");
        let pinsets = vec![pinset("zeta", [b]), pinset("alpha", [a])];
        (dir, store, pinsets)
    }

    fn make_archive(store: &BlobStore, pinsets: &[Pinset]) -> Vec<u8> {
        let mut bytes = Vec::new();
        create_archive(store, pinsets, &BlobReferencesRegistry::empty(), &mut bytes)
            .expect("create archive");
        bytes
    }

    fn header(pinset_count: u16, blob_count: u32, total_bytes: u64) -> Vec<u8> {
        let mut archive = Vec::new();
        archive.extend_from_slice(&ARCHIVE_MAGIC);
        archive.push(ARCHIVE_VERSION);
        archive.extend_from_slice(&pinset_count.to_le_bytes());
        archive.push(0);
        archive.extend_from_slice(&blob_count.to_le_bytes());
        archive.extend_from_slice(&total_bytes.to_le_bytes());
        archive
    }

    fn empty_header(blob_count: u32, total_bytes: u64) -> Vec<u8> {
        header(0, blob_count, total_bytes)
    }

    #[test]
    fn magic_bytes() {
        assert_eq!(ARCHIVE_MAGIC, *b"GBLM\0ARC");
    }

    #[test]
    fn version_byte() {
        assert_eq!(ARCHIVE_VERSION, 1);
    }

    #[test]
    fn header_size_24_bytes() {
        let (_dir, store) = store();
        let bytes = make_archive(&store, &[]);

        assert_eq!(ARCHIVE_HEADER_LEN, 24);
        assert_eq!(bytes.len(), 24);
    }

    #[test]
    fn round_trip() {
        let (_dir, store_a, pinsets) = archive_fixture();
        let archive = make_archive(&store_a, &pinsets);
        let (_dir_b, store_b) = store();
        let extracted =
            extract_archive(&mut Cursor::new(&archive), &store_b).expect("extract archive");

        assert_eq!(
            store_a.list_blobs().expect("list a"),
            store_b.list_blobs().expect("list b")
        );
        assert_eq!(extracted.pinsets[0].name.as_str(), "alpha");
        assert_eq!(extracted.pinsets[1].name.as_str(), "zeta");
    }

    #[test]
    fn extracted_archive_returns_pinsets() {
        let (_dir, store_a, pinsets) = archive_fixture();
        let archive = make_archive(&store_a, &pinsets);
        let (_dir_b, store_b) = store();
        let extracted =
            extract_archive(&mut Cursor::new(&archive), &store_b).expect("extract archive");

        assert_eq!(extracted.pinsets.len(), 2);
    }

    #[test]
    fn pinsets_sorted_by_name_internally() {
        let (_dir, store, pinsets) = archive_fixture();
        let archive = make_archive(&store, &pinsets);
        let listed = list_archive(&mut Cursor::new(archive)).expect("list");

        assert_eq!(listed.pinsets[0].name.as_str(), "alpha");
        assert_eq!(listed.pinsets[1].name.as_str(), "zeta");
    }

    #[test]
    fn list_without_extract() {
        let (_dir, store_a, pinsets) = archive_fixture();
        let archive = make_archive(&store_a, &pinsets);
        let (_dir_b, store_b) = store();
        let listed = list_archive(&mut Cursor::new(archive)).expect("list");

        assert_eq!(listed.blobs.len(), 2);
        assert!(store_b.list_blobs().expect("list empty").is_empty());
    }

    #[test]
    fn deterministic_bytes() {
        let (_dir, store, pinsets) = archive_fixture();
        let mut reversed = pinsets.clone();
        reversed.reverse();

        assert_eq!(
            make_archive(&store, &pinsets),
            make_archive(&store, &reversed)
        );
    }

    #[test]
    fn blobs_sorted_by_hash() {
        let (_dir, store, pinsets) = archive_fixture();
        let archive = make_archive(&store, &pinsets);
        let listed = list_archive(&mut Cursor::new(archive)).expect("list");
        let mut sorted = listed.blobs.clone();
        sorted.sort_by_key(|(hash, _)| *hash);

        assert_eq!(listed.blobs, sorted);
    }

    #[test]
    fn extract_validates_each_record_hash() {
        let (_dir, store_a, pinsets) = archive_fixture();
        let mut archive = make_archive(&store_a, &pinsets);
        let last = archive.last_mut().expect("archive body exists");
        *last ^= 0xff;
        let (_dir_b, store_b) = store();

        let err = extract_archive(&mut Cursor::new(archive), &store_b).expect_err("hash mismatch");

        assert!(matches!(err, ArchiveError::HashMismatch { .. }));
    }

    #[test]
    fn create_rejects_corrupt_existing_blob_before_writing() {
        let (_dir, store) = store();
        let hash = store.put(b"body").expect("put");
        fs::write(store.path_for(hash), b"wrong").expect("corrupt canonical blob");
        let mut out = Vec::new();

        let err = create_archive(
            &store,
            &[pinset("root", [hash])],
            &BlobReferencesRegistry::empty(),
            &mut out,
        )
        .expect_err("corrupt source rejected");

        assert!(matches!(err, ArchiveError::HashMismatch { .. }));
        assert!(out.is_empty());
    }

    #[test]
    fn duplicate_pinset_name_rejected() {
        let (_dir, store) = store();
        let hash = store.put(b"body").expect("put");
        let mut out = Vec::new();

        let err = create_archive(
            &store,
            &[pinset("dup", [hash]), pinset("dup", [hash])],
            &BlobReferencesRegistry::empty(),
            &mut out,
        )
        .expect_err("duplicate pinset names are ambiguous");

        assert!(matches!(err, ArchiveError::DuplicatePinsetName { .. }));
    }

    #[test]
    fn extract_not_transactional() {
        let first_hash = digest(b"first");
        let second_hash = digest(b"second");
        let mut archive = empty_header(2, 10);
        archive.extend_from_slice(first_hash.as_bytes());
        archive.extend_from_slice(&5_u64.to_le_bytes());
        archive.extend_from_slice(b"first");
        archive.extend_from_slice(second_hash.as_bytes());
        archive.extend_from_slice(&5_u64.to_le_bytes());
        archive.extend_from_slice(b"wrong");
        let (_dir, store) = store();

        let err =
            extract_archive(&mut Cursor::new(archive), &store).expect_err("second record fails");

        assert!(matches!(err, ArchiveError::HashMismatch { .. }));
        assert!(store.exists(first_hash));
    }

    #[test]
    fn extract_rejects_record_exceeding_declared_total_before_commit() {
        let hash = digest(b"body");
        let mut archive = empty_header(1, 0);
        archive.extend_from_slice(hash.as_bytes());
        archive.extend_from_slice(&4_u64.to_le_bytes());
        archive.extend_from_slice(b"body");
        let (_dir, store) = store();

        let err = extract_archive(&mut Cursor::new(archive), &store)
            .expect_err("record length exceeds declared total");

        assert!(matches!(
            err,
            ArchiveError::RecordExceedsDeclaredTotal {
                len: 4,
                remaining: 0
            }
        ));
        assert!(store.list_blobs().expect("store remains empty").is_empty());
    }

    #[test]
    fn list_rejects_record_exceeding_declared_total() {
        let hash = digest(b"body");
        let mut archive = empty_header(1, 0);
        archive.extend_from_slice(hash.as_bytes());
        archive.extend_from_slice(&4_u64.to_le_bytes());
        archive.extend_from_slice(b"body");

        let err =
            list_archive(&mut Cursor::new(archive)).expect_err("record exceeds declared total");

        assert!(matches!(
            err,
            ArchiveError::RecordExceedsDeclaredTotal { .. }
        ));
    }

    #[test]
    fn blob_length_too_large_rejected_without_allocation() {
        let mut archive = empty_header(1, u64::MAX);
        archive.extend_from_slice(Hash256::ZERO.as_bytes());
        archive.extend_from_slice(&(MAX_ARCHIVE_BLOB_BODY_LEN + 1).to_le_bytes());
        let (_dir, store) = store();

        let err = extract_archive(&mut Cursor::new(archive), &store)
            .expect_err("oversize blob length rejected");

        assert!(matches!(err, ArchiveError::BlobTooLarge { .. }));
    }

    #[test]
    fn trailing_bytes_rejected() {
        let mut archive = empty_header(0, 0);
        archive.push(1);
        let (_dir, store) = store();

        let list_err =
            list_archive(&mut Cursor::new(archive.clone())).expect_err("trailing list bytes");
        let extract_err =
            extract_archive(&mut Cursor::new(archive), &store).expect_err("trailing extract bytes");

        assert!(matches!(list_err, ArchiveError::TrailingBytes));
        assert!(matches!(extract_err, ArchiveError::TrailingBytes));
    }

    #[test]
    fn total_bytes_mismatch_rejected() {
        let archive = empty_header(0, 1);
        let (_dir, store) = store();

        let list_err =
            list_archive(&mut Cursor::new(archive.clone())).expect_err("list total mismatch");
        let extract_err =
            extract_archive(&mut Cursor::new(archive), &store).expect_err("extract mismatch");

        assert!(matches!(list_err, ArchiveError::TotalBytesMismatch { .. }));
        assert!(matches!(
            extract_err,
            ArchiveError::TotalBytesMismatch { .. }
        ));
    }

    #[test]
    fn bad_magic_rejected() {
        let mut archive = vec![0_u8; 24];
        archive[8] = ARCHIVE_VERSION;

        let err = list_archive(&mut Cursor::new(archive)).expect_err("bad magic");

        assert!(matches!(err, ArchiveError::BadMagic { .. }));
    }

    #[test]
    fn unsupported_version_rejected() {
        let mut archive = Vec::new();
        archive.extend_from_slice(&ARCHIVE_MAGIC);
        archive.push(99);
        archive.extend_from_slice(&0_u16.to_le_bytes());
        archive.push(0);
        archive.extend_from_slice(&0_u32.to_le_bytes());
        archive.extend_from_slice(&0_u64.to_le_bytes());

        let err = list_archive(&mut Cursor::new(archive)).expect_err("bad version");

        assert!(matches!(
            err,
            ArchiveError::UnsupportedVersion { found: 99 }
        ));
    }

    #[test]
    fn reserved_byte_nonzero_rejected() {
        let mut archive = empty_header(0, 0);
        archive[11] = 1;

        let err = list_archive(&mut Cursor::new(archive)).expect_err("reserved byte is nonzero");

        assert!(matches!(err, ArchiveError::ReservedByteNonZero { .. }));
    }

    #[test]
    fn invalid_annotation_tag_rejected() {
        let mut archive = header(1, 0, 0);
        archive.extend_from_slice(&4_u16.to_le_bytes());
        archive.extend_from_slice(b"name");
        archive.push(2);

        let err = list_archive(&mut Cursor::new(archive)).expect_err("bad annotation tag");

        assert!(matches!(err, ArchiveError::InvalidAnnotationTag { .. }));
    }

    #[test]
    fn truncated_input_detected() {
        let err = list_archive(&mut Cursor::new(&ARCHIVE_MAGIC[..4])).expect_err("truncated");

        assert!(matches!(err, ArchiveError::Truncated));
    }

    #[test]
    fn pinset_with_annotation_round_trip() {
        let (_dir, store) = store();
        let hash = store.put(b"body").expect("put");
        let mut p = pinset("annotated", [hash]);
        p.annotation = Some("note".to_owned());
        let archive = make_archive(&store, &[p]);
        let listed = list_archive(&mut Cursor::new(archive)).expect("list");

        assert_eq!(listed.pinsets[0].annotation.as_deref(), Some("note"));
    }

    #[test]
    fn pinset_without_annotation_round_trip() {
        let (_dir, store) = store();
        let hash = store.put(b"body").expect("put");
        let archive = make_archive(&store, &[pinset("plain", [hash])]);
        let listed = list_archive(&mut Cursor::new(archive)).expect("list");

        assert_eq!(listed.pinsets[0].annotation, None);
    }

    #[test]
    fn overflow_too_many_pinsets() {
        let (_dir, store) = store();
        let pinsets: Vec<_> = (0..=u16::MAX as usize)
            .map(|i| pinset(&format!("p{i}"), []))
            .collect();
        let mut out = Vec::new();

        let err = create_archive(&store, &pinsets, &BlobReferencesRegistry::empty(), &mut out)
            .expect_err("too many pinsets");

        assert!(matches!(err, ArchiveError::TooManyPinsets { .. }));
    }

    #[test]
    fn overflow_pinset_name_too_long() {
        let (_dir, store) = store();
        let name = "a".repeat(u16::MAX as usize + 1);
        let pinset = Pinset {
            name: PinsetName::new(name).expect("long but syntactically valid name"),
            roots: BTreeSet::new(),
            annotation: None,
        };
        let mut out = Vec::new();

        let err = create_archive(
            &store,
            &[pinset],
            &BlobReferencesRegistry::empty(),
            &mut out,
        )
        .expect_err("name too long");

        assert!(matches!(err, ArchiveError::PinsetNameTooLong { .. }));
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn overflow_too_many_roots() {
        let err = gbf_store::archive::checked_root_count(u32::MAX as usize + 1)
            .expect_err("too many roots");

        assert!(matches!(err, ArchiveError::TooManyRoots { .. }));
    }
}
