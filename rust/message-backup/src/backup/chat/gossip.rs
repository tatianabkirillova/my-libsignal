use crate::proto::backup::Gossip as ProtoGossip;
use crate::backup::TryIntoWith;

#[derive(Debug, serde::Serialize)]
#[cfg_attr(test, derive(PartialEq, Clone))]
pub struct Gossip {
    pub tree_size: u64,
    pub timestamp: u64,
    pub signature: Vec<u8>,
}

impl<C> TryIntoWith<Gossip, C> for ProtoGossip {
    type Error = ();

    fn try_into_with(self, _context: &C) -> Result<Gossip, Self::Error> {
        Ok(Gossip {
            tree_size: self.tree_size,
            timestamp: self.timestamp,
            signature: self.signature,
        })
    }
}