//
// Copyright 2024 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::commitments::verify as verify_commitment;
use crate::guide::{InvalidState, ProofGuide};
use crate::implicit::{full_monitoring_path, monitoring_path};
use crate::log::{evaluate_batch_proof, verify_consistency_proof};
use crate::prefix::{evaluate as evaluate_prefix, MalformedProof};
use crate::proto::{
    AuditorTreeHead as SingleSignatureTreeHead, CondensedTreeSearchResponse, FullTreeHead,
    MonitorKey, MonitorProof, MonitorRequest, MonitorResponse, ProofStep, TreeHead,
};
use crate::{
    guide, log, vrf, DeploymentMode, FullSearchResponse, LastTreeHead, MonitorContext,
    MonitorStateUpdate, MonitoringData, PublicConfig, SearchContext, SearchStateUpdate,
    SlimSearchRequest, TreeRoot,
};

/// The range of allowed timestamp values relative to "now".
/// The timestamps will have to be in [now - max_behind .. now + max_ahead]
const ALLOWED_TIMESTAMP_RANGE: &TimestampRange = &TimestampRange {
    max_behind: Duration::from_secs(24 * 60 * 60),
    max_ahead: Duration::from_secs(10),
};

/// The range of allowed timestamp values relative to "now" used for auditor.
const ALLOWED_AUDITOR_TIMESTAMP_RANGE: &TimestampRange = &TimestampRange {
    max_behind: Duration::from_secs(7 * 24 * 60 * 60),
    max_ahead: Duration::from_secs(10),
};
const ENTRIES_MAX_BEHIND: u64 = 10_000_000;

