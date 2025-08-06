use crate::proto::backup::Gossip as ProtoGossip;

#[derive(Debug, serde::Serialize)]
#[cfg_attr(test, derive(PartialEq, Clone))]
pub struct Gossip {
    pub tree_size: u64,
    pub timestamp: u64,
    pub signature: Vec<u8>,
}

impl TryFrom<ProtoGossip> for Gossip {
    type Error = ();

    fn try_from(proto: ProtoGossip) -> Result<Self, Self::Error> {
        Ok(Gossip {
            tree_size: proto.tree_size,
            timestamp: proto.timestamp,
            signature: proto.signature,
        })
    }
}