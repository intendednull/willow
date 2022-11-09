use serde::{de::DeserializeOwned, Serialize};

pub fn pack<T: Serialize>(data: &T) -> anyhow::Result<Vec<u8>> {
    Ok(bincode::serialize(data)?)
}

pub fn unpack<T: DeserializeOwned>(data: &[u8]) -> anyhow::Result<T> {
    Ok(bincode::deserialize(data)?)
}