#[derive(Debug, displaydoc::Display)]
pub enum Error {
    /// Required field '{0}' not found
    RequiredFieldMissing(&'static str),
    /// Bad data: {0}
    BadData(&'static str),
    /// Verification failed: {0}
    VerificationFailed(String),
}

impl std::error::Error for Error {}

impl From<log::Error> for Error {
    fn from(err: log::Error) -> Self {
        Self::VerificationFailed(err.to_string())
    }
}

impl From<vrf::Error> for Error {
    fn from(err: vrf::Error) -> Self {
        Self::VerificationFailed(err.to_string())
    }
}

impl From<guide::InvalidState> for Error {
    fn from(err: InvalidState) -> Self {
        Self::VerificationFailed(err.to_string())
    }
}

impl From<MalformedProof> for Error {
    fn from(err: MalformedProof) -> Self {
        Self::VerificationFailed(err.to_string())
    }
}

type Result<T> = std::result::Result<T, Error>;

fn get_proto_field<'a, T>(field: &'a Option<T>, name: &'static str) -> Result<&'a T> {
    field.as_ref().ok_or(Error::RequiredFieldMissing(name))
}

fn get_hash_proof(proof: &[Vec<u8>]) -> Result<Vec<[u8; 32]>> {
    proof
        .iter()
        .map(|elem| <&[u8] as TryInto<[u8; 32]>>::try_into(elem))
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| Error::BadData("proof element is wrong size"))
}

fn serialize_key(buffer: &mut Vec<u8>, key_material: &[u8], key_kind: &str) {
    let key_len = u16::try_from(key_material.len())
        .unwrap_or_else(|_| panic!("{} {}", key_kind, "key is too long to be encoded"));
    buffer.extend_from_slice(&key_len.to_be_bytes());
    buffer.extend_from_slice(key_material);
}

fn marshal_tree_head_tbs(
    tree_size: u64,
    timestamp: i64,
    root: &[u8; 32],
    config: &PublicConfig,
) -> Result<Vec<u8>> {
    let mut buf = vec![];

    buf.extend_from_slice(&[0, 0]); // Ciphersuite
    buf.push(config.mode.byte()); // Deployment mode

    serialize_key(&mut buf, config.signature_key.as_bytes(), "signature");
    serialize_key(&mut buf, config.vrf_key.as_bytes(), "VRF");

    if let Some(key) = config.mode.get_associated_key() {
        serialize_key(&mut buf, key.as_bytes(), "third party signature")
    }

    buf.extend_from_slice(&tree_size.to_be_bytes()); // Tree size
    buf.extend_from_slice(&timestamp.to_be_bytes()); // Timestamp
    buf.extend_from_slice(root); // Root hash

    Ok(buf)
}

fn marshal_update_value(value: &[u8]) -> Result<Vec<u8>> {
    let mut buf = vec![];

    let length =
        u32::try_from(value.len()).map_err(|_| Error::BadData("value too long to be encoded"))?;
    buf.extend_from_slice(&length.to_be_bytes());
    buf.extend_from_slice(value);

    Ok(buf)
}

/// Returns the hash of the leaf of the transparency tree.
fn leaf_hash(prefix_root: &[u8; 32], commitment: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(prefix_root);
    hasher.update(commitment);

    hasher.finalize().into()
}

/// Checks the signature on the provided transparency tree head using the given key
fn verify_tree_head_signature(
    config: &PublicConfig,
    head: &SingleSignatureTreeHead,
    root: &[u8; 32],
    verifying_key: &VerifyingKey,
) -> Result<()> {
    let raw = marshal_tree_head_tbs(head.tree_size, head.timestamp, root, config)?;
    let signature = Signature::from_slice(&head.signature)
        .map_err(|_| Error::BadData("signature has wrong size"))?;
    verifying_key
        .verify(&raw, &signature)
        .map_err(|_| Error::VerificationFailed("failed to verify tree head signature".to_string()))
}

/// Checks that a FullTreeHead structure is valid. It stores the tree head for
/// later requests if it succeeds.
fn verify_full_tree_head(
    config: &PublicConfig,
    fth: &FullTreeHead,
    root: [u8; 32],
    last_tree_head: Option<&LastTreeHead>,
    last_distinguished_tree_head: Option<&LastTreeHead>,
    now: SystemTime,
) -> Result<LastTreeHead> {
    let tree_head = get_proto_field(&fth.tree_head, "tree_head")?;

    {
        let current_tree_head = (tree_head, &root);
        if let Some(verify) = check_consistency_metadata(
            current_tree_head,
            &get_hash_proof(&fth.last)?,
            last_tree_head,
        )? {
            verify()?
        }
        if let Some(verify) = check_consistency_metadata(
            current_tree_head,
            &get_hash_proof(&fth.distinguished)?,
            last_distinguished_tree_head,
        )? {
            verify()?
        }
    }

    // 2. Verify the signature in TreeHead.signature.
    {
        let single_sig_tree_head = tree_head
            .to_single_signature_tree_head(config)
            .ok_or(Error::RequiredFieldMissing("server signature"))?;
        verify_tree_head_signature(config, &single_sig_tree_head, &root, &config.signature_key)?;
    }

    // 3. Verify that the timestamp in TreeHead is sufficiently recent.
    verify_timestamp(tree_head.timestamp, ALLOWED_TIMESTAMP_RANGE, now)?;

    // 4. If third-party auditing is used, verify auditor_tree_head with the
    //    steps described in Section 11.2.
    if let DeploymentMode::ThirdPartyAuditing(auditor_key) = config.mode {
        let auditor_tree_head = fth
            .select_auditor_tree_head(&auditor_key)
            .ok_or(Error::RequiredFieldMissing("auditor tree head"))?;
        let auditor_head = get_proto_field(&auditor_tree_head.tree_head, "tree_head")?;

        // 2. Verify that TreeHead.timestamp is sufficiently recent.
        verify_timestamp(auditor_head.timestamp, ALLOWED_AUDITOR_TIMESTAMP_RANGE, now)?;

        // 3. Verify that TreeHead.tree_size is sufficiently close to the most
        //    recent tree head from the service operator.
        if auditor_head.tree_size > tree_head.tree_size {
            return Err(Error::BadData(
                "auditor tree head may not be further along than service tree head",
            ));
        }
        if tree_head.tree_size - auditor_head.tree_size > ENTRIES_MAX_BEHIND {
            return Err(Error::BadData(
                "auditor tree head is too far behind service tree head",
            ));
        }
        // 4. Verify the consistency proof between this tree head and the most
        //    recent tree head from the service operator.
        // 1. Verify the signature in TreeHead.signature.
        if tree_head.tree_size > auditor_head.tree_size {
            let auditor_root: &[u8; 32] =
                get_proto_field(&auditor_tree_head.root_value, "root_value")?
                    .as_slice()
                    .try_into()
                    .map_err(|_| Error::BadData("auditor tree head is malformed"))?;
            let proof = get_hash_proof(&auditor_tree_head.consistency)?; // Bookmark: consistency proof
            verify_consistency_proof(
                auditor_head.tree_size,
                tree_head.tree_size,
                &proof,
                auditor_root,
                &root,
            )?;
            verify_tree_head_signature(config, auditor_head, auditor_root, &auditor_key)?;
        } else {
            if !auditor_tree_head.consistency.is_empty() {
                return Err(Error::BadData(
                    "consistency proof provided when not expected",
                ));
            }
            if auditor_tree_head.root_value.is_some() {
                return Err(Error::BadData(
                    "explicit root value provided when not expected",
                ));
            }
            verify_tree_head_signature(config, auditor_head, &root, &auditor_key)?;
        }
    }

    Ok((tree_head.clone(), root))
}

/// Checks if the consistency proof against the baseline tree head needs to be
/// verified and if it does, returns a function that performs the verification.
///
/// The `baseline` parameter is either the last_tree_head or the
/// last_distinguished_tree_head, `proof` is the corresponding consistency proof
/// from the search response, and `current` corresponds to the tree head from
/// the search response.
///
/// * If the baseline is present, the proof should be present and valid.
/// * Unless the current tree head size is equal to the baseline tree head size,
///   in which case the proof should be empty.
/// * If the baseline is absent, the proof must be empty.
fn check_consistency_metadata<'a>(
    current: (&'a TreeHead, &'a TreeRoot),
    proof: &'a [[u8; 32]],
    baseline: Option<&'a LastTreeHead>,
) -> Result<Option<impl FnOnce() -> Result<()> + 'a>> {
    let (current_head, current_root) = current;

    match baseline {
        None => {
            if !proof.is_empty() {
                return Err(Error::BadData(
                    "consistency proof provided when not expected",
                ));
            };
            Ok(None)
        }
        Some((last, last_root)) if last.tree_size == current_head.tree_size => {
            if current_root != last_root {
                return Err(Error::BadData("root is different but tree size is same"));
            }
            if current_head.timestamp != last.timestamp {
                return Err(Error::BadData(
                    "tree size is the same but timestamps differ",
                ));
            }
            if !proof.is_empty() {
                return Err(Error::BadData(
                    "consistency proof provided when not expected",
                ));
            }
            Ok(None)
        }
        Some((last_head, last_root)) => {
            if current_head.tree_size < last_head.tree_size {
                return Err(Error::BadData(
                    "current tree size is less than previous tree size",
                ));
            }
            if current_head.timestamp < last_head.timestamp {
                return Err(Error::BadData(
                    "current timestamp is less than previous timestamp",
                ));
            }
            Ok(Some(move || {
                Ok(verify_consistency_proof(
                    last_head.tree_size,
                    current_head.tree_size,
                    proof,
                    last_root,
                    current_root,
                )?)
            }))
        }
    }
}

