use crate::proto::backup::Gossip as ProtoGossip;
use crate::backup::TryIntoWith;
pub(crate) use libsignal_protocol::Timestamp;

#[derive(Debug, serde::Serialize)]
#[cfg_attr(test, derive(PartialEq, Clone))]
pub struct Gossip {
    pub tree_size: u64,
    pub timestamp: Timestamp,
    pub signature: Vec<u8>, 
    pub root_hash: Vec<u8>,
    pub consistency: Vec<Vec<u8>>
}

impl Gossip {
    pub fn new(
        tree_size: u64,
        timestamp: Timestamp,
        signature: Vec<u8>,
        root_hash: Vec<u8>,
        consistency: Vec<Vec<u8>>,
    ) -> Self {
        Self {
            tree_size,
            timestamp,
            signature,
            root_hash,
            consistency,
        }
    }

    #[cfg(test)]
    pub fn minimal_test_data() -> Self {
        Self {
            tree_size: 0,
            timestamp: Timestamp::from_epoch_millis(0), 
            signature: vec![], // empty signature
            root_hash: vec![], // empty root_hash
            consistency: vec![], // empty consistency
        }
    }
}

impl<C> TryIntoWith<Gossip, C> for ProtoGossip {
    type Error = ();

    fn try_into_with(self, _context: &C) -> Result<Gossip, Self::Error> {
        Ok(Gossip {
            tree_size: self.tree_size,
            timestamp: Timestamp::from_epoch_millis(self.timestamp),
            signature: self.signature, 
            root_hash: self.root_hash,
            consistency: self.consistency
        })
    }
}