/// The range of allowed timestamp values relative to "now".
/// The timestamps will have to be in [now - max_behind .. now + max_ahead]
struct TimestampRange {
    max_behind: Duration,
    max_ahead: Duration,
}

fn verify_timestamp(timestamp: i64, allowed_range: &TimestampRange, now: SystemTime) -> Result<()> {
    let TimestampRange {
        max_behind,
        max_ahead,
    } = allowed_range;
    let now = now
        .duration_since(UNIX_EPOCH)
        .expect("valid system time")
        .as_millis() as i128;
    let delta = now - timestamp as i128;
    if delta > max_behind.as_millis() as i128 {
        return Err(Error::BadData("timestamp is too far behind current time"));
    }
    if (-delta) > max_ahead.as_millis() as i128 {
        return Err(Error::BadData("timestamp is too far ahead of current time"));
    }
    Ok(())
}

/// Checks that the provided FullTreeHead has a valid consistency proof relative
/// to the provided distinguished head.
pub fn verify_distinguished(
    fth: &FullTreeHead,
    last_tree_head: Option<&LastTreeHead>,
    last_distinguished_tree_head: &LastTreeHead,
) -> Result<()> {
    let tree_size = get_proto_field(&fth.tree_head, "tree_head")?.tree_size;

    if last_tree_head.is_none() {
        return Ok(());
    }
    let root = match last_tree_head {
        Some((tree_head, root)) if tree_head.tree_size == tree_size => root,
        _ => return Err(Error::BadData("expected tree head not found in storage")),
    };

    let (
        TreeHead {
            tree_size: distinguished_size,
            timestamp: _,
            signatures: _,
        },
        distinguished_root,
    ) = last_distinguished_tree_head;

    // Handle special case when tree_size == distinguished_size.
    if tree_size == *distinguished_size {
        let result = if root == distinguished_root {
            Ok(())
        } else {
            Err(Error::BadData("root hash does not match expected value"))
        };
        return result;
    }

    Ok(verify_consistency_proof(
        *distinguished_size,
        tree_size,
        &get_hash_proof(&fth.distinguished)?,
        distinguished_root,
        root,
    )?)
}

fn evaluate_vrf_proof(
    proof: &[u8],
    vrf_key: &vrf::PublicKey,
    search_key: &[u8],
) -> Result<[u8; 32]> {
    let proof = proof.try_into().map_err(|_| MalformedProof)?;
    Ok(vrf_key.proof_to_hash(search_key, proof)?)
}

/// The shared implementation of verify_search and verify_update.
fn verify_search_internal(
    config: &PublicConfig,
    req: SlimSearchRequest,
    res: FullSearchResponse,
    context: SearchContext,
    monitor: bool,
    now: SystemTime,
) -> Result<SearchStateUpdate> {
    let SlimSearchRequest {
        search_key,
        version,
    } = req;
    let FullSearchResponse {
        condensed:
            CondensedTreeSearchResponse {
                vrf_proof,
                search,
                opening,
                value,
            },
        tree_head: full_tree_head,
    } = res;

    let index = evaluate_vrf_proof(&vrf_proof, &config.vrf_key, &search_key)?;

    // Evaluate the search proof.
    let tree_size = {
        let tree_head = get_proto_field(&full_tree_head.tree_head, "tree_head")?;
        tree_head.tree_size
    };
    let search_proof = get_proto_field(&search, "search")?;

    let guide = ProofGuide::new(version, search_proof.pos, tree_size);

    let mut i = 0;
    let mut leaves = HashMap::new();
    let mut steps = HashMap::new();
    let result = guide.consume(|guide, next_id| {
        if i >= search_proof.steps.len() {
            return Err(Error::VerificationFailed(
                "unexpected number of steps in search proof".to_string(),
            ));
        }
        let step = &search_proof.steps[i];
        let prefix_proof = get_proto_field(&step.prefix, "prefix")?;
        guide.insert(next_id, prefix_proof.counter);

        // Evaluate the prefix proof and combine it with the commitment to get
        // the value stored in the log.
        let prefix_root = evaluate_prefix(&index, search_proof.pos, prefix_proof)?;
        let commitment = step
            .commitment
            .as_slice()
            .try_into()
            .map_err(|_| MalformedProof)?;
        leaves.insert(next_id, leaf_hash(&prefix_root, commitment));
        steps.insert(next_id, step.clone());

        i += 1;
        Ok::<(), Error>(())
    })?;

    let root = {
        if i != search_proof.steps.len() {
            return Err(Error::VerificationFailed(
                "unexpected number of steps in search proof".to_string(),
            ));
        }

        let inclusion_proof = get_hash_proof(&search_proof.inclusion)?;

        let (ids, values) = into_sorted_pairs(leaves);
        evaluate_batch_proof(&ids, tree_size, &values, &inclusion_proof)?
    };

    // Verify commitment opening.
    let (result_i, result_id) = result.ok_or_else(|| {
        Error::VerificationFailed("failed to find expected version of key".to_string())
    })?;
    let result_step = &search_proof.steps[result_i];

    let value = marshal_update_value(&get_proto_field(&value, "value")?.value)?;
    let opening = opening
        .as_slice()
        .try_into()
        .map_err(|_| Error::VerificationFailed("malformed opening".to_string()))?;

    if !verify_commitment(&search_key, &result_step.commitment, &value, opening) {
        return Err(Error::VerificationFailed(
            "failed to verify commitment opening".to_string(),
        ));
    }

    let SearchContext {
        last_tree_head,
        last_distinguished_tree_head,
        data,
    } = context;

    // Verify the tree head with the candidate root.
    let updated_tree_head = verify_full_tree_head(
        config,
        full_tree_head,
        root,
        last_tree_head,
        last_distinguished_tree_head,
        now,
    )?;

    // Update stored monitoring data.
    let size = if search_key == b"distinguished" {
        // Make sure we don't update monitoring data based on parts of the tree
        // that we don't intend to retain.
        result_id + 1
    } else {
        tree_size
    };
    let ver = get_proto_field(&result_step.prefix, "prefix")?.counter;

    let mut mdw = MonitoringDataWrapper::new(data);
    if monitor || config.mode == DeploymentMode::ContactMonitoring {
        mdw.start_monitoring(&index, search_proof.pos, result_id, ver, monitor);
    }
    mdw.check_search_consistency(size, &index, search_proof.pos, result_id, ver, monitor)?;
    mdw.update(size, &steps)?;

    Ok(SearchStateUpdate {
        tree_head: updated_tree_head.0,
        tree_root: updated_tree_head.1,
        monitoring_data: mdw.into_data_update(),
    })
}

/// Checks that the output of a Search operation is valid and updates the
/// client's stored data. `res.value.value` may only be consumed by the
/// application if this function returns successfully.
pub fn verify_search(
    config: &PublicConfig,
    req: SlimSearchRequest,
    res: FullSearchResponse,
    context: SearchContext,
    force_monitor: bool,
    now: SystemTime,
) -> Result<SearchStateUpdate> {
    verify_search_internal(config, req, res, context, force_monitor, now)
}

/// Checks that the output of a Monitor operation is valid and updates the
/// client's stored data.
pub fn verify_monitor<'a>(
    config: &'a PublicConfig,
    req: &'a MonitorRequest,
    res: &'a MonitorResponse,
    context: MonitorContext,
    now: SystemTime,
) -> Result<MonitorStateUpdate> {
    // Verify proof responses are the expected lengths.
    if req.keys.len() != res.proofs.len() {
        return Err(Error::BadData(
            "monitoring response is malformed: wrong number of key proofs",
        ));
    }

    let full_tree_head = get_proto_field(&res.tree_head, "tree_head")?;
    let tree_head = get_proto_field(&full_tree_head.tree_head, "tree_head")?;
    let tree_size = tree_head.tree_size;

    let MonitorContext {
        last_tree_head,
        last_distinguished_tree_head,
        data,
    } = context;

    // Process all the individual MonitorProof structures.
    let mut mpa = MonitorProofAcc::new(tree_size);
    for (key, proof) in req.keys.iter().zip(res.proofs.iter()) {
        mpa.process(&data, key, proof)?;
    }

    // Evaluate the inclusion proof to get a candidate root value.
    let inclusion_proof = get_hash_proof(&res.inclusion)?;
    let root = if mpa.leaves.is_empty() {
        match inclusion_proof[..] {
            [root] => root,
            _ => {
                return Err(Error::VerificationFailed(
                    "monitoring response is malformed: inclusion proof should be root".to_string(),
                ))
            }
        }
    } else {
        let (ids, values) = into_sorted_pairs(mpa.leaves);

        evaluate_batch_proof(&ids, tree_size, &values, &inclusion_proof)?
    };

    // Verify the tree head with the candidate root.
    let updated_tree_head = verify_full_tree_head(
        config,
        full_tree_head,
        root,
        last_tree_head,
        Some(last_distinguished_tree_head),
        now,
    )?;

    let MonitorRequest { keys, consistency } = req;

    let mut data_updates = HashMap::with_capacity(keys.len());
    // Update monitoring data.
    for (key, entry) in keys.iter().zip(mpa.entries.iter()) {
        let size = if key.search_key == b"distinguished" {
            // Generally an effort has been made to avoid referencing the
            // "distinguished" key in the core keytrans library, but it
            // simplifies things here:
            //
            // When working with the "distinguished" key, the last observed tree
            // head is always trimmed back to create an anonymity set. As such,
            // when monitoring the "distinguished" key, we need to make sure we
            // don't update monitoring data based on parts of the tree that we
            // don't intend to retain.
            consistency.and_then(|consistency| consistency.last).ok_or(
                Error::RequiredFieldMissing("consistency field when monitoring distinguished key"),
            )?
        } else {
            tree_size
        };
        let data = data.get(&key.search_key).cloned();
        let mut mdw = MonitoringDataWrapper::new(data);
        mdw.update(size, entry)?;

        if let Some(data_update) = mdw.into_data_update() {
            data_updates.insert(key.search_key.clone(), data_update);
        }
    }

    Ok(MonitorStateUpdate {
        tree_head: updated_tree_head.0,
        tree_root: updated_tree_head.1,
        monitoring_data: data_updates,
    })
}

struct MonitorProofAcc {
    tree_size: u64,
    /// Map from position in the log to leaf hash.
    leaves: HashMap<u64, [u8; 32]>,
    /// For each MonitorProof struct processed, contains the map that needs to be
    /// passed to MonitoringDataWrapper::update to update monitoring data for the
    /// search key.
    entries: Vec<HashMap<u64, ProofStep>>,
}

impl MonitorProofAcc {
    fn new(tree_size: u64) -> Self {
        Self {
            tree_size,
            leaves: HashMap::new(),
            entries: vec![],
        }
    }

    fn process(
        &mut self,
        monitoring_data: &HashMap<Vec<u8>, MonitoringData>,
        key: &MonitorKey,
        proof: &MonitorProof,
    ) -> Result<()> {
        // Get the existing monitoring data from storage and check that it
        // matches the request.
        let data = monitoring_data.get(&key.search_key).ok_or(Error::BadData(
            "unable to process monitoring response for unknown search key",
        ))?;

        // Compute which entry in the log each proof is supposed to correspond to.
        let entries = full_monitoring_path(key.entry_position, data.pos, self.tree_size);
        if entries.len() != proof.steps.len() {
            return Err(Error::VerificationFailed(
                "monitoring response is malformed: wrong number of proof steps".to_string(),
            ));
        }

        // Evaluate each proof step to get the candidate leaf values.
        let mut steps = HashMap::new();
        for (entry, step) in entries.iter().zip(proof.steps.iter()) {
            let prefix_proof = get_proto_field(&step.prefix, "prefix")?;
            let prefix_root = evaluate_prefix(&data.index, data.pos, prefix_proof)?;
            let commitment = step
                .commitment
                .as_slice()
                .try_into()
                .map_err(|_| MalformedProof)?;
            let leaf = leaf_hash(&prefix_root, commitment);

            if let Some(other) = self.leaves.get(entry) {
                if leaf != *other {
                    return Err(Error::VerificationFailed(
                        "monitoring response is malformed: multiple values for same leaf"
                            .to_string(),
                    ));
                }
            } else {
                self.leaves.insert(*entry, leaf);
            }

            steps.insert(*entry, step.clone());
        }
        self.entries.push(steps);

        Ok(())
    }
}

struct MonitoringDataWrapper {
    inner: Option<MonitoringData>,
}

impl MonitoringDataWrapper {
    fn new(monitoring_data: Option<MonitoringData>) -> Self {
        Self {
            inner: monitoring_data,
        }
    }

    /// Adds a key to the database of keys to monitor, if it's not already
    /// present.
    fn start_monitoring(
        &mut self,
        index: &[u8; 32],
        zero_pos: u64,
        ver_pos: u64,
        version: u32,
        owned: bool,
    ) {
        if self.inner.is_none() {
            self.inner = Some(MonitoringData {
                index: *index,
                pos: zero_pos,
                ptrs: HashMap::from([(ver_pos, version)]),
                owned,
            });
        }
    }

    fn check_search_consistency(
        &mut self,
        tree_size: u64,
        index: &[u8; 32],
        zero_pos: u64,
        ver_pos: u64,
        version: u32,
        owned: bool,
    ) -> Result<()> {
        let Some(data) = self.inner.as_mut() else {
            return Ok(());
        };

        if *index != data.index {
            return Err(Error::VerificationFailed(
                "given search key index does not match database".to_string(),
            ));
        }
        if zero_pos != data.pos {
            return Err(Error::VerificationFailed(
                "given search start position does not match database".to_string(),
            ));
        }

        match data.ptrs.get(&ver_pos) {
            Some(ver) => {
                if *ver != version {
                    return Err(Error::VerificationFailed(
                        "different versions of key recorded at same position".to_string(),
                    ));
                }
            }
            None => {
                match monitoring_path(ver_pos, zero_pos, tree_size).find_map(|x| data.ptrs.get(&x))
                {
                    Some(ver) => {
                        if *ver < version {
                            return Err(Error::VerificationFailed(
                                "prefix tree has unexpectedly low version counter".to_string(),
                            ));
                        }
                    }
                    None => {
                        data.ptrs.insert(ver_pos, version);
                    }
                };
            }
        }

        if !data.owned && owned {
            data.owned = true;
        }

        Ok(())
    }

    /// Updates the internal monitoring data for a key as much as possible given
    /// a set of ProofStep structures. It should only be called after
    /// verify_full_tree_head has succeeded to ensure that we don't store updated
    /// monitoring data tied to a tree head that isn't valid.
    fn update(&mut self, tree_size: u64, entries: &HashMap<u64, ProofStep>) -> Result<()> {
        let Some(data) = self.inner.as_mut() else {
            return Ok(());
        };

        let mut changed = false;
        let mut ptrs = HashMap::new();

        for (entry, ver) in data.ptrs.iter() {
            let mut entry = *entry;
            let mut ver = *ver;

            for x in monitoring_path(entry, data.pos, tree_size) {
                match entries.get(&x) {
                    None => break,
                    Some(step) => {
                        let ctr = get_proto_field(&step.prefix, "prefix")?.counter;
                        if ctr < ver {
                            return Err(Error::VerificationFailed(
                                "prefix tree has unexpectedly low version counter".to_string(),
                            ));
                        }
                        changed = true;
                        entry = x;
                        ver = ctr;
                    }
                }
            }

            match ptrs.get(&entry) {
                Some(other) => {
                    if ver != *other {
                        return Err(Error::VerificationFailed(
                            "inconsistent versions found".to_string(),
                        ));
                    }
                }
                None => {
                    ptrs.insert(entry, ver);
                }
            };
        }

        if changed {
            data.ptrs = ptrs;
        }

        Ok(())
    }

    fn into_data_update(self) -> Option<MonitoringData> {
        self.inner
    }
}

fn into_sorted_pairs<K: Ord + Copy, V>(map: HashMap<K, V>) -> (Vec<K>, Vec<V>) {
    let mut pairs = map.into_iter().collect::<Vec<_>>();
    pairs.sort_by_key(|pair| pair.0);
    pairs.into_iter().unzip()
}

#[cfg(test)]
mod test {
    use assert_matches::assert_matches;
    use const_str::hex;
    use prost::Message as _;
    use test_case::test_case;

    use super::*;
    use crate::ChatSearchResponse;

    const MAX_AHEAD: Duration = Duration::from_secs(42);
    const MAX_BEHIND: Duration = Duration::from_secs(42);
    const TIMESTAMP_RANGE: &TimestampRange = &TimestampRange {
        max_behind: MAX_BEHIND,
        max_ahead: MAX_AHEAD,
    };

    const ONE_SECOND: Duration = Duration::from_secs(1);

    fn make_timestamp(time: SystemTime) -> i64 {
        let duration = time.duration_since(UNIX_EPOCH).unwrap();
        duration.as_millis().try_into().unwrap()
    }

    #[test_case(SystemTime::now() + MAX_AHEAD + ONE_SECOND; "far ahead")]
    #[test_case(SystemTime::now() - MAX_BEHIND - ONE_SECOND; "far behind")]
    fn verify_timestamps_error(time: SystemTime) {
        let ts = make_timestamp(time);
        assert_matches!(
            verify_timestamp(ts, TIMESTAMP_RANGE, SystemTime::now()),
            Err(Error::BadData(_))
        );
    }

    #[test_case(SystemTime::now(); "now")]
    #[test_case(SystemTime::now() + MAX_AHEAD - ONE_SECOND; "just ahead enough")]
    #[test_case(SystemTime::now() - MAX_BEHIND + ONE_SECOND; "just behind enough")]
    fn verify_timestamps_success(time: SystemTime) {
        let ts = make_timestamp(time);
        assert_matches!(
            verify_timestamp(ts, TIMESTAMP_RANGE, SystemTime::now()),
            Ok(())
        );
    }

    #[test]
    fn can_verify_search_response() {
        let sig_key = VerifyingKey::from_bytes(&hex!(
            "ac0de1fd7f33552bbeb6ebc12b9d4ea10bf5f025c45073d3fb5f5648955a749e"
        ))
        .unwrap();
        let vrf_key = vrf::PublicKey::try_from(hex!(
            "ec3a268237cf5c47115cf222405d5f90cc633ebe05caf82c0dd5acf9d341dadb"
        ))
        .unwrap();
        let auditor_key = VerifyingKey::from_bytes(&hex!(
            "1123b13ee32479ae6af5739e5d687b51559abf7684120511f68cde7a21a0e755"
        ))
        .unwrap();
        let aci = uuid::uuid!("90c979fd-eab4-4a08-b6da-69dedeab9b29");
        let request = SlimSearchRequest::new([b"a", aci.as_bytes().as_slice()].concat());

        let (response_tree_head, condensed_response) = {
            let bytes = include_bytes!("../res/chat_search_response.dat");
            let response =
                ChatSearchResponse::decode(bytes.as_slice()).expect("can decode chat response");

            let mut head = response.tree_head.expect("has tree head");
            // we don't expect these fields to be present in the verification that follows
            head.distinguished = vec![];
            head.last = vec![];

            (head, response.aci.expect("has ACI condensed response"))
        };
        let response = FullSearchResponse {
            condensed: condensed_response,
            tree_head: &response_tree_head,
        };

        let valid_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1746042060);
        let config = PublicConfig {
            mode: DeploymentMode::ThirdPartyAuditing(auditor_key),
            signature_key: sig_key,
            vrf_key,
        };

        let last_root = hex!("87d59202bfd679c7d5753c6e5ad241852abbc2c45650de114b02620ed6098a07");
        let expected_data_update = MonitoringData {
            index: hex!("3901c94081c4e6321e92b3e434dcaf788f5326913e7bdcab47b4fd2ae7a6848a"),
            pos: 35,
            ptrs: HashMap::from([(16777215, 2)]),
            owned: true,
        };

        assert_matches!(
            verify_search_internal(&config, request.clone(), response.clone(), SearchContext::default(), true, valid_at),
            Ok(update) => {
                assert_eq!(&update.tree_head, response_tree_head.tree_head.as_ref().expect("has tree head"));
                assert_eq!(update.tree_root, last_root);
                assert_eq!(update.monitoring_data, Some(expected_data_update.clone()));
            }
        );
        // Verification result should always include the monitoring data field, even if it has not changed.
        let last_tree_head = response_tree_head.tree_head.as_ref().unwrap();
        let last_tree = (last_tree_head.clone(), last_root);
        let context = SearchContext {
            last_tree_head: Some(&last_tree),
            data: Some(expected_data_update.clone()),
            ..SearchContext::default()
        };

        assert_matches!(
            verify_search_internal(&config, request.clone(), response.clone(), context, true, valid_at),
            Ok(update) => {
                assert_eq!(&update.tree_head, last_tree_head);
                assert_eq!(update.tree_root, last_root);
                assert_eq!(update.monitoring_data, Some(expected_data_update));
            }
        );
        assert_matches!(
            verify_search_internal(&config, request, response, SearchContext::default(), false, valid_at),
            Ok(update) => {
                assert_eq!(&update.tree_head, last_tree_head);
                assert_eq!(update.tree_root, last_root);
                // When monitor == false there should be no data update
                assert!(update.monitoring_data.is_none());
            }
        );
    }

    enum Baseline {
        Absent,
        WithSize(u64),
        WithTimestamp(i64),
        WithRoot([u8; 32]),
    }

    enum VerifierOutcome {
        NoVerificationNeeded,
        Error,
        Verifier,
    }

    #[test_case(&[Baseline::Absent], false, VerifierOutcome::NoVerificationNeeded; "no baseline no proof no problem")]
    #[test_case(&[Baseline::Absent], true, VerifierOutcome::Error; "proof without baseline")]
    #[test_case(&[], false, VerifierOutcome::NoVerificationNeeded; "baseline is current no proof")]
    #[test_case(&[], true, VerifierOutcome::Error; "baseline is current proof not expected")]
    #[test_case(&[Baseline::WithSize(43)], false, VerifierOutcome::Error; "baseline is larger no proof")]
    #[test_case(&[Baseline::WithSize(43)], true, VerifierOutcome::Error; "baseline is larger with proof")]
    #[test_case(&[Baseline::WithSize(41)], false, VerifierOutcome::Verifier; "baseline is smaller no proof")]
    #[test_case(&[Baseline::WithSize(41)], true, VerifierOutcome::Verifier; "baseline is smaller with proof")]
    #[test_case(&[Baseline::WithTimestamp(42)], false, VerifierOutcome::Error; "baseline is newer no proof")]
    #[test_case(&[Baseline::WithTimestamp(42)], true, VerifierOutcome::Error; "baseline is newer with proof")]
    #[test_case(&[Baseline::WithTimestamp(-42)], false, VerifierOutcome::Error; "baseline is older no proof")]
    #[test_case(&[Baseline::WithTimestamp(-42)], true, VerifierOutcome::Error; "baseline is older with proof")]
    #[test_case(&[Baseline::WithSize(41), Baseline::WithTimestamp(42)], false, VerifierOutcome::Error; "baseline is smaller but newer no proof")]
    #[test_case(&[Baseline::WithSize(41), Baseline::WithTimestamp(42)], true, VerifierOutcome::Error; "baseline is smaller but newer with proof")]
    #[test_case(&[Baseline::WithRoot([1u8; 32])], false, VerifierOutcome::Error; "baseline different root no proof")]
    #[test_case(&[Baseline::WithRoot([1u8; 32])], true, VerifierOutcome::Error; "baseline different root with proof")]
    fn get_consistency_verifier_permutations(
        baseline_mods: &[Baseline],
        has_proof: bool,
        outcome: VerifierOutcome,
    ) {
        let current_head = TreeHead {
            tree_size: 42,
            ..TreeHead::default()
        };
        let current_root = [0u8; 32];

        let baseline = {
            let head = current_head.clone();
            let root = [0u8; 32];
            let mut baseline = Some((head, root));

            for baseline_mod in baseline_mods {
                let Some(result) = baseline.as_mut() else {
                    break;
                };
                match baseline_mod {
                    Baseline::Absent => baseline = None,
                    Baseline::WithSize(n) => {
                        result.0.tree_size = *n;
                    }
                    Baseline::WithTimestamp(ts) => {
                        result.0.timestamp = *ts;
                    }
                    Baseline::WithRoot(r) => {
                        result.1 = *r;
                    }
                }
            }
            baseline
        };

        let proof = [[0u8; 32]];

        let result = check_consistency_metadata(
            (&current_head, &current_root),
            if has_proof { &proof } else { &[] },
            baseline.as_ref(),
        );

        match outcome {
            VerifierOutcome::NoVerificationNeeded => assert!(matches!(result, Ok(None))),
            VerifierOutcome::Error => assert!(result.is_err()),
            VerifierOutcome::Verifier => assert!(matches!(result, Ok(Some(_)))),
        }
    }
}
